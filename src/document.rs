use std::{
    fs::File,
    io,
    num::NonZeroUsize,
    os::unix::fs::{FileExt, MetadataExt},
    path::{Path, PathBuf},
};

use rustix::fs::{Mode, OFlags};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    error::{LoadError, sanitize_os},
    line_index::{LineIndex, LineIndexError, LinePosition, LineScan},
    source::{SourceOffset, SourceText, WindowRequest},
};

pub const MAX_FILE_BYTES: usize = 33_554_432;
pub(super) const SOURCE_WINDOW_BYTES: usize = 64 * 1024;
const UTF8_BOM_BYTES: usize = 3;
const UTF8_BOUNDARY_SLOP_BYTES: usize = 3;
const MAX_GRAPHEME_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(super) struct Document {
    store: DocumentStore,
    line_index: LineIndex,
    source_start: SourceOffset,
    source_end: SourceOffset,
    path: PathBuf,
    display_path: String,
    display_name: String,
}

impl Document {
    #[cfg(test)]
    pub(super) fn source(&self) -> SourceText<'_> {
        self.store
            .contiguous_source()
            .expect("test documents use contiguous storage")
    }

    pub(super) const fn source_start(&self) -> SourceOffset {
        self.source_start
    }

    pub(super) const fn source_end(&self) -> SourceOffset {
        self.source_end
    }

    pub(super) fn display_path(&self) -> &str {
        &self.display_path
    }

    pub(super) fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(super) fn reader<'a>(&'a self, cache: &'a mut DocumentCache) -> DocumentReader<'a> {
        DocumentReader {
            document: self,
            cache,
        }
    }

    #[cfg(test)]
    pub(super) fn from_text(path: &Path, text: String) -> Self {
        Self::new(path, text).expect("test documents fit the line-index budget")
    }

    #[cfg(test)]
    pub(super) fn from_text_at(path: &Path, text: String, start: SourceOffset) -> Self {
        let store = DocumentStore::InMemory(InMemoryStore::with_start(text, start));
        let line_index =
            build_line_index(&store, path).expect("test documents fit the line-index budget");
        Self::from_parts(path, store, line_index)
    }

    #[cfg(test)]
    fn new(path: &Path, source: String) -> Result<Self, LoadError> {
        let store = DocumentStore::in_memory(source);
        let line_index = build_line_index(&store, path)?;
        Ok(Self::from_parts(path, store, line_index))
    }

    fn from_parts(path: &Path, store: DocumentStore, line_index: LineIndex) -> Self {
        let source_start = store.source_start();
        let source_end = store.source_end();
        Self {
            store,
            line_index,
            source_start,
            source_end,
            path: path.to_path_buf(),
            display_path: sanitize_os(path.as_os_str()),
            display_name: sanitize_os(path.file_name().unwrap_or(path.as_os_str())),
        }
    }
}

#[derive(Debug)]
pub(super) struct DocumentCache {
    chunk: Vec<u8>,
    grapheme: Vec<u8>,
    window_bytes: NonZeroUsize,
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self {
            chunk: Vec::new(),
            grapheme: Vec::new(),
            window_bytes: NonZeroUsize::new(SOURCE_WINDOW_BYTES)
                .expect("source window size is nonzero"),
        }
    }
}

impl DocumentCache {
    #[cfg(test)]
    pub(super) fn with_window_bytes(window_bytes: usize) -> Self {
        Self {
            window_bytes: NonZeroUsize::new(window_bytes).expect("test window size is nonzero"),
            ..Self::default()
        }
    }
}

pub(super) struct DocumentReader<'a> {
    document: &'a Document,
    cache: &'a mut DocumentCache,
}

