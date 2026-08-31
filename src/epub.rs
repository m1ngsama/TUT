use std::{
    collections::HashSet,
    fs::File,
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use percent_encoding::percent_decode_str;
use quick_xml::{
    Reader as XmlReader, XmlVersion,
    encoding::Decoder,
    escape::resolve_predefined_entity,
    events::{BytesStart, Event},
};
use rbook::{
    Epub,
    epub::{reader::LinearBehavior, toc::EpubTocEntry},
};
use zip::{CompressionMethod, ZipArchive};

use crate::{
    document::{
        self, BookHeading, BookSection, BookStructure, Document, MAX_BOOK_HEADINGS,
        MAX_BOOK_SECTIONS, MAX_BOOK_TITLE_BYTES, MAX_BOOK_TITLE_TOTAL_BYTES, MAX_FILE_BYTES,
        SOURCE_WINDOW_BYTES, SourceFile,
    },
    error::LoadError,
    source::SourceOffset,
};

const MAX_ENTRIES: usize = 4_096;
const MAX_ENTRY_NAME_BYTES: usize = 4_096;
const MAX_ENTRY_NAME_TOTAL_BYTES: usize = 4 * 1024 * 1024;
const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SPINE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_XML_DEPTH: usize = 128;
const MAX_TOC_DEPTH: usize = 64;
const MAX_TOC_FRAGMENT_BYTES: usize = 4_096;
const MAX_TOC_FRAGMENT_TOTAL_BYTES: usize = 1024 * 1024;
const EPUB_MIMETYPE: &[u8] = b"application/epub+zip";
const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
const ZIP64_EOCD_SIGNATURE: &[u8; 4] = b"PK\x06\x06";
const ZIP64_LOCATOR_SIGNATURE: &[u8; 4] = b"PK\x06\x07";
const EOCD_LEN: usize = 22;
const MAX_ZIP_COMMENT_BYTES: usize = u16::MAX as usize;

#[derive(Debug, Clone, Copy)]
struct ArchiveLimits {
    entries: usize,
    entry_name_bytes: usize,
    entry_name_total_bytes: usize,
    central_directory_bytes: u64,
    entry_bytes: u64,
    expanded_bytes: u64,
}

const ARCHIVE_LIMITS: ArchiveLimits = ArchiveLimits {
    entries: MAX_ENTRIES,
    entry_name_bytes: MAX_ENTRY_NAME_BYTES,
    entry_name_total_bytes: MAX_ENTRY_NAME_TOTAL_BYTES,
    central_directory_bytes: MAX_CENTRAL_DIRECTORY_BYTES,
    entry_bytes: MAX_ENTRY_BYTES,
    expanded_bytes: MAX_EXPANDED_BYTES,
};

pub(super) fn is_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
}

pub(super) fn load(path: PathBuf) -> Result<Document, LoadError> {
    let source = document::open_source(path.clone())?;
    let archive = snapshot_archive(&source, &path)?;
    inspect_eocd(&archive, &path, ARCHIVE_LIMITS)?;
    preflight(
        archive
            .try_clone()
            .map_err(|source| LoadError::BufferEpub {
                path: path.clone(),
                source,
            })?,
        &path,
        ARCHIVE_LIMITS,
    )?;

    let (epub, has_toc) = read_epub(archive, &path)?;
    let metadata = epub.metadata();
    let title = metadata
        .title()
        .and_then(|title| collapsed_text(title.value()));
    let creator = metadata
        .creators()
        .next()
        .and_then(|creator| collapsed_text(creator.value()));
    let mut toc = if has_toc {
        TocTargets::collect(&epub)
    } else {
        TocTargets::default()
    };
    let mut reader = epub
        .reader_builder()
        .linear_behavior(LinearBehavior::LinearOnly)
        .create();
    if reader.is_empty() {
        return Err(LoadError::NoLinearEpubContent(path));
    }

    let file = tempfile::tempfile().map_err(|source| LoadError::BufferEpub {
        path: path.clone(),
        source,
    })?;
    let mut output = TextWriter::new(file, path.clone(), MAX_FILE_BYTES);
    let mut headings = HeadingCollector::default();
    let mut spine_bytes = 0_u64;

    for chapter in &mut reader {
        let chapter = chapter.map_err(|source| invalid_epub(&path, source))?;
        let length =
            u64::try_from(chapter.content().len()).expect("strings fit EPUB source coordinates");
        if length > MAX_ENTRY_BYTES {
            return Err(LoadError::EpubEntryTooLarge {
                path,
                limit: MAX_ENTRY_BYTES,
            });
        }
        add_spine_bytes(&mut spine_bytes, length, MAX_SPINE_BYTES, &path)?;

        output.chapter_break();
        let manifest_index = chapter.manifest_entry().index();
        let chapter_targets = toc.for_manifest(manifest_index);
        match chapter.manifest_entry().media_type() {
            "application/xhtml+xml" => {
                project_xhtml(
                    chapter.content(),
                    &mut output,
                    &mut headings,
                    chapter_targets,
                    &path,
                )?;
            }
            "image/svg+xml" => {
                let mut resolver = ChapterTargetResolver::new(chapter_targets);
                if resolver.has_pending() {
                    output.capture_next_content();
                }
                output.image(None)?;
                resolver.resolve_capture(&mut output);
                resolver.finish(&mut output);
            }
            _ => {
                return Err(invalid_epub_message(
                    &path,
                    "spine contains unsupported content",
                ));
            }
        }
    }

    let (file, length) = output.finish()?;
    let navigation = toc.finish();
    let (sections, headings) = headings.finish(navigation);
    let book =
        (title.is_some() || creator.is_some() || !sections.is_empty() || !headings.is_empty())
            .then(|| BookStructure::new(title, creator, sections, headings));
    source.validate()?;
    document::load_snapshot(source, file, length, book)
}

fn add_spine_bytes(total: &mut u64, length: u64, limit: u64, path: &Path) -> Result<(), LoadError> {
    let next = total
        .checked_add(length)
        .filter(|length| *length <= limit)
        .ok_or_else(|| LoadError::EpubExpandedTooLarge {
            path: path.to_path_buf(),
            limit,
        })?;
    *total = next;
    Ok(())
}

fn read_epub(archive: File, path: &Path) -> Result<(Epub, bool), LoadError> {
    let with_toc = archive
        .try_clone()
        .map_err(|source| LoadError::BufferEpub {
            path: path.to_path_buf(),
            source,
        })?;
    match Epub::options().strict(false).read(with_toc) {
        Ok(epub) => Ok((epub, true)),
        Err(_) => Epub::options()
            .strict(false)
            .skip_toc(true)
            .read(archive)
            .map(|epub| (epub, false))
            .map_err(|source| invalid_epub(path, source)),
    }
}

fn snapshot_archive(source: &SourceFile, path: &Path) -> Result<File, LoadError> {
    source.validate()?;
    let mut input = source.try_clone()?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|source| LoadError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    let mut output = tempfile::tempfile().map_err(|source| LoadError::BufferEpub {
        path: path.to_path_buf(),
        source,
    })?;
    let mut buffer = [0_u8; SOURCE_WINDOW_BYTES];
    let mut remaining = source.len();

    while remaining != 0 {
        let request = usize::try_from(remaining.min(SOURCE_WINDOW_BYTES as u64))
            .expect("bounded EPUB reads fit usize");
        let count = loop {
            match input.read(&mut buffer[..request]) {
                Ok(count) => break count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(source) => {
                    return Err(LoadError::Read {
                        path: path.to_path_buf(),
                        source,
                    });
                }
            }
        };
        if count == 0 {
            return Err(changed(path));
        }
        output
            .write_all(&buffer[..count])
            .map_err(|source| LoadError::BufferEpub {
                path: path.to_path_buf(),
                source,
            })?;
        remaining -= u64::try_from(count).expect("source windows fit u64");
    }
    source.validate()?;
    Ok(output)
}

fn changed(path: &Path) -> LoadError {
    LoadError::Read {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, "file changed while reading"),
    }
}