impl<'document> DocumentReader<'document> {
    pub(super) fn graphemes<'reader>(
        &'reader mut self,
        start: SourceOffset,
    ) -> Result<DocumentGraphemes<'reader, 'document>, LoadError> {
        self.require_char_boundary(start, "invalid grapheme cursor offset")?;
        self.cache.grapheme.clear();
        Ok(DocumentGraphemes {
            reader: self,
            cursor: 0,
            next_start: start,
            loaded_end: start,
        })
    }

    pub(super) fn window(
        &mut self,
        start: SourceOffset,
        target_bytes: NonZeroUsize,
    ) -> Result<SourceText<'_>, LoadError> {
        self.document.store.copy_window(
            WindowRequest::new(start, target_bytes),
            &mut self.cache.chunk,
        )
    }

    pub(super) fn source_start(&self) -> SourceOffset {
        self.document.source_start()
    }

    pub(super) fn source_end(&self) -> SourceOffset {
        self.document.source_end()
    }

    pub(super) fn line_position(
        &mut self,
        offset: SourceOffset,
    ) -> Result<LinePosition, LoadError> {
        let (scan, lines) = self.scan_lines(offset)?;
        scan.finish(lines.count)
            .ok_or_else(|| self.protocol_error("physical-line coordinates overflow"))
    }

    pub(super) fn line_start_at_or_before(
        &mut self,
        offset: SourceOffset,
    ) -> Result<SourceOffset, LoadError> {
        let (_, lines) = self.scan_lines(offset)?;
        Ok(lines.last_start)
    }

    pub(super) fn previous_char_start(
        &mut self,
        offset: SourceOffset,
    ) -> Result<Option<SourceOffset>, LoadError> {
        self.require_char_boundary(offset, "invalid character offset")?;
        if offset == self.source_start() {
            return Ok(None);
        }

        let available = usize::try_from(offset.get() - self.source_start().get())
            .unwrap_or(usize::MAX)
            .min(4);
        let start = SourceOffset::new(offset.get() - available as u64);
        self.document
            .store
            .copy_bytes(start, available, &mut self.cache.chunk)?;
        let Some(relative) = self
            .cache
            .chunk
            .iter()
            .rposition(|byte| byte & 0b1100_0000 != 0b1000_0000)
        else {
            return Err(self.protocol_error("invalid UTF-8 before character offset"));
        };
        let scalar = std::str::from_utf8(&self.cache.chunk[relative..])
            .map_err(|_| self.protocol_error("invalid UTF-8 before character offset"))?;
        if scalar.chars().count() != 1 {
            return Err(self.protocol_error("invalid character boundary"));
        }
        start
            .checked_add(relative)
            .map(Some)
            .ok_or_else(|| self.protocol_error("character coordinates overflow"))
    }

    fn scan_lines(&mut self, offset: SourceOffset) -> Result<(LineScan, ScannedLines), LoadError> {
        self.require_char_boundary(offset, "invalid physical-line offset")?;
        let scan = self
            .document
            .line_index
            .scan_from(offset)
            .ok_or_else(|| self.protocol_error("invalid physical-line offset"))?;
        let scan_end = if offset < self.source_end() {
            offset
                .checked_add(1)
                .ok_or_else(|| self.protocol_error("physical-line coordinates overflow"))?
        } else {
            offset
        };
        let mut scanner = LineScanner::new(offset, scan.start());
        let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
        let mut cursor = scan.start();

        while cursor < scan_end {
            let (window_start, window_end, processed, valid_coordinates) = {
                let window = self.window(cursor, target)?;
                let end = window.end().min(scan_end);
                let length = usize::try_from(end.get() - window.start().get())
                    .expect("document windows fit the process address space");
                let valid_coordinates =
                    scanner.extend(window.start(), &window.as_str().as_bytes()[..length]);
                (window.start(), end, length, valid_coordinates)
            };
            if valid_coordinates.is_none() {
                return Err(self.protocol_error("physical-line coordinates overflow"));
            }
            if window_start != cursor || processed == 0 || window_end <= cursor {
                return Err(self.protocol_error("non-contiguous physical-line window"));
            }
            cursor = window_end;
        }

        scanner
            .finish()
            .ok_or_else(|| self.protocol_error("physical-line coordinates overflow"))?;
        Ok((scan, scanner.scanned()))
    }

    fn require_char_boundary(
        &mut self,
        offset: SourceOffset,
        message: &'static str,
    ) -> Result<(), LoadError> {
        if offset < self.source_start() || offset > self.source_end() {
            return Err(self.protocol_error(message));
        }
        if offset == self.source_end() {
            return Ok(());
        }
        self.document
            .store
            .copy_bytes(offset, 1, &mut self.cache.chunk)?;
        if self.cache.chunk[0] & 0b1100_0000 == 0b1000_0000 {
            return Err(self.protocol_error(message));
        }
        Ok(())
    }

    fn protocol_error(&self, message: &'static str) -> LoadError {
        LoadError::Read {
            path: self.document.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidData, message),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScannedLines {
    count: u64,
    last_start: SourceOffset,
}

struct LineScanner {
    through: SourceOffset,
    scanned: ScannedLines,
    pending_cr: Option<SourceOffset>,
}

impl LineScanner {
    const fn new(through: SourceOffset, start: SourceOffset) -> Self {
        Self {
            through,
            scanned: ScannedLines {
                count: 0,
                last_start: start,
            },
            pending_cr: None,
        }
    }

    fn extend(&mut self, start: SourceOffset, bytes: &[u8]) -> Option<()> {
        for (relative, byte) in bytes.iter().copied().enumerate() {
            let position = start.checked_add(relative)?;
            if let Some(cr_end) = self.pending_cr.take() {
                if byte == b'\n' {
                    self.record(position.checked_add(1)?)?;
                    continue;
                }
                self.record(cr_end)?;
            }
            match byte {
                b'\r' => {
                    self.pending_cr = Some(position.checked_add(1)?);
                }
                b'\n' => self.record(position.checked_add(1)?)?,
                _ => {}
            }
        }
        Some(())
    }

    fn finish(&mut self) -> Option<()> {
        if let Some(cr_end) = self.pending_cr.take() {
            self.record(cr_end)?;
        }
        Some(())
    }

    const fn scanned(&self) -> ScannedLines {
        self.scanned
    }

    fn record(&mut self, start: SourceOffset) -> Option<()> {
        if start <= self.through {
            self.scanned.count = self.scanned.count.checked_add(1)?;
            self.scanned.last_start = start;
        }
        Some(())
    }
}

pub(super) struct DocumentGraphemes<'reader, 'document> {
    reader: &'reader mut DocumentReader<'document>,
    cursor: usize,
    next_start: SourceOffset,
    loaded_end: SourceOffset,
}

impl DocumentGraphemes<'_, '_> {
    pub(super) fn next_grapheme(&mut self) -> Result<Option<SourceGrapheme<'_>>, LoadError> {
        if self.next_start == self.reader.source_end() {
            return Ok(None);
        }
        loop {
            self.ensure_data()?;
            let buffer = std::str::from_utf8(&self.reader.cache.grapheme)
                .expect("document caches contain validated UTF-8");
            let candidate = &buffer[self.cursor..];
            let grapheme_bytes = candidate
                .graphemes(true)
                .next()
                .expect("a nonempty source has a grapheme")
                .len();
            let complete =
                grapheme_bytes < candidate.len() || self.loaded_end == self.reader.source_end();

            if complete {
                return self.emit(grapheme_bytes, grapheme_bytes <= MAX_GRAPHEME_BYTES);
            }
            if candidate.len() >= MAX_GRAPHEME_BYTES {
                return self.emit(candidate.len(), false);
            }
            self.compact();
            self.append_window()?;
        }
    }

    fn ensure_data(&mut self) -> Result<(), LoadError> {
        if self.cursor < self.reader.cache.grapheme.len() {
            return Ok(());
        }
        self.reader.cache.grapheme.clear();
        self.cursor = 0;
        self.loaded_end = self.next_start;
        self.append_window()
    }

    fn compact(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let remaining = self.reader.cache.grapheme.len() - self.cursor;
        self.reader.cache.grapheme.copy_within(self.cursor.., 0);
        self.reader.cache.grapheme.truncate(remaining);
        self.cursor = 0;
    }

    fn append_window(&mut self) -> Result<(), LoadError> {
        let remaining_budget = MAX_GRAPHEME_BYTES
            .saturating_sub(self.reader.cache.grapheme.len())
            .max(1);
        let target = NonZeroUsize::new(self.reader.cache.window_bytes.get().min(remaining_budget))
            .expect("window targets are nonzero");
        let expected_start = self.loaded_end;
        let (start, end, length) = {
            let window = self.reader.window(expected_start, target)?;
            (window.start(), window.end(), window.len_bytes())
        };
        if start != expected_start || end <= start {
            return Err(self.reader.protocol_error("non-contiguous document window"));
        }
        self.reader
            .cache
            .grapheme
            .try_reserve_exact(length)
            .map_err(|_| LoadError::Allocation("grapheme buffer"))?;
        self.reader
            .cache
            .grapheme
            .extend_from_slice(&self.reader.cache.chunk);
        self.loaded_end = end;
        Ok(())
    }

    fn emit(
        &mut self,
        length: usize,
        include_text: bool,
    ) -> Result<Option<SourceGrapheme<'_>>, LoadError> {
        let start = self.next_start;
        let end = start
            .checked_add(length)
            .ok_or_else(|| self.reader.protocol_error("grapheme coordinates overflow"))?;
        let text_start = self.cursor;
        self.cursor += length;
        self.next_start = end;
        let text = include_text.then(|| {
            let buffer = std::str::from_utf8(&self.reader.cache.grapheme)
                .expect("document caches contain validated UTF-8");
            &buffer[text_start..text_start + length]
        });
        Ok(Some(SourceGrapheme { start, end, text }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SourceGrapheme<'a> {
    start: SourceOffset,
    end: SourceOffset,
    text: Option<&'a str>,
}

impl<'a> SourceGrapheme<'a> {
    pub(super) const fn start(self) -> SourceOffset {
        self.start
    }

    pub(super) const fn end(self) -> SourceOffset {
        self.end
    }

    pub(super) const fn text(self) -> Option<&'a str> {
        self.text
    }
}

#[derive(Debug)]
enum DocumentStore {
    File(FileStore),
    #[cfg(test)]
    InMemory(InMemoryStore),
}

impl DocumentStore {
    #[cfg(test)]
    fn in_memory(source: String) -> Self {
        Self::InMemory(InMemoryStore::new(source))
    }

    const fn source_start(&self) -> SourceOffset {
        match self {
            Self::File(store) => store.source_start,
            #[cfg(test)]
            Self::InMemory(store) => store.source_start,
        }
    }

    fn source_end(&self) -> SourceOffset {
        match self {
            Self::File(store) => store.source_end,
            #[cfg(test)]
            Self::InMemory(store) => store.source().end(),
        }
    }

    #[cfg(test)]
    fn contiguous_source(&self) -> Option<SourceText<'_>> {
        match self {
            Self::File(_) => None,
            Self::InMemory(store) => Some(store.source()),
        }
    }

    fn copy_window<'a>(
        &self,
        request: WindowRequest,
        output: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        match self {
            Self::File(store) => store.copy_window(request, output),
            #[cfg(test)]
            Self::InMemory(store) => store.copy_window(request, output),
        }
    }

    fn copy_bytes(
        &self,
        start: SourceOffset,
        length: usize,
        output: &mut Vec<u8>,
    ) -> Result<(), LoadError> {
        match self {
            Self::File(store) => store.copy_bytes(start, length, output),
            #[cfg(test)]
            Self::InMemory(store) => store.copy_bytes(start, length, output),
        }
    }
}

#[cfg(test)]
#[derive(Debug)]
struct InMemoryStore {
    source: String,
    content_start: usize,
    source_start: SourceOffset,
}

#[cfg(test)]
impl InMemoryStore {
    fn new(source: String) -> Self {
        let content_start = if source.starts_with('\u{feff}') {
            UTF8_BOM_BYTES
        } else {
            0
        };
        Self {
            source,
            content_start,
            source_start: SourceOffset::from_usize(content_start),
        }
    }

    fn with_start(source: String, source_start: SourceOffset) -> Self {
        Self {
            source,
            content_start: 0,
            source_start,
        }
    }

    fn source(&self) -> SourceText<'_> {
        SourceText::with_start(&self.source[self.content_start..], self.source_start)
            .expect("an in-memory source span fits in u64 coordinates")
    }

    fn copy_window<'a>(
        &self,
        request: WindowRequest,
        output: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        let source = self
            .source()
            .window(request)
            .expect("document readers request valid forward windows");
        copy_source(source, output)
    }

    fn copy_bytes(
        &self,
        start: SourceOffset,
        length: usize,
        output: &mut Vec<u8>,
    ) -> Result<(), LoadError> {
        let source = self.source();
        let relative = source
            .relative_offset(start)
            .expect("document readers request in-range bytes");
        let end = relative
            .checked_add(length)
            .filter(|end| *end <= source.len_bytes())
            .expect("document readers request bounded bytes");
        output.clear();
        output
            .try_reserve_exact(length)
            .map_err(|_| LoadError::Allocation("source byte window"))?;
        output.extend_from_slice(&source.as_str().as_bytes()[relative..end]);
        Ok(())
    }
}