fn inspect_eocd(file: &File, path: &Path, limits: ArchiveLimits) -> Result<(), LoadError> {
    let length = file
        .metadata()
        .map_err(|source| LoadError::BufferEpub {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if length < EOCD_LEN as u64 {
        return Err(invalid_epub_message(path, "ZIP end record is missing"));
    }

    let tail_len = usize::try_from(
        length.min(
            u64::try_from(EOCD_LEN + MAX_ZIP_COMMENT_BYTES)
                .expect("ZIP end-record search length fits u64"),
        ),
    )
    .expect("bounded ZIP end-record searches fit usize");
    let mut tail = Vec::new();
    tail.try_reserve_exact(tail_len)
        .map_err(|_| LoadError::Allocation("EPUB ZIP end-record buffer"))?;
    tail.resize(tail_len, 0);
    read_exact_at(file, &mut tail, length - tail_len as u64)
        .map_err(|source| invalid_epub(path, source))?;

    let Some(position) = (0..=tail_len - EOCD_LEN).rev().find(|position| {
        tail[*position..].starts_with(EOCD_SIGNATURE)
            && *position + EOCD_LEN + usize::from(le_u16(&tail[*position + 20..])) == tail_len
    }) else {
        return Err(invalid_epub_message(path, "ZIP end record is invalid"));
    };
    let record = &tail[position..position + EOCD_LEN];
    let eocd_offset = length - tail_len as u64 + position as u64;
    let disk = le_u16(&record[4..]);
    let central_disk = le_u16(&record[6..]);
    let disk_entries = le_u16(&record[8..]);
    let entries = le_u16(&record[10..]);
    let central_size = le_u32(&record[12..]);
    let central_offset = le_u32(&record[16..]);

    let uses_zip64 = disk_entries == u16::MAX
        || entries == u16::MAX
        || central_size == u32::MAX
        || central_offset == u32::MAX;
    let (entries, central_size, central_offset, central_end) = if uses_zip64 {
        inspect_zip64(file, path, eocd_offset)?
    } else {
        if disk != 0 || central_disk != 0 || disk_entries != entries {
            return Err(invalid_epub_message(path, "multi-disk ZIP is unsupported"));
        }
        (
            u64::from(entries),
            u64::from(central_size),
            u64::from(central_offset),
            eocd_offset,
        )
    };

    if entries > limits.entries as u64 {
        return Err(LoadError::EpubTooManyEntries {
            path: path.to_path_buf(),
            limit: limits.entries,
        });
    }
    if central_size > limits.central_directory_bytes {
        return Err(invalid_epub_message(
            path,
            "ZIP central directory exceeds its limit",
        ));
    }
    if central_offset
        .checked_add(central_size)
        .is_none_or(|end| end > central_end)
    {
        return Err(invalid_epub_message(
            path,
            "ZIP central directory is out of bounds",
        ));
    }
    Ok(())
}

fn inspect_zip64(
    file: &File,
    path: &Path,
    eocd_offset: u64,
) -> Result<(u64, u64, u64, u64), LoadError> {
    let locator_offset = eocd_offset
        .checked_sub(20)
        .ok_or_else(|| invalid_epub_message(path, "ZIP64 locator is missing"))?;
    let mut locator = [0_u8; 20];
    read_exact_at(file, &mut locator, locator_offset)
        .map_err(|source| invalid_epub(path, source))?;
    if !locator.starts_with(ZIP64_LOCATOR_SIGNATURE)
        || le_u32(&locator[4..]) != 0
        || le_u32(&locator[16..]) != 1
    {
        return Err(invalid_epub_message(path, "ZIP64 locator is invalid"));
    }

    let record_offset = le_u64(&locator[8..]);
    if record_offset
        .checked_add(56)
        .is_none_or(|end| end > locator_offset)
    {
        return Err(invalid_epub_message(
            path,
            "ZIP64 end record is out of bounds",
        ));
    }
    let mut record = [0_u8; 56];
    read_exact_at(file, &mut record, record_offset).map_err(|source| invalid_epub(path, source))?;
    if !record.starts_with(ZIP64_EOCD_SIGNATURE)
        || le_u64(&record[4..]) < 44
        || le_u32(&record[16..]) != 0
        || le_u32(&record[20..]) != 0
        || le_u64(&record[24..]) != le_u64(&record[32..])
    {
        return Err(invalid_epub_message(path, "ZIP64 end record is invalid"));
    }
    Ok((
        le_u64(&record[32..]),
        le_u64(&record[40..]),
        le_u64(&record[48..]),
        record_offset,
    ))
}

fn preflight(file: File, path: &Path, limits: ArchiveLimits) -> Result<(), LoadError> {
    let mut archive = ZipArchive::new(file).map_err(|source| invalid_epub(path, source))?;
    if archive.len() > limits.entries {
        return Err(LoadError::EpubTooManyEntries {
            path: path.to_path_buf(),
            limit: limits.entries,
        });
    }
    let mut names = HashSet::new();
    names
        .try_reserve(archive.len())
        .map_err(|_| LoadError::Allocation("EPUB entry index"))?;
    let mut name_bytes = 0_usize;
    let mut advertised_total = 0_u64;
    let mut actual_total = 0_u64;

    for index in 0..archive.len() {
        let metadata = archive
            .by_index_raw(index)
            .map_err(|source| invalid_epub(path, source))?;
        if metadata.encrypted() {
            return Err(invalid_epub_message(
                path,
                "encrypted EPUB entries are unsupported",
            ));
        }
        if !matches!(
            metadata.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(invalid_epub_message(
                path,
                "EPUB compression method is unsupported",
            ));
        }
        let raw_name = metadata.name_raw();
        if raw_name.len() > limits.entry_name_bytes {
            return Err(invalid_epub_message(
                path,
                "EPUB entry name exceeds its limit",
            ));
        }
        name_bytes = name_bytes
            .checked_add(raw_name.len())
            .filter(|length| *length <= limits.entry_name_total_bytes)
            .ok_or_else(|| invalid_epub_message(path, "EPUB entry names exceed their limit"))?;
        let name = std::str::from_utf8(raw_name).map_err(|source| invalid_epub(path, source))?;
        if !safe_entry_name(name) {
            return Err(invalid_epub_message(
                path,
                "EPUB contains an unsafe entry name",
            ));
        }
        let mut owned_name = String::new();
        owned_name
            .try_reserve_exact(name.len())
            .map_err(|_| LoadError::Allocation("EPUB entry name"))?;
        owned_name.push_str(name);
        if !names.insert(owned_name) {
            return Err(invalid_epub_message(
                path,
                "EPUB contains duplicate entry names",
            ));
        }
        let advertised = metadata.size();
        if advertised > limits.entry_bytes {
            return Err(LoadError::EpubEntryTooLarge {
                path: path.to_path_buf(),
                limit: limits.entry_bytes,
            });
        }
        advertised_total = advertised_total
            .checked_add(advertised)
            .filter(|length| *length <= limits.expanded_bytes)
            .ok_or_else(|| LoadError::EpubExpandedTooLarge {
                path: path.to_path_buf(),
                limit: limits.expanded_bytes,
            })?;
        let is_mimetype =
            index == 0 && name == "mimetype" && metadata.compression() == CompressionMethod::Stored;
        drop(metadata);

        let mut entry = archive
            .by_index(index)
            .map_err(|source| invalid_epub(path, source))?;
        let actual = if index == 0 {
            let mut value = Vec::new();
            value
                .try_reserve_exact(EPUB_MIMETYPE.len() + 1)
                .map_err(|_| LoadError::Allocation("EPUB mimetype"))?;
            (&mut entry)
                .take((EPUB_MIMETYPE.len() + 1) as u64)
                .read_to_end(&mut value)
                .map_err(|source| invalid_epub(path, source))?;
            if !is_mimetype || value != EPUB_MIMETYPE {
                return Err(invalid_epub_message(path, "EPUB mimetype entry is invalid"));
            }
            value.len() as u64
        } else {
            io::copy(
                &mut (&mut entry).take(limits.entry_bytes.saturating_add(1)),
                &mut io::sink(),
            )
            .map_err(|source| invalid_epub(path, source))?
        };
        if actual > limits.entry_bytes {
            return Err(LoadError::EpubEntryTooLarge {
                path: path.to_path_buf(),
                limit: limits.entry_bytes,
            });
        }
        if actual != advertised {
            return Err(invalid_epub_message(
                path,
                "EPUB entry length does not match its ZIP record",
            ));
        }
        actual_total = actual_total
            .checked_add(actual)
            .filter(|length| *length <= limits.expanded_bytes)
            .ok_or_else(|| LoadError::EpubExpandedTooLarge {
                path: path.to_path_buf(),
                limit: limits.expanded_bytes,
            })?;
    }
    Ok(())
}

fn safe_entry_name(name: &str) -> bool {
    if name.is_empty() || name.starts_with('/') || name.contains(['\\', '\0']) {
        return false;
    }
    let name = name.strip_suffix('/').unwrap_or(name);
    !name.is_empty()
        && name
            .split('/')
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

#[derive(Debug)]
struct TocTarget {
    manifest_index: usize,
    fragment: Option<String>,
    title: String,
    level: u8,
    ordinal: usize,
    resolved: Option<SourceOffset>,
    attempted: bool,
    active: bool,
    awaiting: bool,
}

#[derive(Debug, Default)]
struct TocTargets {
    targets: Vec<TocTarget>,
    visited: usize,
    title_bytes: usize,
    fragment_bytes: usize,
    saturated: bool,
}

impl TocTargets {
    fn collect(epub: &Epub) -> Self {
        let mut targets = Self::default();
        if let Some(root) = epub.toc().contents() {
            targets.visit(root);
        }
        targets.targets.sort_unstable_by(|left, right| {
            left.manifest_index
                .cmp(&right.manifest_index)
                .then_with(|| left.fragment.cmp(&right.fragment))
                .then_with(|| left.ordinal.cmp(&right.ordinal))
        });
        targets
    }

    fn visit(&mut self, parent: EpubTocEntry<'_>) {
        if self.saturated || parent.depth() >= MAX_TOC_DEPTH {
            return;
        }
        for entry in parent.iter() {
            if self.visited == MAX_BOOK_SECTIONS {
                self.saturated = true;
                return;
            }
            self.visited += 1;
            self.push(entry);
            self.visit(entry);
            if self.saturated {
                return;
            }
        }
    }

    fn push(&mut self, entry: EpubTocEntry<'_>) {
        let Some(href) = entry.href() else {
            return;
        };
        let Some(manifest) = entry.manifest_entry() else {
            return;
        };
        let Some(title) = collapsed_text(entry.label()) else {
            return;
        };
        let Ok(level) = u8::try_from(entry.depth()) else {
            return;
        };
        let Some(next_title_bytes) = self
            .title_bytes
            .checked_add(title.len())
            .filter(|total| *total <= MAX_BOOK_TITLE_TOTAL_BYTES)
        else {
            return;
        };
        let fragment = match href.fragment().filter(|fragment| !fragment.is_empty()) {
            Some(fragment) => {
                let Some(fragment) = decoded_fragment(fragment) else {
                    return;
                };
                let Some(next_fragment_bytes) = self
                    .fragment_bytes
                    .checked_add(fragment.len())
                    .filter(|total| *total <= MAX_TOC_FRAGMENT_TOTAL_BYTES)
                else {
                    return;
                };
                self.fragment_bytes = next_fragment_bytes;
                Some(fragment)
            }
            None => None,
        };
        if self.targets.try_reserve(1).is_err() {
            self.saturated = true;
            return;
        }
        self.targets.push(TocTarget {
            manifest_index: manifest.index(),
            fragment,
            title,
            level,
            ordinal: self.visited,
            resolved: None,
            attempted: false,
            active: false,
            awaiting: false,
        });
        self.title_bytes = next_title_bytes;
    }

    fn for_manifest(&mut self, manifest_index: usize) -> &mut [TocTarget] {
        let start = self
            .targets
            .partition_point(|target| target.manifest_index < manifest_index);
        let end = self
            .targets
            .partition_point(|target| target.manifest_index <= manifest_index);
        let targets = &mut self.targets[start..end];
        for target in &mut *targets {
            if !target.attempted {
                target.attempted = true;
                target.active = true;
                target.awaiting = target.fragment.is_none();
            }
        }
        targets
    }

    fn finish(mut self) -> Vec<BookSection> {
        self.targets.sort_unstable_by_key(|target| {
            (
                target.resolved.map(SourceOffset::get).unwrap_or(u64::MAX),
                target.ordinal,
            )
        });
        let mut sections = Vec::new();
        if sections.try_reserve(self.targets.len()).is_err() {
            return sections;
        }
        let mut previous = None;
        for target in self.targets {
            let Some(offset) = target.resolved else {
                continue;
            };
            if previous == Some(offset) {
                continue;
            }
            if let Some(section) = BookSection::new(target.title, target.level, offset) {
                sections.push(section);
                previous = Some(offset);
            }
        }
        sections
    }
}

struct ChapterTargetResolver<'a> {
    targets: &'a mut [TocTarget],
    fragment_start: usize,
    unresolved_fragments: usize,
    pending: Vec<usize>,
    active: bool,
}

impl<'a> ChapterTargetResolver<'a> {
    fn new(targets: &'a mut [TocTarget]) -> Self {
        let fragment_start = targets.partition_point(|target| target.fragment.is_none());
        let mut pending = Vec::new();
        let mut active = targets.first().is_some_and(|target| target.active);
        if active && pending.try_reserve_exact(targets.len()).is_err() {
            active = false;
        }
        if active {
            pending.extend(0..fragment_start);
        } else {
            for target in &mut *targets {
                target.active = false;
                target.awaiting = false;
            }
        }
        let unresolved_fragments = if active {
            targets.len() - fragment_start
        } else {
            0
        };
        Self {
            targets,
            fragment_start,
            unresolved_fragments,
            pending,
            active,
        }
    }

    fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn has_unresolved_fragments(&self) -> bool {
        self.unresolved_fragments != 0
    }

    fn match_fragment(&mut self, value: &str) -> bool {
        if !self.active || self.unresolved_fragments == 0 {
            return false;
        }
        let fragments = &self.targets[self.fragment_start..];
        let start = fragments.partition_point(|target| {
            target
                .fragment
                .as_deref()
                .expect("fragment targets are partitioned")
                < value
        });
        let end = fragments.partition_point(|target| {
            target
                .fragment
                .as_deref()
                .expect("fragment targets are partitioned")
                <= value
        });
        if start == end {
            return false;
        }
        for relative in start..end {
            let index = self.fragment_start + relative;
            let target = &mut self.targets[index];
            if !target.awaiting && target.resolved.is_none() {
                target.awaiting = true;
                self.pending.push(index);
                self.unresolved_fragments -= 1;
            }
        }
        true
    }

    fn resolve_capture(&mut self, output: &mut TextWriter) {
        let Some(offset) = output.take_captured_content_start() else {
            return;
        };
        for index in self.pending.drain(..) {
            let target = &mut self.targets[index];
            target.resolved = Some(offset);
            target.awaiting = false;
        }
    }

    fn finish(mut self, output: &mut TextWriter) {
        output.cancel_content_capture();
        self.pending.clear();
        for target in self.targets {
            target.active = false;
            target.awaiting = false;
        }
    }
}

fn decoded_fragment(fragment: &str) -> Option<String> {
    if fragment.len() > MAX_TOC_FRAGMENT_BYTES.saturating_mul(3) {
        return None;
    }
    let decoded = percent_decode_str(fragment).decode_utf8().ok()?;
    if decoded.is_empty() || decoded.len() > MAX_TOC_FRAGMENT_BYTES {
        return None;
    }
    let mut owned = String::new();
    owned.try_reserve_exact(decoded.len()).ok()?;
    owned.push_str(&decoded);
    Some(owned)
}

#[derive(Debug)]
struct CapturedHeading {
    title: Option<String>,
    heading: BookHeading,
}

#[derive(Debug, Default)]
struct HeadingCollector {
    captured: Vec<CapturedHeading>,
    title_bytes: usize,
    saturated: bool,
}

impl HeadingCollector {
    fn push(&mut self, title: String, level: u8, start: SourceOffset, end: SourceOffset) {
        if self.saturated || self.captured.len() == MAX_BOOK_HEADINGS {
            self.saturated = true;
            return;
        }
        let heading = BookHeading::new(level, start, end);
        let retained_title = self
            .title_bytes
            .checked_add(title.len())
            .filter(|total| *total <= MAX_BOOK_TITLE_TOTAL_BYTES);
        let Some(heading) = heading else {
            return;
        };
        if self.captured.try_reserve(1).is_err() {
            self.saturated = true;
            return;
        }
        self.captured.push(CapturedHeading {
            title: retained_title.map(|_| title),
            heading,
        });
        if let Some(total) = retained_title {
            self.title_bytes = total;
        }
    }

    fn finish(self, mut sections: Vec<BookSection>) -> (Vec<BookSection>, Vec<BookHeading>) {
        let fallback = sections.is_empty();
        let mut headings = Vec::new();
        let retain_headings = headings.try_reserve(self.captured.len()).is_ok();
        let retain_fallback = !fallback || sections.try_reserve(self.captured.len()).is_ok();

        for captured in self.captured {
            if retain_headings {
                headings.push(captured.heading);
            }
            if fallback
                && retain_fallback
                && let Some(title) = captured.title
                && let Some(section) =
                    BookSection::new(title, captured.heading.level(), captured.heading.start())
            {
                sections.push(section);
            }
        }
        (sections, headings)
    }
}

#[derive(Debug)]
struct ActiveHeading {
    depth: usize,
    level: u8,
    start: SourceOffset,
    title: CollapsedText,
}

#[derive(Debug, Default)]
struct CollapsedText {
    text: String,
    pending_space: bool,
    truncated: bool,
    discarded: bool,
}

impl CollapsedText {
    fn push(&mut self, value: &str) {
        if self.truncated || self.discarded {
            return;
        }
        let reserve = value
            .len()
            .saturating_add(1)
            .min(MAX_BOOK_TITLE_BYTES.saturating_sub(self.text.len()));
        if self.text.try_reserve(reserve).is_err() {
            self.text.clear();
            self.discarded = true;
            return;
        }

        for character in value.chars() {
            if character.is_whitespace() {
                self.space();
                continue;
            }
            let separator = usize::from(self.pending_space && !self.text.is_empty());
            let required = separator.saturating_add(character.len_utf8());
            if self.text.len().saturating_add(required) > MAX_BOOK_TITLE_BYTES {
                self.truncate();
                return;
            }
            if separator != 0 {
                self.text.push(' ');
            }
            self.text.push(character);
            self.pending_space = false;
        }
    }

    fn space(&mut self) {
        if !self.text.is_empty() {
            self.pending_space = true;
        }
    }

    fn truncate(&mut self) {
        const ELLIPSIS: char = '\u{2026}';
        while self.text.len().saturating_add(ELLIPSIS.len_utf8()) > MAX_BOOK_TITLE_BYTES {
            self.text.pop();
        }
        self.text.push(ELLIPSIS);
        self.pending_space = false;
        self.truncated = true;
    }

    fn finish(self) -> Option<String> {
        (!self.discarded && !self.text.is_empty()).then_some(self.text)
    }
}

fn collapsed_text(value: &str) -> Option<String> {
    let mut text = CollapsedText::default();
    text.push(value);
    text.finish()
}

fn project_xhtml(
    source: &str,
    output: &mut TextWriter,
    headings: &mut HeadingCollector,
    toc_targets: &mut [TocTarget],
    path: &Path,
) -> Result<(), LoadError> {
    let mut resolver = ChapterTargetResolver::new(toc_targets);
    let mut reader = XmlReader::from_str(source);
    reader.config_mut().check_end_names = true;
    let decoder = reader.decoder();
    let mut depth = 0_usize;
    let mut suppressed = 0_usize;
    let mut preformatted = 0_usize;
    let mut in_body = false;
    let mut saw_body = false;
    let mut heading: Option<ActiveHeading> = None;
    if resolver.has_pending() {
        output.capture_next_content();
    }

    loop {
        match reader
            .read_event()
            .map_err(|source| invalid_epub(path, source))?
        {
            Event::Start(element) => {
                depth = depth
                    .checked_add(1)
                    .filter(|depth| *depth <= MAX_XML_DEPTH)
                    .ok_or_else(|| invalid_epub_message(path, "XHTML nesting exceeds its limit"))?;
                let name = element.local_name();
                let name = name.as_ref();
                if name.eq_ignore_ascii_case(b"body") {
                    if saw_body {
                        return Err(invalid_epub_message(path, "XHTML contains multiple bodies"));
                    }
                    saw_body = true;
                    in_body = true;
                    if match_toc_targets(&element, name, decoder, &mut resolver, path)? {
                        output.capture_next_content();
                    }
                    if element_hidden(&element, decoder, path)? {
                        suppressed = 1;
                    }
                    continue;
                }
                if !in_body {
                    continue;
                }
                if match_toc_targets(&element, name, decoder, &mut resolver, path)? {
                    output.capture_next_content();
                }
                if suppressed != 0 {
                    suppressed += 1;
                    continue;
                }
                if element_hidden(&element, decoder, path)? {
                    suppressed = 1;
                    continue;
                }
                if ignored_element(name) {
                    if image_element(name) {
                        output.image(image_alt(&element, decoder, path)?.as_deref())?;
                        resolver.resolve_capture(output);
                    }
                    suppressed = 1;
                    continue;
                }
                element_start(name, &element, decoder, output, path)?;
                if let Some(active) = heading.as_mut()
                    && name.eq_ignore_ascii_case(b"br")
                {
                    active.title.space();
                }
                if heading.is_none()
                    && let Some(level) = heading_level(name)
                {
                    output.flush_separator()?;
                    heading = Some(ActiveHeading {
                        depth,
                        level,
                        start: output.position(),
                        title: CollapsedText::default(),
                    });
                }
                if name.eq_ignore_ascii_case(b"pre") {
                    preformatted += 1;
                }
            }
            Event::Empty(element) => {
                let name = element.local_name();
                let name = name.as_ref();
                if name.eq_ignore_ascii_case(b"body") {
                    saw_body = true;
                } else if in_body {
                    if match_toc_targets(&element, name, decoder, &mut resolver, path)? {
                        output.capture_next_content();
                    }
                    if suppressed != 0 || element_hidden(&element, decoder, path)? {
                        continue;
                    }
                    element_start(name, &element, decoder, output, path)?;
                    if let Some(active) = heading.as_mut()
                        && name.eq_ignore_ascii_case(b"br")
                    {
                        active.title.space();
                    }
                    element_end(name, output)?;
                }
            }
            Event::End(element) => {
                let name = element.local_name();
                let name = name.as_ref();
                if in_body {
                    if suppressed != 0 {
                        suppressed -= 1;
                    } else {
                        if heading
                            .as_ref()
                            .is_some_and(|heading| heading.depth == depth)
                        {
                            let heading = heading.take().expect("matching heading is active");
                            let heading_end = output.position();
                            if let Some(title) = heading.title.finish() {
                                headings.push(title, heading.level, heading.start, heading_end);
                            }
                        }
                        if name.eq_ignore_ascii_case(b"pre") {
                            preformatted = preformatted.saturating_sub(1);
                        }
                        element_end(name, output)?;
                    }
                    if name.eq_ignore_ascii_case(b"body") {
                        in_body = false;
                    }
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid_epub_message(path, "XHTML nesting is invalid"))?;
            }
            Event::Text(text) if in_body && suppressed == 0 => {
                let text = text
                    .html_content()
                    .map_err(|source| invalid_epub(path, source))?;
                if let Some(heading) = heading.as_mut() {
                    heading.title.push(&text);
                }
                if preformatted == 0 {
                    output.text(&text)?;
                } else {
                    output.preformatted(&text)?;
                }
            }
            Event::CData(text) if in_body && suppressed == 0 => {
                let text = text.decode().map_err(|source| invalid_epub(path, source))?;
                if let Some(heading) = heading.as_mut() {
                    heading.title.push(&text);
                }
                if preformatted == 0 {
                    output.text(&text)?;
                } else {
                    output.preformatted(&text)?;
                }
            }
            Event::GeneralRef(reference) if in_body && suppressed == 0 => {
                if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|source| invalid_epub(path, source))?
                {
                    let mut encoded = [0_u8; 4];
                    let value = character.encode_utf8(&mut encoded);
                    if let Some(heading) = heading.as_mut() {
                        heading.title.push(value);
                    }
                    if preformatted == 0 {
                        output.text(value)?;
                    } else {
                        output.preformatted(value)?;
                    }
                } else {
                    let name = reference
                        .decode()
                        .map_err(|source| invalid_epub(path, source))?;
                    let value = resolve_predefined_entity(&name)
                        .ok_or_else(|| invalid_epub_message(path, "XHTML entity is undefined"))?;
                    if let Some(heading) = heading.as_mut() {
                        heading.title.push(value);
                    }
                    if preformatted == 0 {
                        output.text(value)?;
                    } else {
                        output.preformatted(value)?;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        resolver.resolve_capture(output);
    }
    resolver.finish(output);
    if !saw_body || depth != 0 {
        return Err(invalid_epub_message(
            path,
            "XHTML body is missing or incomplete",
        ));
    }
    Ok(())
}

fn match_toc_targets(
    element: &BytesStart<'_>,
    element_name: &[u8],
    decoder: Decoder,
    resolver: &mut ChapterTargetResolver<'_>,
    path: &Path,
) -> Result<bool, LoadError> {
    if !resolver.has_unresolved_fragments() {
        return Ok(false);
    }
    let mut matched = false;
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|source| invalid_epub(path, source))?;
        let name = attribute.key.as_ref();
        let is_id = name.eq_ignore_ascii_case(b"id") || name.eq_ignore_ascii_case(b"xml:id");
        let is_legacy_name = element_name.eq_ignore_ascii_case(b"a")
            && attribute
                .key
                .local_name()
                .as_ref()
                .eq_ignore_ascii_case(b"name");
        if !is_id && !is_legacy_name {
            continue;
        }
        let value = attribute
            .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
            .map_err(|source| invalid_epub(path, source))?;
        matched |= resolver.match_fragment(&value);
    }
    Ok(matched)
}

fn heading_level(name: &[u8]) -> Option<u8> {
    match name {
        name if name.eq_ignore_ascii_case(b"h1") => Some(1),
        name if name.eq_ignore_ascii_case(b"h2") => Some(2),
        name if name.eq_ignore_ascii_case(b"h3") => Some(3),
        name if name.eq_ignore_ascii_case(b"h4") => Some(4),
        name if name.eq_ignore_ascii_case(b"h5") => Some(5),
        name if name.eq_ignore_ascii_case(b"h6") => Some(6),
        _ => None,
    }
}

fn element_start(
    name: &[u8],
    element: &BytesStart<'_>,
    decoder: Decoder,
    output: &mut TextWriter,
    path: &Path,
) -> Result<(), LoadError> {
    if block_element(name) {
        output.paragraph();
    }
    if name.eq_ignore_ascii_case(b"br") {
        output.break_line();
    } else if name.eq_ignore_ascii_case(b"li") {
        output.literal("•")?;
        output.space();
    } else if name.eq_ignore_ascii_case(b"hr") {
        output.paragraph();
        output.literal("* * *")?;
        output.paragraph();
    } else if image_element(name) {
        output.image(image_alt(element, decoder, path)?.as_deref())?;
    } else if name.eq_ignore_ascii_case(b"rt") {
        output.literal("(")?;
    }
    Ok(())
}

fn element_end(name: &[u8], output: &mut TextWriter) -> Result<(), LoadError> {
    if name.eq_ignore_ascii_case(b"td") || name.eq_ignore_ascii_case(b"th") {
        output.tab();
    } else if name.eq_ignore_ascii_case(b"tr") || name.eq_ignore_ascii_case(b"li") {
        output.line();
    } else if name.eq_ignore_ascii_case(b"rt") {
        output.literal(")")?;
    }
    if block_element(name) {
        output.paragraph();
    }
    Ok(())
}

fn element_hidden(
    element: &BytesStart<'_>,
    decoder: Decoder,
    path: &Path,
) -> Result<bool, LoadError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|source| invalid_epub(path, source))?;
        let name = attribute.key.local_name();
        let name = name.as_ref();
        if name.eq_ignore_ascii_case(b"hidden") {
            return Ok(true);
        }
        if name.eq_ignore_ascii_case(b"aria-hidden") || name.eq_ignore_ascii_case(b"style") {
            let value = attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
                .map_err(|source| invalid_epub(path, source))?;
            if (name.eq_ignore_ascii_case(b"aria-hidden")
                && value.trim().eq_ignore_ascii_case("true"))
                || (name.eq_ignore_ascii_case(b"style") && style_hides(&value))
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn image_alt(
    element: &BytesStart<'_>,
    decoder: Decoder,
    path: &Path,
) -> Result<Option<String>, LoadError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|source| invalid_epub(path, source))?;
        if attribute
            .key
            .local_name()
            .as_ref()
            .eq_ignore_ascii_case(b"alt")
        {
            let value = attribute
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
                .map_err(|source| invalid_epub(path, source))?;
            return Ok(collapsed_text(&value).filter(|alt| useful_image_alt(alt)));
        }
    }
    Ok(None)
}