#[cfg(test)]
fn copy_source<'a>(
    source: SourceText<'_>,
    output: &'a mut Vec<u8>,
) -> Result<SourceText<'a>, LoadError> {
    output.clear();
    output
        .try_reserve_exact(source.len_bytes())
        .map_err(|_| LoadError::Allocation("source window"))?;
    output.extend_from_slice(source.as_str().as_bytes());
    let text = std::str::from_utf8(output).expect("copied source windows remain valid UTF-8");
    Ok(SourceText::with_start(text, source.start())
        .expect("copied source windows retain validated coordinates"))
}

pub(super) fn load(path: PathBuf) -> Result<Document, LoadError> {
    let store = FileStore::open(path, MAX_FILE_BYTES)?;
    let line_index = store.build_line_index(MAX_FILE_BYTES)?;
    let path = store.path.clone();
    Ok(Document::from_parts(
        &path,
        DocumentStore::File(store),
        line_index,
    ))
}

#[cfg(test)]
fn build_line_index(store: &DocumentStore, path: &Path) -> Result<LineIndex, LoadError> {
    let source_start = store.source_start();
    let source_end = store.source_end();
    let mut index = LineIndex::new(source_start, source_end)
        .map_err(|error| map_line_index_error(path, error))?;
    let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
    let mut scratch = Vec::new();
    let mut cursor = source_start;

    while cursor < source_end {
        let window = store.copy_window(WindowRequest::new(cursor, target), &mut scratch)?;
        index
            .extend(window)
            .map_err(|error| map_line_index_error(path, error))?;
        cursor = window.end();
    }
    index
        .finish()
        .map_err(|error| map_line_index_error(path, error))?;
    Ok(index)
}

fn map_line_index_error(path: &Path, error: LineIndexError) -> LoadError {
    match error {
        LineIndexError::Allocation => LoadError::Allocation("physical-line index"),
        error => LoadError::Read {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidData, error),
        },
    }
}

#[derive(Debug)]
struct FileStore {
    file: File,
    path: PathBuf,
    source_start: SourceOffset,
    source_end: SourceOffset,
    fingerprint: FileFingerprint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileFingerprint {
    device: u64,
    inode: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

impl FileFingerprint {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }
    }
}

impl FileStore {
    fn open(path: PathBuf, limit: usize) -> Result<Self, LoadError> {
        let descriptor = rustix::fs::open(
            &path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|source| LoadError::Open {
            path: path.clone(),
            source: io::Error::from(source),
        })?;

        let file = File::from(descriptor);
        let metadata = file.metadata().map_err(|source| LoadError::Read {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(LoadError::NotRegular(path));
        }
        if metadata.len() > limit as u64 {
            return Err(LoadError::TooLarge { path, limit });
        }

        let source_len = usize::try_from(metadata.len()).map_err(|_| LoadError::TooLarge {
            path: path.clone(),
            limit,
        })?;
        let mut store = Self {
            file,
            path,
            source_start: SourceOffset::ZERO,
            source_end: SourceOffset::from_usize(source_len),
            fingerprint: FileFingerprint::from_metadata(&metadata),
        };
        store.source_start = SourceOffset::from_usize(store.content_start(source_len)?);
        Ok(store)
    }

    fn build_line_index(&self, limit: usize) -> Result<LineIndex, LoadError> {
        let source_len = self.bounded_len(limit)?;
        if SourceOffset::from_usize(source_len) != self.source_end {
            return Err(self.changed());
        }
        self.require_unchanged()?;
        let mut line_index = LineIndex::new(self.source_start, self.source_end)
            .map_err(|error| map_line_index_error(&self.path, error))?;
        let mut window_cache = Vec::new();
        let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
        let mut cursor = SourceOffset::ZERO;

        while cursor < self.source_end {
            let window = self.window(
                WindowRequest::new(cursor, target),
                self.source_end,
                &mut window_cache,
            )?;
            let indexed_start = window.start().max(self.source_start);
            if indexed_start < window.end() {
                let relative = window
                    .relative_offset(indexed_start)
                    .expect("indexed source starts inside the file window");
                let indexed = SourceText::with_start(&window.as_str()[relative..], indexed_start)
                    .expect("validated file windows fit source coordinates");
                line_index
                    .extend(indexed)
                    .map_err(|error| map_line_index_error(&self.path, error))?;
            }
            cursor = window.end();
        }

        line_index
            .finish()
            .map_err(|error| map_line_index_error(&self.path, error))?;
        let final_len = self.bounded_len(limit)?;
        if final_len != source_len {
            return Err(self.changed());
        }
        self.require_unchanged()?;
        Ok(line_index)
    }

    fn copy_window<'a>(
        &self,
        request: WindowRequest,
        output: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        self.require_unchanged()?;
        if request.start() < self.source_start {
            return Err(self.invalid_window("file window starts before document content"));
        }
        let window = self.window(request, self.source_end, output)?;
        self.require_unchanged()?;
        Ok(window)
    }