fn useful_image_alt(alt: &str) -> bool {
    let mut value = alt.trim();
    while let Some(inner) = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        value = inner.trim();
    }
    if let Some((kind, inner)) = value.split_once(':')
        && (kind.trim().eq_ignore_ascii_case("image")
            || kind.trim().eq_ignore_ascii_case("illustration"))
    {
        value = inner.trim();
        while let Some(inner) = value
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            value = inner.trim();
        }
    }
    let value = value.trim_end_matches(['.', '!', ':']).trim();
    !value.is_empty()
        && ![
            "cover",
            "image",
            "illustration",
            "image unavailable",
            "image not available",
            "illustration unavailable",
            "illustration not available",
        ]
        .iter()
        .any(|generic| value.eq_ignore_ascii_case(generic))
}

fn style_hides(style: &str) -> bool {
    style.split(';').any(|declaration| {
        let Some((property, value)) = declaration.split_once(':') else {
            return false;
        };
        let property = property.trim();
        let value = value
            .trim()
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default();
        (property.eq_ignore_ascii_case("display") && value.eq_ignore_ascii_case("none"))
            || (property.eq_ignore_ascii_case("visibility") && value.eq_ignore_ascii_case("hidden"))
    })
}

fn image_element(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"img")
        || name.eq_ignore_ascii_case(b"svg")
        || name.eq_ignore_ascii_case(b"canvas")
}