    fn copy_bytes(
        &self,
        start: SourceOffset,
        length: usize,
        output: &mut Vec<u8>,
    ) -> Result<(), LoadError> {
        self.require_unchanged()?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| self.invalid_window("byte window coordinates overflow"))?;
        if start < self.source_start || end > self.source_end {
            return Err(self.invalid_window("byte window exceeds document content"));
        }
        output.clear();
        output
            .try_reserve_exact(length)
            .map_err(|_| LoadError::Allocation("source byte window"))?;
        output.resize(length, 0);
        self.read_exact_at(output, start)?;
        self.require_unchanged()
    }

    fn bounded_len(&self, limit: usize) -> Result<usize, LoadError> {
        let metadata = self.file.metadata().map_err(|source| LoadError::Read {
            path: self.path.clone(),
            source,
        })?;
        if metadata.len() > limit as u64 {
            return Err(LoadError::TooLarge {
                path: self.path.clone(),
                limit,
            });
        }
        usize::try_from(metadata.len()).map_err(|_| LoadError::TooLarge {
            path: self.path.clone(),
            limit,
        })
    }

    fn content_start(&self, source_len: usize) -> Result<usize, LoadError> {
        if source_len < UTF8_BOM_BYTES {
            return Ok(0);
        }
        let mut prefix = [0_u8; UTF8_BOM_BYTES];
        self.read_exact_at(&mut prefix, SourceOffset::ZERO)?;
        Ok(if prefix == [0xef, 0xbb, 0xbf] {
            UTF8_BOM_BYTES
        } else {
            0
        })
    }

    fn window<'a>(
        &self,
        request: WindowRequest,
        source_end: SourceOffset,
        cache: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        if request.start() >= source_end {
            return Err(self.invalid_window("file window starts outside the source"));
        }
        let remaining = usize::try_from(source_end.get() - request.start().get())
            .map_err(|_| self.invalid_window("file window length exceeds address space"))?;
        let target_len = request.target_bytes().min(remaining);
        let read_len = target_len
            .saturating_add(UTF8_BOUNDARY_SLOP_BYTES)
            .min(remaining);

        cache.clear();
        cache
            .try_reserve_exact(read_len)
            .map_err(|_| LoadError::Allocation("file window"))?;
        cache.resize(read_len, 0);
        self.read_exact_at(cache, request.start())?;

        let (text, window_len) = match std::str::from_utf8(cache) {
            Ok(text) => {
                let mut window_len = target_len;
                while window_len < text.len() && !text.is_char_boundary(window_len) {
                    window_len += 1;
                }
                (text, window_len)
            }
            Err(error)
                if error.error_len().is_none()
                    && read_len < remaining
                    && error.valid_up_to() >= target_len =>
            {
                let valid = std::str::from_utf8(&cache[..error.valid_up_to()])
                    .expect("UTF-8 errors identify a valid prefix");
                (valid, valid.len())
            }
            Err(error) => {
                let offset = usize::try_from(request.start().get())
                    .expect("bounded file offsets fit usize")
                    + error.valid_up_to();
                return Err(LoadError::InvalidUtf8 {
                    path: self.path.clone(),
                    offset,
                });
            }
        };

        SourceText::with_start(&text[..window_len], request.start())
            .ok_or_else(|| self.invalid_window("file window coordinates overflow"))
    }

    fn read_exact_at(
        &self,
        mut output: &mut [u8],
        mut offset: SourceOffset,
    ) -> Result<(), LoadError> {
        while !output.is_empty() {
            let count = loop {
                match self.file.read_at(output, offset.get()) {
                    Ok(count) => break count,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(source) => {
                        return Err(LoadError::Read {
                            path: self.path.clone(),
                            source,
                        });
                    }
                }
            };
            if count == 0 {
                return Err(LoadError::Read {
                    path: self.path.clone(),
                    source: io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "file changed while reading",
                    ),
                });
            }
            offset = offset.checked_add(count).ok_or_else(|| {
                self.invalid_window("file window coordinates overflow while reading")
            })?;
            output = &mut output[count..];
        }
        Ok(())
    }

    fn invalid_window(&self, message: &'static str) -> LoadError {
        LoadError::Read {
            path: self.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidInput, message),
        }
    }

    fn changed(&self) -> LoadError {
        LoadError::Read {
            path: self.path.clone(),
            source: io::Error::new(io::ErrorKind::InvalidData, "file changed while reading"),
        }
    }

    fn require_unchanged(&self) -> Result<(), LoadError> {
        let metadata = self.file.metadata().map_err(|source| LoadError::Read {
            path: self.path.clone(),
            source,
        })?;
        if FileFingerprint::from_metadata(&metadata) != self.fingerprint {
            return Err(self.changed());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;

    fn read_all(document: &Document) -> String {
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut cursor = reader.source_start();
        let mut output = String::new();
        let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
        while cursor < reader.source_end() {
            let window = reader.window(cursor, target).unwrap();
            output.push_str(window.as_str());
            cursor = window.end();
        }
        output
    }

    #[test]
    fn loads_symlinks_and_preserves_source_coordinates() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let link = directory.path().join("link.txt");
        fs::write(&source, b"\xef\xbb\xbfone\r\ntwo\rthree\n\xef\xbb\xbf").unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let document = load(link).unwrap();
        assert_eq!(read_all(&document), "one\r\ntwo\rthree\n\u{feff}");
        assert_eq!(document.source_start(), SourceOffset::new(3));
        assert_eq!(document.source_end(), SourceOffset::new(21));
        let mut cache = DocumentCache::default();
        let position = document
            .reader(&mut cache)
            .line_position(SourceOffset::new(18))
            .unwrap();
        assert_eq!((position.current(), position.total()), (4, 4));
        assert_eq!(document.display_name(), "link.txt");
    }

    #[test]
    fn rejects_invalid_utf8_at_the_original_byte_offset() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("invalid.txt");
        fs::write(&path, b"ok\xffbad").unwrap();
        assert!(matches!(
            load(path),
            Err(LoadError::InvalidUtf8 { offset: 2, .. })
        ));
    }

    #[test]
    fn bounded_read_detects_growth_beyond_metadata() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("growing.txt");
        fs::write(&path, b"1").unwrap();
        let store = FileStore::open(path, 4).unwrap();
        fs::write(&store.path, b"12345").unwrap();
        let error = store.build_line_index(4).unwrap_err();
        assert!(matches!(error, LoadError::TooLarge { limit: 4, .. }));
    }

    #[test]
    fn reads_reject_changes_to_the_open_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing.txt");
        fs::write(&path, "stable").unwrap();
        let document = load(path.clone()).unwrap();
        fs::write(path, "change").unwrap();

        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let error = reader
            .window(
                reader.source_start(),
                NonZeroUsize::new(1).expect("one is nonzero"),
            )
            .unwrap_err();
        assert!(
            matches!(error, LoadError::Read { source, .. } if source.kind() == io::ErrorKind::InvalidData)
        );
    }

    #[test]
    fn file_windows_extend_to_utf8_boundaries_with_bounded_slop() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("unicode.txt");
        fs::write(&path, "🙂z").unwrap();
        let store = FileStore::open(path, 16).unwrap();
        let end = SourceOffset::new(5);
        let target = NonZeroUsize::new(1).unwrap();
        let mut cache = Vec::new();

        let first = store
            .window(
                WindowRequest::new(SourceOffset::ZERO, target),
                end,
                &mut cache,
            )
            .unwrap();
        assert_eq!(first.as_str(), "🙂");
        assert_eq!(first.end(), SourceOffset::new(4));
        assert_eq!(cache.len(), 4);

        let second = store
            .window(
                WindowRequest::new(SourceOffset::new(4), target),
                end,
                &mut cache,
            )
            .unwrap();
        assert_eq!(second.as_str(), "z");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn reports_invalid_utf8_when_a_sequence_crosses_a_window_boundary() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("invalid-boundary.txt");
        let mut bytes = vec![b'a'; SOURCE_WINDOW_BYTES - 1];
        bytes.extend_from_slice(&[0xf0, 0x9f, b'x']);
        fs::write(&path, bytes).unwrap();

        assert!(matches!(
            load(path),
            Err(LoadError::InvalidUtf8 { offset, .. })
                if offset == SOURCE_WINDOW_BYTES - 1
        ));
    }

    #[test]
    fn loads_utf8_scalars_that_cross_file_windows() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("valid-boundary.txt");
        let mut text = "a".repeat(SOURCE_WINDOW_BYTES - 1);
        text.push_str("🙂z");
        fs::write(&path, &text).unwrap();

        let document = load(path).unwrap();
        assert_eq!(read_all(&document), text);
    }

    #[test]
    fn reports_incomplete_utf8_at_end_of_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("incomplete.txt");
        let mut bytes = vec![b'a'; SOURCE_WINDOW_BYTES - 1];
        bytes.extend_from_slice(&[0xf0, 0x9f]);
        fs::write(&path, bytes).unwrap();

        assert!(matches!(
            load(path),
            Err(LoadError::InvalidUtf8 { offset, .. })
                if offset == SOURCE_WINDOW_BYTES - 1
        ));
    }

    #[test]
    fn rejects_directories() {
        let directory = tempdir().unwrap();
        assert!(matches!(
            FileStore::open(directory.path().to_path_buf(), 16),
            Err(LoadError::NotRegular(_))
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_fifos_without_blocking() {
        use rustix::fs::{CWD, Mode, mkfifoat};

        let directory = tempdir().unwrap();
        let fifo = directory.path().join("input.fifo");
        mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
        assert!(matches!(
            FileStore::open(fifo, 16),
            Err(LoadError::NotRegular(_))
        ));
    }

    #[test]
    fn test_constructor_preserves_display_metadata() {
        let path = PathBuf::from("/tmp/book.txt");
        let document = Document::from_text(&path, "text".to_owned());
        assert_eq!(document.display_path(), "/tmp/book.txt");
        assert_eq!(document.source().as_str(), "text");
    }

    #[test]
    fn line_index_handles_crlf_across_store_windows() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("book.txt");
        let mut text = "a".repeat(SOURCE_WINDOW_BYTES - 1);
        text.push_str("\r\nb");
        fs::write(&path, text).unwrap();
        let document = load(path).unwrap();
        let mut cache = DocumentCache::default();
        let position = document
            .reader(&mut cache)
            .line_position(SourceOffset::from_usize(SOURCE_WINDOW_BYTES + 1))
            .unwrap();

        assert_eq!((position.current(), position.total()), (2, 2));
    }

    #[test]
    fn file_backed_line_scans_match_contiguous_coordinates() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("lines.txt");
        let text = "\u{feff}a\r\né\rb\nc";
        fs::write(&path, text).unwrap();
        let document = load(path).unwrap();
        let expected = Document::from_text(Path::new("expected.txt"), text.to_owned());
        let source = expected.source();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);

        for relative in source
            .as_str()
            .char_indices()
            .map(|(relative, _)| relative)
            .chain(std::iter::once(source.len_bytes()))
        {
            let offset = source.start().checked_add(relative).unwrap();
            assert_eq!(
                reader.line_position(offset).unwrap(),
                expected.line_index.position(source, offset).unwrap()
            );
            let previous = source.as_str()[..relative]
                .char_indices()
                .next_back()
                .map(|(previous, _)| source.start().checked_add(previous).unwrap());
            assert_eq!(reader.previous_char_start(offset).unwrap(), previous);
        }
    }

    #[test]
    fn incremental_graphemes_match_contiguous_segmentation_at_every_small_window_size() {
        let text = "\u{feff}a\r\ne\u{301}🇷🇸🇮🇴👩\u{200d}🔬क्\u{200d}ष\u{200b}z";
        let document = Document::from_text(Path::new("unicode.txt"), text.to_owned());
        let source = document.source();
        let expected: Vec<_> = source
            .as_str()
            .grapheme_indices(true)
            .map(|(relative, grapheme)| {
                (
                    source.start().checked_add(relative).unwrap(),
                    grapheme.to_owned(),
                )
            })
            .collect();

        for window_bytes in 1..=8 {
            let mut cache = DocumentCache::with_window_bytes(window_bytes);
            let mut reader = document.reader(&mut cache);
            let mut graphemes = reader.graphemes(source.start()).unwrap();
            let mut actual = Vec::new();
            while let Some(grapheme) = graphemes.next_grapheme().unwrap() {
                actual.push((
                    grapheme.start(),
                    grapheme
                        .text()
                        .expect("test graphemes fit the limit")
                        .to_owned(),
                ));
            }
            assert_eq!(actual, expected, "window size {window_bytes}");
        }
    }

    #[test]
    fn oversized_graphemes_are_bounded_and_preserve_following_coordinates() {
        let combining_count = MAX_GRAPHEME_BYTES / '\u{301}'.len_utf8() + 1;
        let mut text = String::with_capacity(1 + combining_count * '\u{301}'.len_utf8() + 1);
        text.push('a');
        text.extend(std::iter::repeat_n('\u{301}', combining_count));
        text.push('z');

        let document = Document::from_text(Path::new("oversized.txt"), text);
        let source_end = document.source().end();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
        let first = graphemes.next_grapheme().unwrap().unwrap();

        assert_eq!(first.start(), SourceOffset::ZERO);
        assert!(first.text().is_none());
        assert!(first.end().get() <= (MAX_GRAPHEME_BYTES + UTF8_BOUNDARY_SLOP_BYTES) as u64);

        let mut next_start = first.end();
        let mut saw_final = false;
        while let Some(grapheme) = graphemes.next_grapheme().unwrap() {
            assert_eq!(grapheme.start(), next_start);
            saw_final |= grapheme.text() == Some("z");
            next_start = grapheme.end();
        }
        assert!(saw_final);
        assert_eq!(next_start, source_end);
    }
}