fn ignored_element(name: &[u8]) -> bool {
    [
        b"head".as_slice(),
        b"script",
        b"style",
        b"template",
        b"svg",
        b"canvas",
        b"rp",
    ]
    .iter()
    .any(|ignored| name.eq_ignore_ascii_case(ignored))
}

fn block_element(name: &[u8]) -> bool {
    [
        b"address".as_slice(),
        b"article",
        b"aside",
        b"blockquote",
        b"caption",
        b"dd",
        b"div",
        b"dl",
        b"dt",
        b"figcaption",
        b"figure",
        b"footer",
        b"form",
        b"h1",
        b"h2",
        b"h3",
        b"h4",
        b"h5",
        b"h6",
        b"header",
        b"main",
        b"nav",
        b"ol",
        b"p",
        b"pre",
        b"section",
        b"table",
        b"ul",
    ]
    .iter()
    .any(|block| name.eq_ignore_ascii_case(block))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Separator {
    None,
    Space,
    Tab,
    Line,
    Paragraph,
}

struct TextWriter {
    output: BufWriter<File>,
    path: PathBuf,
    limit: u64,
    length: u64,
    pending: Separator,
    ending_newlines: u8,
    horizontal_space: bool,
    has_content: bool,
    capture_next_content: bool,
    captured_content_start: Option<SourceOffset>,
}

impl TextWriter {
    fn new(file: File, path: PathBuf, limit: u64) -> Self {
        Self {
            output: BufWriter::new(file),
            path,
            limit,
            length: 0,
            pending: Separator::None,
            ending_newlines: 0,
            horizontal_space: false,
            has_content: false,
            capture_next_content: false,
            captured_content_start: None,
        }
    }

    fn chapter_break(&mut self) {
        self.request(Separator::Paragraph);
    }

    fn paragraph(&mut self) {
        self.request(Separator::Paragraph);
    }

    fn line(&mut self) {
        self.request(Separator::Line);
    }

    fn break_line(&mut self) {
        if self.pending == Separator::Line {
            self.pending = Separator::Paragraph;
        } else {
            self.request(Separator::Line);
        }
    }

    fn tab(&mut self) {
        self.request(Separator::Tab);
    }

    fn space(&mut self) {
        self.request(Separator::Space);
    }

    fn request(&mut self, separator: Separator) {
        self.pending = self.pending.max(separator);
    }

    const fn position(&self) -> SourceOffset {
        SourceOffset::new(self.length)
    }

    fn capture_next_content(&mut self) {
        if self.captured_content_start.is_none() {
            self.capture_next_content = true;
        }
    }

    fn take_captured_content_start(&mut self) -> Option<SourceOffset> {
        self.captured_content_start.take()
    }

    fn cancel_content_capture(&mut self) {
        self.capture_next_content = false;
        self.captured_content_start = None;
    }

    fn text(&mut self, text: &str) -> Result<(), LoadError> {
        for character in text.chars() {
            if character.is_whitespace() {
                self.space();
            } else {
                self.flush_separator()?;
                let mut encoded = [0_u8; 4];
                self.write_raw(character.encode_utf8(&mut encoded), true)?;
            }
        }
        Ok(())
    }

    fn preformatted(&mut self, text: &str) -> Result<(), LoadError> {
        self.flush_separator()?;
        let Some((content_start, _)) = text
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
        else {
            return self.write_raw(text, false);
        };
        if content_start != 0 {
            self.write_raw(&text[..content_start], false)?;
        }
        self.write_raw(&text[content_start..], true)
    }

    fn literal(&mut self, text: &str) -> Result<(), LoadError> {
        self.flush_separator()?;
        self.write_raw(text, true)
    }

    fn image(&mut self, alt: Option<&str>) -> Result<(), LoadError> {
        let Some(alt) = alt.filter(|alt| !alt.trim().is_empty()) else {
            return Ok(());
        };
        self.space();
        self.literal("[Illustration: ")?;
        self.text(alt)?;
        self.literal("]")?;
        self.space();
        Ok(())
    }

    fn flush_separator(&mut self) -> Result<(), LoadError> {
        let pending = std::mem::replace(&mut self.pending, Separator::None);
        if self.length == 0 {
            return Ok(());
        }
        match pending {
            Separator::None => {}
            Separator::Space if self.ending_newlines == 0 && !self.horizontal_space => {
                self.write_raw(" ", false)?;
            }
            Separator::Tab if self.ending_newlines == 0 => {
                if self.horizontal_space {
                    return Ok(());
                }
                self.write_raw("\t", false)?;
            }
            Separator::Line if self.ending_newlines == 0 => self.write_raw("\n", false)?,
            Separator::Paragraph => match self.ending_newlines {
                0 => self.write_raw("\n\n", false)?,
                1 => self.write_raw("\n", false)?,
                _ => {}
            },
            Separator::Space | Separator::Tab | Separator::Line => {}
        }
        Ok(())
    }

    fn write_raw(&mut self, value: &str, content: bool) -> Result<(), LoadError> {
        let value_len = u64::try_from(value.len()).expect("strings fit EPUB source coordinates");
        let next = self
            .length
            .checked_add(value_len)
            .filter(|next| *next <= self.limit)
            .ok_or_else(|| LoadError::EpubTextTooLarge {
                path: self.path.clone(),
                limit: self.limit,
            })?;
        if content && self.capture_next_content {
            self.captured_content_start = Some(self.position());
            self.capture_next_content = false;
        }
        self.output
            .write_all(value.as_bytes())
            .map_err(|source| LoadError::BufferEpub {
                path: self.path.clone(),
                source,
            })?;
        self.length = next;
        self.has_content |= content;
        if value.ends_with('\n') {
            let previous = self.ending_newlines;
            let trailing = value
                .as_bytes()
                .iter()
                .rev()
                .take_while(|byte| **byte == b'\n')
                .take(2)
                .count() as u8;
            self.ending_newlines = if usize::from(trailing) == value.len() {
                previous.saturating_add(trailing).min(2)
            } else {
                trailing
            };
            self.horizontal_space = false;
        } else if let Some(last) = value.as_bytes().last() {
            self.ending_newlines = 0;
            self.horizontal_space = matches!(last, b' ' | b'\t');
        }
        Ok(())
    }

    fn finish(mut self) -> Result<(File, u64), LoadError> {
        if !self.has_content {
            return Err(LoadError::NoLinearEpubContent(self.path));
        }
        self.pending = Separator::None;
        if self.ending_newlines == 0 {
            self.write_raw("\n", false)?;
        }
        self.output
            .flush()
            .map_err(|source| LoadError::BufferEpub {
                path: self.path.clone(),
                source,
            })?;
        let file = self
            .output
            .into_inner()
            .map_err(|error| LoadError::BufferEpub {
                path: self.path.clone(),
                source: error.into_error(),
            })?;
        Ok((file, self.length))
    }
}

fn read_exact_at(file: &File, mut output: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !output.is_empty() {
        match file.read_at(output, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(count) => {
                offset = offset
                    .checked_add(count as u64)
                    .ok_or(io::ErrorKind::InvalidData)?;
                output = &mut output[count..];
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn le_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes[..2].try_into().expect("ZIP fields are bounded"))
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("ZIP fields are bounded"))
}

fn le_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().expect("ZIP fields are bounded"))
}

fn invalid_epub(
    path: &Path,
    source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> LoadError {
    LoadError::InvalidEpub {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    }
}

fn invalid_epub_message(path: &Path, message: &'static str) -> LoadError {
    LoadError::InvalidEpub {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, message),
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, num::NonZeroUsize};

    use tempfile::{Builder, NamedTempFile, tempdir};
    use zip::{ZipWriter, write::SimpleFileOptions};

    use crate::{
        document::{DocumentCache, overwrite_with_distinct_fingerprint},
        source::SourceOffset,
    };

    use super::*;

    const CONTAINER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

    const CHAPTER_ONE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Hidden title</title><style>.secret { display: none; }</style></head>
  <body>
    <h1>Chapter One</h1>
    <p>Hello <em>wide</em> world &amp; &#x00E6;.</p>
    <p><img src="plate.jpg" alt="Plate I"/></p>
    <script>hidden script</script>
  </body>
</html>"#;

    const SUPPLEMENT: &str = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
<p>NONLINEAR IMAGE WRAPPER</p></body></html>"#;

    const CHAPTER_TWO: &str = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
<h2>Chapter Two</h2>
<ul><li>One</li><li>Two<br/>continued</li></ul>
<table><tr><td>A</td><td>B</td></tr></table>
<p hidden="hidden">hidden attribute</p>
<p aria-hidden="true">hidden aria</p>
<p style="display: none !important">hidden style</p>
<p>Visible <ruby>字<rt>zi</rt></ruby>.</p>
</body></html>"#;

    fn fixture(mimetype: &[u8]) -> NamedTempFile {
        fixture_version(mimetype, "2.0")
    }

    fn fixture_version(mimetype: &[u8], version: &str) -> NamedTempFile {
        let file = Builder::new().suffix(".epub").tempfile().unwrap();
        let mut writer = ZipWriter::new(file.reopen().unwrap());
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file("mimetype", stored).unwrap();
        writer.write_all(mimetype).unwrap();
        writer
            .start_file("META-INF/container.xml", deflated)
            .unwrap();
        writer.write_all(CONTAINER.as_bytes()).unwrap();

        let modified = if version == "3.0" {
            "<meta property=\"dcterms:modified\">2026-08-28T00:00:00Z</meta>"
        } else {
            ""
        };
        let package = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:uuid:test</dc:identifier>
    <dc:title>Test Book</dc:title>
    <dc:creator>Test Author</dc:creator>
    <dc:language>en</dc:language>
    {modified}
  </metadata>
  <manifest>
    <item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
    <item id="supplement" href="supplement.xhtml" media-type="application/xhtml+xml"/>
    <item id="two" href="two.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="one"/>
    <itemref idref="supplement" linear="no"/>
    <itemref idref="two"/>
  </spine>
</package>"#
        );
        for (name, contents) in [
            ("OEBPS/content.opf", package.as_str()),
            ("OEBPS/one.xhtml", CHAPTER_ONE),
            ("OEBPS/supplement.xhtml", SUPPLEMENT),
            ("OEBPS/two.xhtml", CHAPTER_TWO),
        ] {
            writer.start_file(name, deflated).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        file
    }

    fn fixture_with_package(package: &str, resources: &[(&str, &str)]) -> NamedTempFile {
        let file = Builder::new().suffix(".epub").tempfile().unwrap();
        let mut writer = ZipWriter::new(file.reopen().unwrap());
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file("mimetype", stored).unwrap();
        writer.write_all(EPUB_MIMETYPE).unwrap();
        writer
            .start_file("META-INF/container.xml", deflated)
            .unwrap();
        writer.write_all(CONTAINER.as_bytes()).unwrap();
        writer.start_file("OEBPS/content.opf", deflated).unwrap();
        writer.write_all(package.as_bytes()).unwrap();
        for (name, contents) in resources {
            writer.start_file(*name, deflated).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
        file
    }

    fn epub_two_navigation_fixture(ncx: &str) -> NamedTempFile {
        let package = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:uuid:ncx-test</dc:identifier>
    <dc:title>NCX Book</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
    <item id="supplement" href="supplement.xhtml" media-type="application/xhtml+xml"/>
    <item id="two" href="two.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="one"/><itemref idref="supplement" linear="no"/><itemref idref="two"/>
  </spine>
</package>"#;
        fixture_with_package(
            package,
            &[
                ("OEBPS/toc.ncx", ncx),
                (
                    "OEBPS/one.xhtml",
                    r#"<html xmlns="http://www.w3.org/1999/xhtml"><body>
                    <p>Front matter.</p><a id="opening"></a><p>Opening paragraph.</p>
                    <p id="parté">Unicode destination.</p>
                    <h2 id="styled">Styled Heading</h2><p>Body.</p></body></html>"#,
                ),
                (
                    "OEBPS/supplement.xhtml",
                    r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="extra">Supplement</h1></body></html>"#,
                ),
                (
                    "OEBPS/two.xhtml",
                    r#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Second chapter.</p><h3>Second Heading</h3></body></html>"#,
                ),
            ],
        )
    }

    fn read_all(document: &Document) -> String {
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut cursor = reader.source_start();
        let mut output = String::new();
        let target = NonZeroUsize::new(4_096).unwrap();
        while cursor < reader.source_end() {
            let window = reader.window(cursor, target).unwrap();
            output.push_str(window.as_str());
            cursor = window.end();
        }
        output
    }

    fn project(source: &str) -> (String, Vec<CapturedHeading>) {
        let file = tempfile::tempfile().unwrap();
        let mut output = TextWriter::new(file, PathBuf::from("project.epub"), MAX_FILE_BYTES);
        let mut headings = HeadingCollector::default();
        let mut targets = Vec::new();
        project_xhtml(
            source,
            &mut output,
            &mut headings,
            &mut targets,
            Path::new("project.epub"),
        )
        .unwrap();
        let (mut file, _) = output.finish().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        (text, headings.captured)
    }

    #[test]
    fn extension_detection_is_case_insensitive() {
        assert!(is_path(Path::new("book.EPUB")));
        assert!(!is_path(Path::new("book.epub.zip")));
        assert!(!is_path(Path::new("epub")));
    }

    #[test]
    fn projects_only_linear_spine_content() {
        let fixture = fixture(EPUB_MIMETYPE);
        let document = load(fixture.path().to_path_buf()).unwrap();
        let text = read_all(&document);

        assert!(text.starts_with("Chapter One\n\nHello wide world & æ."));
        assert!(text.contains("[Illustration: Plate I]"));
        assert!(text.contains("Chapter Two"));
        assert!(text.contains("• One\n• Two\ncontinued"));
        assert!(text.contains("A\tB"));
        assert!(text.contains("Visible 字(zi)."));
        assert!(!text.contains("NONLINEAR"));
        assert!(!text.contains("Hidden title"));
        assert!(!text.contains("hidden script"));
        assert!(!text.contains("hidden attribute"));
        assert!(!text.contains("hidden aria"));
        assert!(!text.contains("hidden style"));
        let book = document.book().unwrap();
        assert_eq!(book.title(), Some("Test Book"));
        assert_eq!(book.creator(), Some("Test Author"));
        assert_eq!(book.sections().len(), 2);
        assert_eq!(book.sections()[0].title(), "Chapter One");
        assert_eq!(book.sections()[0].level(), 1);
        assert_eq!(book.sections()[1].title(), "Chapter Two");
        assert_eq!(book.sections()[1].level(), 2);
        assert_eq!(book.headings().len(), 2);
        assert_eq!(document.preferred_start(), book.headings()[0].start());
        for (section, heading) in book.sections().iter().zip(book.headings()) {
            assert_eq!(section.target(), heading.start());
            let start = usize::try_from(heading.start().get()).unwrap();
            let end = usize::try_from(heading.end().get()).unwrap();
            assert_eq!(
                text[start..end].split_whitespace().collect::<Vec<_>>(),
                section.title().split_whitespace().collect::<Vec<_>>()
            );
        }
        assert!(document.input_identity().is_some());
        document.validate().unwrap();
    }

    #[test]
    fn epub_two_ncx_drives_navigation_without_replacing_heading_ranges() {
        let ncx = r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <docTitle><text>NCX Book</text></docTitle><navMap>
    <navPoint id="one"><navLabel><text>Opening</text></navLabel>
      <content src="one.xhtml#opening"/>
      <navPoint id="unicode"><navLabel><text>Unicode Label</text></navLabel>
        <content src="one.xhtml#part%C3%A9"/></navPoint>
      <navPoint id="styled"><navLabel><text>Styled from NCX</text></navLabel>
        <content src="one.xhtml#styled"/></navPoint>
      <navPoint id="missing"><navLabel><text>Missing</text></navLabel>
        <content src="one.xhtml#missing"/></navPoint>
    </navPoint>
    <navPoint id="supplement"><navLabel><text>Supplement</text></navLabel>
      <content src="supplement.xhtml#extra"/></navPoint>
    <navPoint id="two"><navLabel><text>Second from NCX</text></navLabel>
      <content src="two.xhtml"/></navPoint>
  </navMap>
</ncx>"#;
        let fixture = epub_two_navigation_fixture(ncx);
        let document = load(fixture.path().to_path_buf()).unwrap();
        let text = read_all(&document);
        let book = document.book().unwrap();

        assert_eq!(
            book.sections()
                .iter()
                .map(BookSection::title)
                .collect::<Vec<_>>(),
            [
                "Opening",
                "Unicode Label",
                "Styled from NCX",
                "Second from NCX"
            ]
        );
        assert_eq!(
            book.sections()
                .iter()
                .map(BookSection::level)
                .collect::<Vec<_>>(),
            [1, 2, 2, 1]
        );
        for (section, visible) in book.sections().iter().zip([
            "Opening paragraph.",
            "Unicode destination.",
            "Styled Heading",
            "Second chapter.",
        ]) {
            let target = usize::try_from(section.target().get()).unwrap();
            assert!(text[target..].starts_with(visible));
        }
        assert_eq!(book.headings().len(), 2);
        assert_eq!(book.headings()[0].level(), 2);
        assert_eq!(book.headings()[0].start(), book.sections()[2].target());
        assert_eq!(book.headings()[1].level(), 3);
        assert_eq!(document.preferred_start(), book.headings()[0].start());
        assert!(!text.contains("Supplement"));
    }

    #[test]
    fn epub_three_nav_is_preferred_over_legacy_ncx() {
        let package = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">urn:uuid:nav-test</dc:identifier><dc:title>Nav Book</dc:title>
    <dc:language>en</dc:language><meta property="dcterms:modified">2026-08-28T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
    <item id="two" href="two.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx"><itemref idref="one"/><itemref idref="two"/></spine>
</package>"#;
        let nav = r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="toc"><h1>Contents</h1><ol>
  <li><a href="one.xhtml#nav-start">EPUB 3 Label</a><ol>
    <li><a href="one.xhtml#heading">Nested Label</a></li></ol></li>
  <li><a href="two.xhtml">Last Label</a></li>
</ol></nav></body></html>"#;
        let ncx = r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
<docTitle><text>Legacy</text></docTitle><navMap><navPoint id="old">
<navLabel><text>NCX Label</text></navLabel><content src="one.xhtml#nav-start"/>
</navPoint></navMap></ncx>"#;
        let fixture = fixture_with_package(
            package,
            &[
                (
                    "OEBPS/one.xhtml",
                    r#"<html><body><p id="nav-start">Nav body.</p><h1 id="heading">Actual Heading</h1></body></html>"#,
                ),
                (
                    "OEBPS/two.xhtml",
                    r#"<html><body><p>Last body.</p></body></html>"#,
                ),
                ("OEBPS/nav.xhtml", nav),
                ("OEBPS/toc.ncx", ncx),
            ],
        );
        let document = load(fixture.path().to_path_buf()).unwrap();
        let text = read_all(&document);
        let book = document.book().unwrap();

        assert_eq!(
            book.sections()
                .iter()
                .map(BookSection::title)
                .collect::<Vec<_>>(),
            ["EPUB 3 Label", "Nested Label", "Last Label"]
        );
        assert_eq!(
            book.sections()
                .iter()
                .map(BookSection::level)
                .collect::<Vec<_>>(),
            [1, 2, 1]
        );
        assert!(
            !book
                .sections()
                .iter()
                .any(|section| section.title() == "NCX Label")
        );
        for (section, visible) in
            book.sections()
                .iter()
                .zip(["Nav body.", "Actual Heading", "Last body."])
        {
            let target = usize::try_from(section.target().get()).unwrap();
            assert!(text[target..].starts_with(visible));
        }
        assert_eq!(book.headings().len(), 1);
        assert_eq!(book.headings()[0].start(), book.sections()[1].target());
    }

    #[test]
    fn malformed_toc_retries_without_it_and_uses_heading_navigation() {
        let fixture = epub_two_navigation_fixture("<ncx><navMap></ncx>");
        let archive = File::open(fixture.path()).unwrap();
        let (_, has_toc) = read_epub(archive, fixture.path()).unwrap();
        assert!(!has_toc);

        let document = load(fixture.path().to_path_buf()).unwrap();
        let book = document.book().unwrap();
        assert_eq!(
            book.sections()
                .iter()
                .map(BookSection::title)
                .collect::<Vec<_>>(),
            ["Styled Heading", "Second Heading"]
        );
        assert_eq!(book.sections().len(), book.headings().len());
    }

    #[test]
    fn chapter_targets_begin_at_preformatted_content_not_its_leading_space() {
        let mut toc = TocTargets {
            targets: vec![TocTarget {
                manifest_index: 0,
                fragment: None,
                title: "Code".to_owned(),
                level: 1,
                ordinal: 1,
                resolved: None,
                attempted: false,
                active: false,
                awaiting: false,
            }],
            ..TocTargets::default()
        };
        let file = tempfile::tempfile().unwrap();
        let mut output = TextWriter::new(file, PathBuf::from("pre.epub"), MAX_FILE_BYTES);
        let mut headings = HeadingCollector::default();
        project_xhtml(
            "<html><body><pre>\n  code</pre></body></html>",
            &mut output,
            &mut headings,
            toc.for_manifest(0),
            Path::new("pre.epub"),
        )
        .unwrap();
        let (mut file, _) = output.finish().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        let sections = toc.finish();

        assert_eq!(sections.len(), 1);
        let target = usize::try_from(sections[0].target().get()).unwrap();
        assert!(text[target..].starts_with("code"));
        assert!(text[..target].chars().all(char::is_whitespace));
    }

    #[test]
    fn unresolved_fragment_targets_do_not_cross_repeated_spine_chapters() {
        let mut toc = TocTargets {
            targets: vec![TocTarget {
                manifest_index: 0,
                fragment: Some("late".to_owned()),
                title: "Late".to_owned(),
                level: 1,
                ordinal: 1,
                resolved: None,
                attempted: false,
                active: false,
                awaiting: false,
            }],
            ..TocTargets::default()
        };
        let file = tempfile::tempfile().unwrap();
        let mut output = TextWriter::new(file, PathBuf::from("repeat.epub"), MAX_FILE_BYTES);
        let mut headings = HeadingCollector::default();
        project_xhtml(
            "<html><body><p>First occurrence.</p><a id=\"late\"></a></body></html>",
            &mut output,
            &mut headings,
            toc.for_manifest(0),
            Path::new("repeat.epub"),
        )
        .unwrap();
        output.chapter_break();
        project_xhtml(
            "<html><body><p id=\"late\">Second occurrence.</p></body></html>",
            &mut output,
            &mut headings,
            toc.for_manifest(0),
            Path::new("repeat.epub"),
        )
        .unwrap();
        output.finish().unwrap();

        assert!(toc.finish().is_empty());
    }

    #[test]
    fn toc_traversal_budget_counts_unusable_nodes() {
        let package = r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:many</dc:identifier>
<dc:title>Many</dc:title><dc:language>en</dc:language></metadata>
<manifest><item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>
<spine><itemref idref="one"/></spine></package>"#;
        let mut nav = String::from(
            r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>
<nav epub:type="toc"><ol>"#,
        );
        for index in 0..=MAX_BOOK_SECTIONS {
            use std::fmt::Write as _;
            write!(
                nav,
                "<li><a href=\"missing-{index}.xhtml\">Missing {index}</a></li>"
            )
            .unwrap();
        }
        nav.push_str("</ol></nav></body></html>");
        let fixture = fixture_with_package(
            package,
            &[
                (
                    "OEBPS/one.xhtml",
                    "<html><body><h1>Fallback</h1></body></html>",
                ),
                ("OEBPS/nav.xhtml", &nav),
            ],
        );
        let archive = File::open(fixture.path()).unwrap();
        let (epub, has_toc) = read_epub(archive, fixture.path()).unwrap();
        assert!(has_toc);
        let toc = TocTargets::collect(&epub);

        assert_eq!(toc.visited, MAX_BOOK_SECTIONS);
        assert!(toc.saturated);
        assert!(toc.targets.is_empty());
    }

    #[test]
    fn headings_capture_collapsed_titles_and_projected_ranges() {
        let (text, sections) = project(
            r#"<html><body><p>Front matter.</p><h1> THE<br/> Principles <i>of</i>
            Ornament </h1><h2 hidden="hidden">Invisible</h2><h3>Part &amp; Form</h3></body></html>"#,
        );

        assert_eq!(sections.len(), 2);
        assert_eq!(
            sections[0].title.as_deref(),
            Some("THE Principles of Ornament")
        );
        assert_eq!(sections[0].heading.level(), 1);
        assert_eq!(sections[1].title.as_deref(), Some("Part & Form"));
        assert_eq!(sections[1].heading.level(), 3);
        for section in &sections {
            let start = usize::try_from(section.heading.start().get()).unwrap();
            let end = usize::try_from(section.heading.end().get()).unwrap();
            let projected = text[start..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(projected, section.title.as_deref().unwrap());
        }
    }

    #[test]
    fn consecutive_breaks_preserve_a_blank_line() {
        let (text, _) = project("<html><body><p>one<br/><br/>two</p></body></html>");
        assert_eq!(text, "one\n\ntwo\n");
    }

    #[test]
    fn table_rows_and_list_items_use_single_line_breaks() {
        let (text, _) = project(
            "<html><body><table><thead><tr><th>A</th><th>B</th></tr></thead>\
             <tbody><tr><td>1</td><td>2</td></tr><tr><td>3</td><td>4</td></tr></tbody>\
             </table><ul><li>One</li><li>Two</li></ul></body></html>",
        );
        assert_eq!(text, "A\tB\n1\t2\n3\t4\n\n• One\n• Two\n");
    }

    #[test]
    fn generic_image_alternatives_are_silent() {
        let (text, _) = project(
            r#"<html><body><p>
            <img src="a" alt="[Image unavailable.]"/>
            <img src="b" alt="[Image: [Image unavailable.]]"/>
            <img src="c" alt="Cover"/>
            <img src="d" alt=""/>
            <img src="e"/>
            <img src="f" alt="Plate I"/>
            </p></body></html>"#,
        );
        assert_eq!(text, "[Illustration: Plate I]\n");
        assert!(!text.contains("unavailable"));
    }

    #[test]
    fn horizontal_rules_remain_visible() {
        let (text, _) = project("<html><body><p>Before</p><hr/><p>After</p></body></html>");
        assert_eq!(text, "Before\n\n* * *\n\nAfter\n");
    }

    #[test]
    fn section_limits_degrade_without_rejecting_projection() {
        let mut collector = HeadingCollector::default();
        for index in 0..=MAX_BOOK_HEADINGS {
            let start = SourceOffset::from_usize(index * 2);
            collector.push("x".to_owned(), 1, start, start.checked_add(1).unwrap());
        }
        assert_eq!(collector.captured.len(), MAX_BOOK_HEADINGS);

        let oversized = "x".repeat(MAX_BOOK_TITLE_BYTES + 1);
        let (text, sections) = project(&format!(
            "<html><body><h1>{oversized}</h1><p>body</p></body></html>"
        ));
        assert!(text.contains("body"));
        assert_eq!(sections.len(), 1);
        let title = sections[0].title.as_deref().unwrap();
        assert!(title.len() <= MAX_BOOK_TITLE_BYTES);
        assert!(title.ends_with('…'));

        let mut collector = HeadingCollector::default();
        let title = "x".repeat(MAX_BOOK_TITLE_BYTES);
        let retained = MAX_BOOK_TITLE_TOTAL_BYTES / MAX_BOOK_TITLE_BYTES;
        for index in 0..=retained {
            let start = SourceOffset::from_usize(index * 2);
            collector.push(title.clone(), 1, start, start.checked_add(1).unwrap());
        }
        let (sections, headings) = collector.finish(Vec::new());
        assert_eq!(sections.len(), retained);
        assert_eq!(headings.len(), retained + 1);
    }

    #[test]
    fn fragment_decoding_is_bounded_before_allocation() {
        let limit = "%61".repeat(MAX_TOC_FRAGMENT_BYTES);
        assert_eq!(
            decoded_fragment(&limit).unwrap().len(),
            MAX_TOC_FRAGMENT_BYTES
        );

        let oversized = format!("{limit}%61");
        assert!(decoded_fragment(&oversized).is_none());
    }

    #[test]
    fn projects_epub_three_spine_content() {
        let fixture = fixture_version(EPUB_MIMETYPE, "3.0");
        let document = load(fixture.path().to_path_buf()).unwrap();
        let text = read_all(&document);
        assert!(text.contains("Chapter One"));
        assert!(text.contains("Chapter Two"));
        assert!(!text.contains("NONLINEAR"));
    }

    #[test]
    fn projected_documents_continue_tracking_the_origin() {
        let fixture = fixture(EPUB_MIMETYPE);
        let document = load(fixture.path().to_path_buf()).unwrap();
        overwrite_with_distinct_fingerprint(fixture.path(), b"changed");
        assert!(matches!(document.validate(), Err(LoadError::Read { .. })));
    }

    #[test]
    fn rejects_an_invalid_mimetype_entry() {
        let fixture = fixture(b"application/zip");
        assert!(matches!(
            load(fixture.path().to_path_buf()),
            Err(LoadError::InvalidEpub { .. })
        ));
    }

    #[test]
    fn rejects_html_spine_content_instead_of_parsing_it_as_xhtml() {
        let package = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="2.0" unique-identifier="book-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">urn:uuid:html-test</dc:identifier>
    <dc:title>HTML Book</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest><item id="one" href="one.html" media-type="text/html"/></manifest>
  <spine><itemref idref="one"/></spine>
</package>"#;
        let fixture = fixture_with_package(
            package,
            &[(
                "OEBPS/one.html",
                "<html><body><p>Well-formed but not XHTML.</p></body></html>",
            )],
        );

        assert!(matches!(
            load(fixture.path().to_path_buf()),
            Err(LoadError::InvalidEpub { .. })
        ));
    }

    #[test]
    fn spine_limit_accepts_the_boundary_and_rejects_the_next_byte_atomically() {
        let path = Path::new("bounded.epub");
        let mut total = 6;
        add_spine_bytes(&mut total, 4, 10, path).unwrap();
        assert_eq!(total, 10);

        assert!(matches!(
            add_spine_bytes(&mut total, 1, 10, path),
            Err(LoadError::EpubExpandedTooLarge { limit: 10, .. })
        ));
        assert_eq!(total, 10);
    }

    #[test]
    fn eocd_entry_limit_is_checked_before_zip_construction() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("many.epub");
        let mut bytes = [0_u8; EOCD_LEN];
        bytes[..4].copy_from_slice(EOCD_SIGNATURE);
        bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&2_u16.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let file = File::open(&path).unwrap();
        let limits = ArchiveLimits {
            entries: 1,
            ..ARCHIVE_LIMITS
        };
        assert!(matches!(
            inspect_eocd(&file, &path, limits),
            Err(LoadError::EpubTooManyEntries { limit: 1, .. })
        ));
    }

    #[test]
    fn eocd_central_directory_limit_is_exact() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("directory.epub");
        let mut bytes = vec![0_u8; EOCD_LEN + 1];
        bytes[1..5].copy_from_slice(EOCD_SIGNATURE);
        bytes[13..17].copy_from_slice(&1_u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let file = File::open(&path).unwrap();
        let exact = ArchiveLimits {
            central_directory_bytes: 1,
            ..ARCHIVE_LIMITS
        };
        inspect_eocd(&file, &path, exact).unwrap();

        let exceeded = ArchiveLimits {
            central_directory_bytes: 0,
            ..ARCHIVE_LIMITS
        };
        assert!(matches!(
            inspect_eocd(&file, &path, exceeded),
            Err(LoadError::InvalidEpub { .. })
        ));
    }

    #[test]
    fn preflight_entry_name_limits_are_exact() {
        let file = Builder::new().suffix(".epub").tempfile().unwrap();
        let mut writer = ZipWriter::new(file.reopen().unwrap());
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("mimetype", stored).unwrap();
        writer.write_all(EPUB_MIMETYPE).unwrap();
        writer.start_file("a", stored).unwrap();
        writer.finish().unwrap();

        let exact = ArchiveLimits {
            entry_name_bytes: 8,
            entry_name_total_bytes: 9,
            ..ARCHIVE_LIMITS
        };
        preflight(File::open(file.path()).unwrap(), file.path(), exact).unwrap();

        for limits in [
            ArchiveLimits {
                entry_name_bytes: 7,
                entry_name_total_bytes: 9,
                ..ARCHIVE_LIMITS
            },
            ArchiveLimits {
                entry_name_bytes: 8,
                entry_name_total_bytes: 8,
                ..ARCHIVE_LIMITS
            },
        ] {
            assert!(matches!(
                preflight(File::open(file.path()).unwrap(), file.path(), limits),
                Err(LoadError::InvalidEpub { .. })
            ));
        }
    }

    #[test]
    fn preflight_counts_actual_expansion() {
        let fixture = fixture(EPUB_MIMETYPE);
        let file = File::open(fixture.path()).unwrap();
        let limits = ArchiveLimits {
            expanded_bytes: 64,
            ..ARCHIVE_LIMITS
        };
        assert!(matches!(
            preflight(file, fixture.path(), limits),
            Err(LoadError::EpubExpandedTooLarge { limit: 64, .. })
        ));
    }

    #[test]
    fn text_writer_enforces_projection_limit_without_partial_overflow() {
        let path = PathBuf::from("small.epub");
        let file = tempfile::tempfile().unwrap();
        let mut writer = TextWriter::new(file, path, 4);
        writer.text("abcd").unwrap();
        assert!(matches!(
            writer.text("e"),
            Err(LoadError::EpubTextTooLarge { limit: 4, .. })
        ));
    }

    #[test]
    fn xhtml_depth_is_bounded() {
        let mut source = String::from("<html><body>");
        source.extend(std::iter::repeat_n("<div>", MAX_XML_DEPTH));
        source.extend(std::iter::repeat_n("</div>", MAX_XML_DEPTH));
        source.push_str("</body></html>");
        let file = tempfile::tempfile().unwrap();
        let mut output = TextWriter::new(file, PathBuf::from("deep.epub"), MAX_FILE_BYTES);
        let mut headings = HeadingCollector::default();
        let mut targets = Vec::new();
        assert!(matches!(
            project_xhtml(
                &source,
                &mut output,
                &mut headings,
                &mut targets,
                Path::new("deep.epub")
            ),
            Err(LoadError::InvalidEpub { .. })
        ));
    }

    #[test]
    fn entry_names_reject_archive_paths() {
        for name in ["", "/root", "../book", "a/./b", "a//b", "a\\b", "a\0b"] {
            assert!(!safe_entry_name(name), "accepted {name:?}");
        }
        for name in ["mimetype", "META-INF/", "OEBPS/text/chapter.xhtml"] {
            assert!(safe_entry_name(name), "rejected {name:?}");
        }
    }

    #[test]
    fn paragraph_separator_accumulates_across_buffered_writes() {
        let file = tempfile::tempfile().unwrap();
        let mut writer = TextWriter::new(file, PathBuf::from("spacing.epub"), 64);
        writer.literal("one").unwrap();
        writer.line();
        writer.literal("two").unwrap();
        writer.paragraph();
        writer.literal("three").unwrap();
        let (mut file, length) = writer.finish().unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "one\ntwo\n\nthree\n");
        assert_eq!(length, text.len() as u64);
        assert_eq!(SourceOffset::new(length).get(), text.len() as u64);
    }

    #[test]
    fn zip_probe_rejects_truncated_records() {
        let path = Path::new("truncated.epub");
        let file = tempfile::tempfile().unwrap();
        file.set_len(EOCD_LEN as u64).unwrap();
        assert!(matches!(
            inspect_eocd(&file, path, ARCHIVE_LIMITS),
            Err(LoadError::InvalidEpub { .. })
        ));

        let mut cursor = Cursor::new(Vec::<u8>::new());
        cursor.write_all(EOCD_SIGNATURE).unwrap();
        assert_eq!(cursor.into_inner(), EOCD_SIGNATURE);
    }
}
