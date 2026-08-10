use std::{
    fs::File,
    io::{self, Read, Write},
    num::{NonZeroU64, NonZeroUsize},
    os::unix::fs::{FileExt, MetadataExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
use std::cell::Cell;

use rustix::fs::{Mode, OFlags};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::{
    error::{LoadError, sanitize_os},
    line_index::{LineIndex, LineIndexError, LinePosition, LineScan},
    path_binding::{BoundPath, PathIdentity},
    source::{BackwardWindowRequest, SourceOffset, SourceText, WindowRequest},
};

pub const MAX_FILE_BYTES: u64 = 33_554_432;
pub(super) const SOURCE_WINDOW_BYTES: usize = 64 * 1024;
const STANDARD_INPUT_NAME: &str = "standard input";
const UTF8_BOM_BYTES: usize = 3;
const UTF8_BOUNDARY_SLOP_BYTES: usize = 3;
const MAX_GRAPHEME_BYTES: usize = 1024 * 1024;
static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub(super) struct DocumentId(NonZeroU64);

impl DocumentId {
    fn next() -> Self {
        Self::try_next(&NEXT_DOCUMENT_ID).expect("document identity space is exhausted")
    }

    fn try_next(counter: &AtomicU64) -> Option<Self> {
        let value = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .ok()?;
        NonZeroU64::new(value).map(Self)
    }

    #[cfg(test)]
    const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug)]
pub(super) struct Document {
    id: DocumentId,
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

    pub(super) fn validate(&self) -> Result<(), LoadError> {
        self.store.validate()
    }

    #[cfg(test)]
    pub(super) fn stability_checks(&self) -> usize {
        match &self.store {
            DocumentStore::File(store) => store.stability_checks(),
            DocumentStore::InMemory(_) => 0,
        }
    }

    pub(super) fn input_identity(&self) -> Option<InputIdentity<'_>> {
        self.store.input_identity()
    }

    pub(super) const fn content_len(&self) -> u64 {
        self.source_end.get() - self.source_start.get()
    }

    pub(super) fn reader<'a>(&'a self, cache: &'a mut DocumentCache) -> DocumentReader<'a> {
        DocumentReader {
            document: self,
            cache,
            validated: false,
        }
    }

    pub(super) const fn line_index_complete(&self) -> bool {
        self.line_index.is_complete()
    }

    pub(super) fn line_index_covers(&self, offset: SourceOffset) -> bool {
        self.line_index.covers(offset)
    }

    pub(super) fn advance_line_index(
        &mut self,
        cache: &mut DocumentCache,
    ) -> Result<bool, LoadError> {
        if self.line_index.is_complete() {
            return Ok(false);
        }
        let cursor = self.line_index.scanned_to();
        if cursor < self.source_end {
            let target =
                NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
            let window = self
                .store
                .copy_window(WindowRequest::new(cursor, target), &mut cache.chunk)?;
            self.line_index
                .extend(window)
                .map_err(|error| map_line_index_error(&self.path, error))?;
        }
        if self.line_index.scanned_to() == self.source_end {
            self.line_index
                .finish()
                .map_err(|error| map_line_index_error(&self.path, error))?;
        }
        Ok(true)
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
        Self::assemble(
            path.to_path_buf(),
            sanitize_os(path.as_os_str()),
            sanitize_os(path.file_name().unwrap_or(path.as_os_str())),
            store,
            line_index,
        )
    }

    fn from_standard_input(store: DocumentStore, line_index: LineIndex) -> Self {
        Self::assemble(
            PathBuf::from(STANDARD_INPUT_NAME),
            STANDARD_INPUT_NAME.to_owned(),
            STANDARD_INPUT_NAME.to_owned(),
            store,
            line_index,
        )
    }

    fn assemble(
        path: PathBuf,
        display_path: String,
        display_name: String,
        store: DocumentStore,
        line_index: LineIndex,
    ) -> Self {
        let source_start = store.source_start();
        let source_end = store.source_end();
        Self {
            id: DocumentId::next(),
            store,
            line_index,
            source_start,
            source_end,
            path,
            display_path,
            display_name,
        }
    }
}

#[derive(Debug)]
pub(super) struct DocumentCache {
    chunk: Vec<u8>,
    grapheme: String,
    grapheme_start: SourceOffset,
    grapheme_end: SourceOffset,
    grapheme_document_id: Option<DocumentId>,
    window_bytes: NonZeroUsize,
    #[cfg(test)]
    metrics: DocumentMetrics,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct DocumentMetrics {
    window_calls: usize,
    byte_window_calls: usize,
    grapheme_emissions: usize,
    segmentation_runs: usize,
    segmentation_advanced_bytes: usize,
    grapheme_window_calls: usize,
    grapheme_window_returned_bytes: usize,
    grapheme_utf8_validated_bytes: usize,
}

#[cfg(test)]
impl DocumentMetrics {
    pub(super) const fn window_calls(self) -> usize {
        self.window_calls
    }

    pub(super) const fn byte_window_calls(self) -> usize {
        self.byte_window_calls
    }

    pub(super) const fn grapheme_emissions(self) -> usize {
        self.grapheme_emissions
    }

    pub(super) const fn segmentation_runs(self) -> usize {
        self.segmentation_runs
    }

    pub(super) const fn segmentation_advanced_bytes(self) -> usize {
        self.segmentation_advanced_bytes
    }

    pub(super) const fn grapheme_window_calls(self) -> usize {
        self.grapheme_window_calls
    }

    pub(super) const fn grapheme_window_returned_bytes(self) -> usize {
        self.grapheme_window_returned_bytes
    }

    pub(super) const fn grapheme_utf8_validated_bytes(self) -> usize {
        self.grapheme_utf8_validated_bytes
    }
}

impl Default for DocumentCache {
    fn default() -> Self {
        Self {
            chunk: Vec::new(),
            grapheme: String::new(),
            grapheme_start: SourceOffset::ZERO,
            grapheme_end: SourceOffset::ZERO,
            grapheme_document_id: None,
            window_bytes: NonZeroUsize::new(SOURCE_WINDOW_BYTES)
                .expect("source window size is nonzero"),
            #[cfg(test)]
            metrics: DocumentMetrics::default(),
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

    #[cfg(test)]
    pub(super) fn reset_metrics(&mut self) {
        self.metrics = DocumentMetrics::default();
    }

    #[cfg(test)]
    pub(super) const fn metrics(&self) -> DocumentMetrics {
        self.metrics
    }
}

pub(super) struct DocumentReader<'a> {
    document: &'a Document,
    cache: &'a mut DocumentCache,
    validated: bool,
}

impl<'document> DocumentReader<'document> {
    pub(super) fn graphemes<'reader>(
        &'reader mut self,
        start: SourceOffset,
    ) -> Result<DocumentGraphemes<'reader, 'document>, LoadError> {
        self.validate_once()?;
        let document_id = self.document.id;
        if self.cache.grapheme_document_id == Some(document_id)
            && start >= self.cache.grapheme_start
            && start <= self.cache.grapheme_end
        {
            let cursor = usize::try_from(start.get() - self.cache.grapheme_start.get())
                .expect("grapheme caches fit the process address space");
            if !self.cache.grapheme.is_char_boundary(cursor) {
                return Err(self.protocol_error("invalid grapheme cursor offset"));
            }
            debug_assert_eq!(
                self.cache.grapheme_end.get() - self.cache.grapheme_start.get(),
                self.cache.grapheme.len() as u64
            );
            let loaded_end = self.cache.grapheme_end;
            return Ok(DocumentGraphemes {
                reader: self,
                cursor,
                next_start: start,
                loaded_end,
            });
        }

        self.require_char_boundary(start, "invalid grapheme cursor offset")?;
        self.cache.grapheme.clear();
        self.cache.grapheme_start = start;
        self.cache.grapheme_end = start;
        self.cache.grapheme_document_id = Some(document_id);
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
        #[cfg(test)]
        {
            self.cache.metrics.window_calls += 1;
        }
        self.document.store.copy_window(
            WindowRequest::new(start, target_bytes),
            &mut self.cache.chunk,
        )
    }

    pub(super) fn window_ending_at(
        &mut self,
        end: SourceOffset,
        target_bytes: NonZeroUsize,
    ) -> Result<SourceText<'_>, LoadError> {
        self.document.store.copy_window_ending_at(
            BackwardWindowRequest::new(end, target_bytes),
            &mut self.cache.chunk,
        )
    }

    pub(super) fn source_start(&self) -> SourceOffset {
        self.document.source_start()
    }

    pub(super) const fn document_id(&self) -> DocumentId {
        self.document.id
    }

    pub(super) fn source_end(&self) -> SourceOffset {
        self.document.source_end()
    }

    pub(super) fn validate(&mut self) -> Result<(), LoadError> {
        self.validate_once()
    }

    pub(super) fn line_position(
        &mut self,
        offset: SourceOffset,
    ) -> Result<Option<LinePosition>, LoadError> {
        let Some((scan, lines)) = self.scan_lines(offset)? else {
            return Ok(None);
        };
        scan.finish(lines.count)
            .map(Some)
            .ok_or_else(|| self.protocol_error("physical-line coordinates overflow"))
    }

    pub(super) fn line_start_at_or_before(
        &mut self,
        offset: SourceOffset,
    ) -> Result<SourceOffset, LoadError> {
        match self.scan_lines(offset)? {
            Some((_, lines)) => Ok(lines.last_start),
            None => self.find_line_start_backward(offset),
        }
    }

    pub(super) fn previous_char_start(
        &mut self,
        offset: SourceOffset,
    ) -> Result<Option<SourceOffset>, LoadError> {
        self.require_char_boundary(offset, "invalid character offset")?;
        if offset == self.source_start() {
            return Ok(None);
        }
        let one = NonZeroUsize::new(1).expect("one is nonzero");
        Ok(Some(self.window_ending_at(offset, one)?.start()))
    }

    fn scan_lines(
        &mut self,
        offset: SourceOffset,
    ) -> Result<Option<(LineScan, ScannedLines)>, LoadError> {
        self.require_char_boundary(offset, "invalid physical-line offset")?;
        let Some(scan) = self.document.line_index.scan_from(offset) else {
            return Ok(None);
        };
        let scan_end = if offset < self.source_end() {
            offset
                .checked_add(1)
                .ok_or_else(|| self.protocol_error("physical-line coordinates overflow"))?
        } else {
            offset
        };
        let mut scanner = LineScanner::new(
            offset,
            scan.line_start(),
            scan.pending_cr().then_some(scan.start()),
        );
        let mut cursor = scan.start();

        while cursor < scan_end {
            let remaining = scan_end.get() - cursor.get();
            let target = NonZeroUsize::new(
                usize::try_from(remaining.min(SOURCE_WINDOW_BYTES as u64))
                    .expect("bounded line scan lengths fit the process address space"),
            )
            .expect("nonempty line scans request nonzero windows");
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
        Ok(Some((scan, scanner.scanned())))
    }

    fn find_line_start_backward(
        &mut self,
        offset: SourceOffset,
    ) -> Result<SourceOffset, LoadError> {
        if offset == self.source_start() {
            return Ok(offset);
        }
        let cr_joins_lf = if offset < self.source_end() {
            #[cfg(test)]
            {
                self.cache.metrics.byte_window_calls += 1;
            }
            self.document
                .store
                .copy_bytes(offset, 1, &mut self.cache.chunk)?;
            self.cache.chunk[0] == b'\n'
        } else {
            false
        };
        let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
        let mut cursor = offset;

        while cursor > self.source_start() {
            let (window_start, found) = {
                let window = self.window_ending_at(cursor, target)?;
                let bytes = window.as_str().as_bytes();
                let found = bytes.iter().enumerate().rev().find_map(|(relative, byte)| {
                    let is_ignored_cr = cr_joins_lf
                        && window.end() == offset
                        && relative + 1 == bytes.len()
                        && *byte == b'\r';
                    ((*byte == b'\n' || *byte == b'\r') && !is_ignored_cr).then_some(relative)
                });
                (window.start(), found)
            };
            if let Some(relative) = found {
                return window_start
                    .checked_add(relative + 1)
                    .ok_or_else(|| self.protocol_error("physical-line coordinates overflow"));
            }
            if window_start >= cursor {
                return Err(self.protocol_error("non-contiguous backward line window"));
            }
            cursor = window_start;
        }
        Ok(self.source_start())
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
        #[cfg(test)]
        {
            self.cache.metrics.byte_window_calls += 1;
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

    fn validate_once(&mut self) -> Result<(), LoadError> {
        if !self.validated {
            self.document.store.validate()?;
            self.validated = true;
        }
        Ok(())
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
    const fn new(
        through: SourceOffset,
        line_start: SourceOffset,
        pending_cr: Option<SourceOffset>,
    ) -> Self {
        Self {
            through,
            scanned: ScannedLines {
                count: 0,
                last_start: line_start,
            },
            pending_cr,
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
        let remaining = usize::try_from(self.reader.source_end().get() - self.next_start.get())
            .map_err(|_| {
                self.reader
                    .protocol_error("grapheme length exceeds the address space")
            })?;
        let mut segmenter = GraphemeCursor::new(0, remaining, true);
        #[cfg(test)]
        let mut advanced = 0;
        loop {
            self.ensure_data()?;
            let (candidate_len, boundary) = {
                let candidate = &self.reader.cache.grapheme[self.cursor..];
                #[cfg(test)]
                {
                    self.reader.cache.metrics.segmentation_runs += 1;
                }
                (candidate.len(), segmenter.next_boundary(candidate, 0))
            };
            #[cfg(test)]
            {
                let cursor = segmenter.cur_cursor();
                self.reader.cache.metrics.segmentation_advanced_bytes += cursor - advanced;
                advanced = cursor;
            }
            match boundary {
                Ok(Some(grapheme_bytes)) => {
                    return if grapheme_bytes <= MAX_GRAPHEME_BYTES {
                        self.emit(grapheme_bytes, true)
                    } else {
                        self.emit(self.bounded_grapheme_length(), false)
                    };
                }
                Err(GraphemeIncomplete::NextChunk) if candidate_len >= MAX_GRAPHEME_BYTES => {
                    let bounded = self.bounded_grapheme_length();
                    if candidate_len > bounded {
                        return self.emit(bounded, false);
                    }
                }
                Err(GraphemeIncomplete::NextChunk) => {}
                Ok(None)
                | Err(
                    GraphemeIncomplete::PreContext(_)
                    | GraphemeIncomplete::PrevChunk
                    | GraphemeIncomplete::InvalidOffset,
                ) => {
                    return Err(self
                        .reader
                        .protocol_error("invalid incremental grapheme state"));
                }
            }
            self.compact();
            self.append_window()?;
        }
    }

    fn bounded_grapheme_length(&self) -> usize {
        let candidate = &self.reader.cache.grapheme[self.cursor..];
        let mut length = MAX_GRAPHEME_BYTES;
        while !candidate.is_char_boundary(length) {
            length -= 1;
        }
        length
    }

    fn ensure_data(&mut self) -> Result<(), LoadError> {
        if self.cursor < self.reader.cache.grapheme.len() {
            return Ok(());
        }
        self.reader.cache.grapheme.clear();
        self.cursor = 0;
        self.loaded_end = self.next_start;
        self.reader.cache.grapheme_start = self.next_start;
        self.reader.cache.grapheme_end = self.next_start;
        self.append_window()
    }

    fn compact(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.reader.cache.grapheme.replace_range(..self.cursor, "");
        self.reader.cache.grapheme_start = self.next_start;
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
        #[cfg(test)]
        {
            self.reader.cache.metrics.grapheme_window_calls += 1;
            self.reader.cache.metrics.grapheme_window_returned_bytes += length;
        }
        if start != expected_start || end <= start {
            return Err(self.reader.protocol_error("non-contiguous document window"));
        }
        let chunk = std::str::from_utf8(&self.reader.cache.chunk)
            .expect("document windows contain validated UTF-8");
        #[cfg(test)]
        {
            self.reader.cache.metrics.grapheme_utf8_validated_bytes += chunk.len();
        }
        let cache = &mut self.reader.cache.grapheme;
        if cache.len().saturating_add(length) >= MAX_GRAPHEME_BYTES {
            cache
                .try_reserve_exact(length)
                .map_err(|_| LoadError::Allocation("grapheme buffer"))?;
        } else {
            cache
                .try_reserve(length)
                .map_err(|_| LoadError::Allocation("grapheme buffer"))?;
        }
        cache.push_str(chunk);
        self.loaded_end = end;
        self.reader.cache.grapheme_end = end;
        Ok(())
    }

    fn emit(
        &mut self,
        length: usize,
        include_text: bool,
    ) -> Result<Option<SourceGrapheme<'_>>, LoadError> {
        #[cfg(test)]
        {
            self.reader.cache.metrics.grapheme_emissions += 1;
        }
        let start = self.next_start;
        let end = start
            .checked_add(length)
            .ok_or_else(|| self.reader.protocol_error("grapheme coordinates overflow"))?;
        let text_start = self.cursor;
        self.cursor += length;
        self.next_start = end;
        let text =
            include_text.then(|| &self.reader.cache.grapheme[text_start..text_start + length]);
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

    fn validate(&self) -> Result<(), LoadError> {
        match self {
            Self::File(store) => store.require_unchanged(),
            #[cfg(test)]
            Self::InMemory(_) => Ok(()),
        }
    }

    fn input_identity(&self) -> Option<InputIdentity<'_>> {
        match self {
            Self::File(store) => store.input_identity(),
            #[cfg(test)]
            Self::InMemory(_) => None,
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

    fn copy_window_ending_at<'a>(
        &self,
        request: BackwardWindowRequest,
        output: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        match self {
            Self::File(store) => store.copy_window_ending_at(request, output),
            #[cfg(test)]
            Self::InMemory(store) => store.copy_window_ending_at(request, output),
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

    fn copy_window_ending_at<'a>(
        &self,
        request: BackwardWindowRequest,
        output: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        let source = self
            .source()
            .window_ending_at(request)
            .expect("document readers request valid backward windows");
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

pub(super) fn load_standard_input(reader: &mut impl Read) -> Result<Document, LoadError> {
    load_standard_input_with_limit(reader, MAX_FILE_BYTES)
}

fn load_standard_input_with_limit(
    reader: &mut impl Read,
    limit: u64,
) -> Result<Document, LoadError> {
    let mut file =
        tempfile::tempfile().map_err(|source| LoadError::BufferStandardInput { source })?;
    let mut buffer = [0_u8; SOURCE_WINDOW_BYTES];
    let mut validator = Utf8StreamValidator::default();
    let mut total = 0_u64;

    loop {
        let probe = limit.saturating_sub(total).saturating_add(1);
        let request = usize::try_from(probe.min(SOURCE_WINDOW_BYTES as u64))
            .expect("bounded standard-input reads fit usize");
        let count = match reader.read(&mut buffer[..request]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(source) if source.kind() == io::ErrorKind::Interrupted => continue,
            Err(source) => return Err(LoadError::ReadStandardInput { source }),
        };
        let next = total
            .checked_add(u64::try_from(count).expect("source windows fit u64"))
            .filter(|next| *next <= limit)
            .ok_or(LoadError::StandardInputTooLarge { limit })?;
        validator.advance(&buffer[..count], total);
        if validator.invalid_offset.is_none() {
            file.write_all(&buffer[..count])
                .map_err(|source| LoadError::BufferStandardInput { source })?;
        }
        total = next;
    }

    if let Some(offset) = validator.finish() {
        return Err(LoadError::InvalidStandardInputUtf8 { offset });
    }

    let path = PathBuf::from(STANDARD_INPUT_NAME);
    let store = FileStore::from_private_snapshot(file, path.clone(), total)?;
    let line_index = LineIndex::new(store.source_start, store.source_end)
        .map_err(|error| map_line_index_error(&path, error))?;
    Ok(Document::from_standard_input(
        DocumentStore::File(store),
        line_index,
    ))
}

#[derive(Debug, Default)]
struct Utf8StreamValidator {
    tail: [u8; 4],
    tail_len: usize,
    tail_start: u64,
    invalid_offset: Option<u64>,
}

impl Utf8StreamValidator {
    fn advance(&mut self, bytes: &[u8], start: u64) {
        if self.invalid_offset.is_some() {
            return;
        }

        let mut consumed = 0;
        while self.tail_len != 0 && consumed < bytes.len() {
            self.tail[self.tail_len] = bytes[consumed];
            self.tail_len += 1;
            consumed += 1;
            match std::str::from_utf8(&self.tail[..self.tail_len]) {
                Ok(_) => self.tail_len = 0,
                Err(error) if error.error_len().is_some() => {
                    self.invalid_offset = Some(
                        self.tail_start
                            + u64::try_from(error.valid_up_to()).expect("UTF-8 tails fit u64"),
                    );
                    return;
                }
                Err(_) => {}
            }
        }
        if self.tail_len != 0 {
            return;
        }

        let remaining = &bytes[consumed..];
        let Err(error) = std::str::from_utf8(remaining) else {
            return;
        };
        let valid = error.valid_up_to();
        let invalid_start =
            start + u64::try_from(consumed + valid).expect("source windows fit u64");
        if error.error_len().is_some() {
            self.invalid_offset = Some(invalid_start);
            return;
        }

        let incomplete = &remaining[valid..];
        debug_assert!(incomplete.len() < self.tail.len());
        self.tail[..incomplete.len()].copy_from_slice(incomplete);
        self.tail_len = incomplete.len();
        self.tail_start = invalid_start;
    }

    const fn finish(&self) -> Option<u64> {
        match (self.invalid_offset, self.tail_len) {
            (Some(offset), _) => Some(offset),
            (None, 0) => None,
            (None, _) => Some(self.tail_start),
        }
    }
}

pub(super) fn load(path: PathBuf) -> Result<Document, LoadError> {
    let store = FileStore::open(path, MAX_FILE_BYTES)?;
    let path = store.path.clone();
    let line_index = LineIndex::new(store.source_start, store.source_end)
        .map_err(|error| map_line_index_error(&path, error))?;
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
    path_identity: Option<PathIdentity>,
    source_start: SourceOffset,
    source_end: SourceOffset,
    stability: FileStability,
    #[cfg(test)]
    stability_checks: Cell<usize>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct InputIdentity<'a> {
    pathname: &'a PathIdentity,
    file: FileIdentity,
}

impl InputIdentity<'_> {
    pub(super) fn pathname_matches(self, pathname: &PathIdentity) -> bool {
        self.pathname == pathname
    }

    pub(super) fn file_matches(self, metadata: &std::fs::Metadata) -> bool {
        self.file == FileIdentity::from_metadata(metadata)
    }
}

impl FileIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileStability {
    Tracked(FileFingerprint),
    PrivateSnapshot,
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

    const fn identity(self) -> FileIdentity {
        FileIdentity {
            device: self.device,
            inode: self.inode,
        }
    }
}

impl FileStore {
    fn open(path: PathBuf, limit: u64) -> Result<Self, LoadError> {
        let bound = BoundPath::capture(&path).map_err(|source| LoadError::Open {
            path: path.clone(),
            source,
        })?;
        let descriptor = rustix::fs::open(
            bound.open_path(),
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|source| LoadError::Open {
            path: path.clone(),
            source: io::Error::from(source),
        })?;

        Self::from_tracked_file_with_identity(
            File::from(descriptor),
            path,
            bound.into_identity(),
            limit,
        )
    }

    fn from_tracked_file_with_identity(
        file: File,
        path: PathBuf,
        path_identity: PathIdentity,
        limit: u64,
    ) -> Result<Self, LoadError> {
        let metadata = file.metadata().map_err(|source| LoadError::Read {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(LoadError::NotRegular(path));
        }
        if metadata.len() > limit {
            return Err(LoadError::TooLarge { path, limit });
        }

        Self::new(
            file,
            path,
            metadata.len(),
            FileStability::Tracked(FileFingerprint::from_metadata(&metadata)),
            Some(path_identity),
        )
    }

    fn from_private_snapshot(
        file: File,
        path: PathBuf,
        source_len: u64,
    ) -> Result<Self, LoadError> {
        Self::new(file, path, source_len, FileStability::PrivateSnapshot, None)
    }

    fn input_identity(&self) -> Option<InputIdentity<'_>> {
        let pathname = self.path_identity.as_ref()?;
        let FileStability::Tracked(fingerprint) = self.stability else {
            return None;
        };
        Some(InputIdentity {
            pathname,
            file: fingerprint.identity(),
        })
    }

    fn new(
        file: File,
        path: PathBuf,
        source_len: u64,
        stability: FileStability,
        path_identity: Option<PathIdentity>,
    ) -> Result<Self, LoadError> {
        debug_assert_eq!(
            path_identity.is_some(),
            matches!(stability, FileStability::Tracked(_))
        );
        let mut store = Self {
            file,
            path,
            path_identity,
            source_start: SourceOffset::ZERO,
            source_end: SourceOffset::new(source_len),
            stability,
            #[cfg(test)]
            stability_checks: Cell::new(0),
        };
        store.source_start = SourceOffset::new(store.content_start(source_len)?);
        Ok(store)
    }

    #[cfg(test)]
    fn build_line_index(&self, limit: u64) -> Result<LineIndex, LoadError> {
        let source_len = self.bounded_len(limit)?;
        if SourceOffset::new(source_len) != self.source_end {
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

    fn copy_window_ending_at<'a>(
        &self,
        request: BackwardWindowRequest,
        output: &'a mut Vec<u8>,
    ) -> Result<SourceText<'a>, LoadError> {
        self.require_unchanged()?;
        if request.end() <= self.source_start || request.end() > self.source_end {
            return Err(self.invalid_window("backward file window ends outside the source"));
        }

        let available =
            usize::try_from(request.end().get() - self.source_start.get()).unwrap_or(usize::MAX);
        let target_len = request.target_bytes().min(available);
        let read_len = target_len
            .saturating_add(UTF8_BOUNDARY_SLOP_BYTES)
            .min(available);
        let read_start = request
            .end()
            .checked_sub(read_len)
            .ok_or_else(|| self.invalid_window("backward file window coordinates overflow"))?;

        output.clear();
        output
            .try_reserve_exact(read_len)
            .map_err(|_| LoadError::Allocation("backward file window"))?;
        output.resize(read_len, 0);
        self.read_exact_at(output, read_start)?;

        let mut window_start = read_len - target_len;
        while window_start > 0 && output[window_start] & 0b1100_0000 == 0b1000_0000 {
            window_start -= 1;
        }
        if let Err(error) = std::str::from_utf8(&output[window_start..]) {
            let relative = window_start
                .checked_add(error.valid_up_to())
                .ok_or_else(|| self.invalid_window("UTF-8 error coordinates overflow"))?;
            let offset = read_start
                .checked_add(relative)
                .ok_or_else(|| self.invalid_window("UTF-8 error coordinates overflow"))?
                .get();
            return Err(LoadError::InvalidUtf8 {
                path: self.path.clone(),
                offset,
            });
        }

        let window_len = read_len - window_start;
        output.copy_within(window_start.., 0);
        output.truncate(window_len);
        let text = std::str::from_utf8(output).expect("backward file windows retain valid UTF-8");
        let source_start = request
            .end()
            .checked_sub(window_len)
            .ok_or_else(|| self.invalid_window("backward file window coordinates overflow"))?;
        let window = SourceText::with_start(text, source_start)
            .ok_or_else(|| self.invalid_window("backward file window coordinates overflow"))?;
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

    #[cfg(test)]
    fn bounded_len(&self, limit: u64) -> Result<u64, LoadError> {
        let metadata = self.file.metadata().map_err(|source| LoadError::Read {
            path: self.path.clone(),
            source,
        })?;
        if metadata.len() > limit {
            return Err(LoadError::TooLarge {
                path: self.path.clone(),
                limit,
            });
        }
        Ok(metadata.len())
    }

    fn content_start(&self, source_len: u64) -> Result<u64, LoadError> {
        if source_len < UTF8_BOM_BYTES as u64 {
            return Ok(0);
        }
        let mut prefix = [0_u8; UTF8_BOM_BYTES];
        self.read_exact_at(&mut prefix, SourceOffset::ZERO)?;
        Ok(if prefix == [0xef, 0xbb, 0xbf] {
            UTF8_BOM_BYTES as u64
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
        let remaining_bytes = source_end.get() - request.start().get();
        let maximum_window = request
            .target_bytes()
            .saturating_add(UTF8_BOUNDARY_SLOP_BYTES);
        let remaining = usize::try_from(
            remaining_bytes
                .min(u64::try_from(maximum_window).expect("window targets fit source coordinates")),
        )
        .expect("bounded file windows fit the process address space");
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

        let window_len = match std::str::from_utf8(cache) {
            Ok(text) => {
                let mut window_len = target_len;
                while window_len < text.len() && !text.is_char_boundary(window_len) {
                    window_len += 1;
                }
                window_len
            }
            Err(error)
                if error.error_len().is_none()
                    && u64::try_from(read_len).expect("file window lengths fit u64")
                        < remaining_bytes
                    && error.valid_up_to() >= target_len =>
            {
                error.valid_up_to()
            }
            Err(error) => {
                let offset = request
                    .start()
                    .checked_add(error.valid_up_to())
                    .ok_or_else(|| self.invalid_window("UTF-8 error coordinates overflow"))?
                    .get();
                return Err(LoadError::InvalidUtf8 {
                    path: self.path.clone(),
                    offset,
                });
            }
        };

        cache.truncate(window_len);
        let text = std::str::from_utf8(cache).expect("file windows retain validated UTF-8");
        SourceText::with_start(text, request.start())
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
        let FileStability::Tracked(fingerprint) = self.stability else {
            return Ok(());
        };
        #[cfg(test)]
        self.stability_checks.set(
            self.stability_checks
                .get()
                .checked_add(1)
                .expect("stability check count overflow"),
        );
        let metadata = self.file.metadata().map_err(|source| LoadError::Read {
            path: self.path.clone(),
            source,
        })?;
        if FileFingerprint::from_metadata(&metadata) != fingerprint {
            return Err(self.changed());
        }
        Ok(())
    }

    #[cfg(test)]
    fn stability_checks(&self) -> usize {
        self.stability_checks.get()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{self, Cursor, Read},
    };

    use tempfile::tempdir;
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;

    fn finish_index(document: &mut Document) -> Result<(), LoadError> {
        let mut cache = DocumentCache::default();
        while document.advance_line_index(&mut cache)? {}
        Ok(())
    }

    fn load_indexed(path: PathBuf) -> Result<Document, LoadError> {
        let mut document = load(path)?;
        finish_index(&mut document)?;
        Ok(document)
    }

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

    fn file_store(document: &Document) -> &FileStore {
        match &document.store {
            DocumentStore::File(store) => store,
            DocumentStore::InMemory(_) => panic!("expected file-backed document"),
        }
    }

    struct ChunkedReader {
        bytes: Vec<u8>,
        cursor: usize,
        chunk: usize,
        interrupted: bool,
    }

    impl ChunkedReader {
        fn new(bytes: &[u8], chunk: usize) -> Self {
            Self {
                bytes: bytes.to_vec(),
                cursor: 0,
                chunk,
                interrupted: true,
            }
        }
    }

    impl Read for ChunkedReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.interrupted {
                self.interrupted = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.cursor == self.bytes.len() {
                return Ok(0);
            }
            let count = output
                .len()
                .min(self.chunk)
                .min(self.bytes.len() - self.cursor);
            output[..count].copy_from_slice(&self.bytes[self.cursor..self.cursor + count]);
            self.cursor += count;
            Ok(count)
        }
    }

    struct EndlessReader {
        consumed: usize,
    }

    impl Read for EndlessReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            output.fill(b'a');
            self.consumed += output.len();
            Ok(output.len())
        }
    }

    struct FailedReader;

    impl Read for FailedReader {
        fn read(&mut self, _output: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("source failed"))
        }
    }

    #[test]
    fn standard_input_preserves_bom_coordinates_and_display_identity() {
        let bytes = b"\xef\xbb\xbfone\r\ntwo\n";
        let mut reader = Cursor::new(bytes);
        let mut document = load_standard_input(&mut reader).unwrap();
        assert_eq!(document.display_name(), STANDARD_INPUT_NAME);
        assert_eq!(document.display_path(), STANDARD_INPUT_NAME);
        assert_eq!(document.source_start(), SourceOffset::new(3));
        assert_eq!(
            document.source_end(),
            SourceOffset::new(u64::try_from(bytes.len()).unwrap())
        );
        assert_eq!(read_all(&document), "one\r\ntwo\n");
        finish_index(&mut document).unwrap();
        let mut cache = DocumentCache::default();
        let position = document
            .reader(&mut cache)
            .line_position(document.source_end())
            .unwrap()
            .unwrap();
        assert_eq!((position.current(), position.total()), (3, Some(3)));
    }

    #[test]
    fn standard_input_uses_a_private_snapshot_without_stability_checks() {
        let mut source = Cursor::new("one\n🙂\n");
        let mut document = load_standard_input(&mut source).unwrap();
        assert_eq!(
            file_store(&document).stability,
            FileStability::PrivateSnapshot
        );
        assert!(document.input_identity().is_none());
        assert_eq!(file_store(&document).stability_checks.get(), 0);

        document.validate().unwrap();
        assert_eq!(read_all(&document), "one\n🙂\n");
        finish_index(&mut document).unwrap();

        assert_eq!(file_store(&document).stability_checks.get(), 0);
    }

    #[test]
    fn tracked_files_count_stability_checks_per_read() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("tracked.txt");
        fs::write(&path, "stable").unwrap();
        let document = load(path).unwrap();
        assert!(matches!(
            file_store(&document).stability,
            FileStability::Tracked(_)
        ));
        assert_eq!(file_store(&document).stability_checks.get(), 0);

        document.validate().unwrap();
        assert_eq!(read_all(&document), "stable");

        assert_eq!(file_store(&document).stability_checks.get(), 3);
    }

    #[test]
    fn private_snapshot_windows_preserve_utf8_validation() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"valid").unwrap();
        let alias = file.try_clone().unwrap();
        let store =
            FileStore::from_private_snapshot(file, PathBuf::from(STANDARD_INPUT_NAME), 5).unwrap();
        assert_eq!(
            std::os::unix::fs::FileExt::write_at(&alias, b"\xff", 0).unwrap(),
            1
        );

        let mut output = Vec::new();
        let error = store
            .copy_window(
                WindowRequest::new(SourceOffset::ZERO, NonZeroUsize::new(1).unwrap()),
                &mut output,
            )
            .unwrap_err();

        assert!(matches!(error, LoadError::InvalidUtf8 { offset: 0, .. }));
        assert_eq!(store.stability_checks.get(), 0);
    }

    #[test]
    fn standard_input_enforces_its_limit_with_one_probe_byte() {
        let mut exact = Cursor::new(b"abcd");
        let document = load_standard_input_with_limit(&mut exact, 4).unwrap();
        assert_eq!(read_all(&document), "abcd");

        let mut endless = EndlessReader { consumed: 0 };
        assert!(matches!(
            load_standard_input_with_limit(&mut endless, 4),
            Err(LoadError::StandardInputTooLarge { limit: 4 })
        ));
        assert_eq!(endless.consumed, 5);
    }

    #[test]
    fn standard_input_validates_utf8_across_every_small_chunk_size() {
        let text = "\u{feff}\u{00a2}\u{20ac}\u{1f642}z";
        for chunk in 1..=text.len() {
            let mut reader = ChunkedReader::new(text.as_bytes(), chunk);
            let document =
                load_standard_input_with_limit(&mut reader, u64::try_from(text.len()).unwrap())
                    .unwrap();
            assert_eq!(read_all(&document), "\u{00a2}\u{20ac}\u{1f642}z");
        }
    }

    #[test]
    fn standard_input_reports_precise_utf8_errors_after_size_resolution() {
        for bytes in [b"ok\xff".as_slice(), b"ok\xe2\x82".as_slice()] {
            let mut reader = ChunkedReader::new(bytes, 1);
            assert!(matches!(
                load_standard_input_with_limit(&mut reader, 8),
                Err(LoadError::InvalidStandardInputUtf8 { offset: 2 })
            ));
        }

        let mut oversized = ChunkedReader::new(b"\xffabcd", 1);
        assert!(matches!(
            load_standard_input_with_limit(&mut oversized, 4),
            Err(LoadError::StandardInputTooLarge { limit: 4 })
        ));
    }

    #[test]
    fn standard_input_utf8_errors_match_the_standard_validator_at_every_split() {
        for bytes in [
            b"\x80".as_slice(),
            b"a\xc2A".as_slice(),
            b"a\xe0\x80\x80".as_slice(),
            b"a\xed\xa0\x80".as_slice(),
            b"a\xf0\x90\x80A".as_slice(),
            b"a\xf4\x90\x80\x80".as_slice(),
            b"a\xf0\x90\x80".as_slice(),
        ] {
            let expected =
                u64::try_from(std::str::from_utf8(bytes).unwrap_err().valid_up_to()).unwrap();
            for chunk in 1..=bytes.len() {
                let mut reader = ChunkedReader::new(bytes, chunk);
                assert!(matches!(
                    load_standard_input_with_limit(&mut reader, 32),
                    Err(LoadError::InvalidStandardInputUtf8 { offset })
                        if offset == expected
                ));
            }
        }
    }

    #[test]
    fn standard_input_preserves_non_interrupted_read_errors() {
        let mut reader = FailedReader;
        assert!(matches!(
            load_standard_input_with_limit(&mut reader, 4),
            Err(LoadError::ReadStandardInput { source })
                if source.kind() == io::ErrorKind::Other
                    && source.to_string() == "source failed"
        ));
    }

    #[test]
    fn loads_symlinks_and_preserves_source_coordinates() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let link = directory.path().join("link.txt");
        fs::write(&source, b"\xef\xbb\xbfone\r\ntwo\rthree\n\xef\xbb\xbf").unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let document = load_indexed(link).unwrap();
        assert_eq!(read_all(&document), "one\r\ntwo\rthree\n\u{feff}");
        assert_eq!(document.source_start(), SourceOffset::new(3));
        assert_eq!(document.source_end(), SourceOffset::new(21));
        let mut cache = DocumentCache::default();
        let position = document
            .reader(&mut cache)
            .line_position(SourceOffset::new(18))
            .unwrap()
            .unwrap();
        assert_eq!((position.current(), position.total()), (4, Some(4)));
        assert_eq!(document.display_name(), "link.txt");
    }

    #[test]
    fn rejects_invalid_utf8_at_the_original_byte_offset() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("invalid.txt");
        fs::write(&path, b"ok\xffbad").unwrap();
        assert!(matches!(
            load_indexed(path),
            Err(LoadError::InvalidUtf8 { offset: 2, .. })
        ));
    }

    #[test]
    fn loading_defers_validation_beyond_the_first_index_window() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("deferred-invalid.txt");
        let invalid_offset = SOURCE_WINDOW_BYTES + 10;
        let mut bytes = vec![b'a'; invalid_offset];
        bytes.push(0xff);
        fs::write(&path, bytes).unwrap();

        let mut document = load(path).unwrap();
        assert!(!document.line_index_complete());
        let mut cache = DocumentCache::default();
        assert!(document.advance_line_index(&mut cache).unwrap());
        assert!(!document.line_index_complete());
        assert!(matches!(
            document.advance_line_index(&mut cache),
            Err(LoadError::InvalidUtf8 { offset, .. })
                if offset == u64::try_from(invalid_offset).unwrap()
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
    fn grapheme_cache_hits_reject_changes_to_the_open_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("cached.txt");
        fs::write(&path, "stable").unwrap();
        let document = load(path.clone()).unwrap();
        let mut cache = DocumentCache::default();
        {
            let mut reader = document.reader(&mut cache);
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("s")
            );
        }

        fs::write(path, "change").unwrap();
        let mut reader = document.reader(&mut cache);
        assert!(matches!(
            reader.graphemes(SourceOffset::ZERO),
            Err(LoadError::Read { source, .. }) if source.kind() == io::ErrorKind::InvalidData
        ));
    }

    #[test]
    fn file_windows_extend_to_utf8_boundaries_with_bounded_slop() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("unicode.txt");
        fs::write(&path, "🙂abc").unwrap();
        let store = FileStore::open(path, 16).unwrap();
        let end = SourceOffset::new(7);
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
        assert_eq!(second.as_str(), "a");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn backward_file_windows_extend_to_utf8_boundaries_with_exact_storage() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("backward-unicode.txt");
        fs::write(&path, "\u{feff}a🙂éz").unwrap();
        let store = FileStore::open(path, 16).unwrap();
        let one = NonZeroUsize::new(1).unwrap();
        let mut cache = Vec::new();
        let end = store.source_start.checked_add("a🙂é".len()).unwrap();

        let third = store
            .copy_window_ending_at(BackwardWindowRequest::new(end, one), &mut cache)
            .unwrap();
        assert_eq!(third.as_str(), "é");
        assert_eq!(third.end(), end);
        let second_end = third.start();
        assert_eq!(cache.len(), "é".len());

        let second = store
            .copy_window_ending_at(BackwardWindowRequest::new(second_end, one), &mut cache)
            .unwrap();
        assert_eq!(second.as_str(), "🙂");
        assert_eq!(second.end(), second_end);
        let first_end = second.start();
        assert_eq!(cache.len(), "🙂".len());

        let first = store
            .copy_window_ending_at(BackwardWindowRequest::new(first_end, one), &mut cache)
            .unwrap();
        assert_eq!(first.as_str(), "a");
        assert_eq!(first.start(), store.source_start);
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
            load_indexed(path),
            Err(LoadError::InvalidUtf8 { offset, .. })
                if offset == u64::try_from(SOURCE_WINDOW_BYTES - 1).unwrap()
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
            load_indexed(path),
            Err(LoadError::InvalidUtf8 { offset, .. })
                if offset == u64::try_from(SOURCE_WINDOW_BYTES - 1).unwrap()
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
        let document = load_indexed(path).unwrap();
        let mut cache = DocumentCache::default();
        let position = document
            .reader(&mut cache)
            .line_position(SourceOffset::from_usize(SOURCE_WINDOW_BYTES + 1))
            .unwrap()
            .unwrap();

        assert_eq!((position.current(), position.total()), (2, Some(2)));
    }

    #[test]
    fn line_queries_read_only_the_required_checkpoint_prefix() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2);
        let document = Document::new(Path::new("long-line.txt"), text).unwrap();
        let mut cache = DocumentCache::default();
        let position = document
            .reader(&mut cache)
            .line_position(SourceOffset::ZERO)
            .unwrap()
            .unwrap();

        assert_eq!((position.current(), position.total()), (1, Some(1)));
        assert_eq!(cache.chunk.len(), 1);
    }

    #[test]
    fn incremental_line_queries_match_complete_indexes_at_every_line_ending_split() {
        let directory = tempdir().unwrap();

        for ending in ["\r", "\n", "\r\n"] {
            for split in 0..=ending.len() {
                let header = "head\r\n";
                let ending_start = SOURCE_WINDOW_BYTES - split;
                let mut text = String::with_capacity(SOURCE_WINDOW_BYTES + 32);
                text.push_str(header);
                text.push_str(&"x".repeat(ending_start - header.len()));
                text.push_str(ending);
                text.push_str("tail\nend\rfinal");

                let path = directory
                    .path()
                    .join(format!("ending-{}-{split}.txt", ending.len()));
                fs::write(&path, &text).unwrap();
                let reference = Document::from_text(Path::new("reference.txt"), text.clone());
                let mut document = load(path).unwrap();
                let mut index_cache = DocumentCache::default();
                assert!(document.advance_line_index(&mut index_cache).unwrap());
                assert!(!document.line_index_complete());

                let frontier = SourceOffset::from_usize(SOURCE_WINDOW_BYTES);
                let frontier_pending_cr = text.as_bytes()[SOURCE_WINDOW_BYTES - 1] == b'\r';
                let ending_end = ending_start + ending.len();
                let mut probes = vec![
                    SourceOffset::ZERO,
                    SourceOffset::from_usize(ending_start.saturating_sub(1)),
                    frontier,
                    SourceOffset::from_usize((ending_end + 1).min(text.len())),
                    SourceOffset::from_usize(text.len()),
                ];
                probes.extend(
                    (ending_start..=ending_end.min(text.len())).map(SourceOffset::from_usize),
                );
                probes.sort_unstable();
                probes.dedup();

                let mut reference_cache = DocumentCache::default();
                let mut partial_cache = DocumentCache::default();
                for offset in probes.iter().copied() {
                    let expected_position = reference
                        .reader(&mut reference_cache)
                        .line_position(offset)
                        .unwrap()
                        .unwrap();
                    let actual_position = document
                        .reader(&mut partial_cache)
                        .line_position(offset)
                        .unwrap();
                    let expected_coverage =
                        offset < frontier || (offset == frontier && !frontier_pending_cr);
                    assert_eq!(
                        document.line_index_covers(offset),
                        expected_coverage,
                        "ending={ending:?}, split={split}, offset={}",
                        offset.get()
                    );
                    if expected_coverage {
                        let actual_position = actual_position.unwrap();
                        assert_eq!(
                            actual_position.current(),
                            expected_position.current(),
                            "ending={ending:?}, split={split}, offset={}",
                            offset.get()
                        );
                        assert_eq!(actual_position.total(), None);
                    } else {
                        assert_eq!(actual_position, None);
                    }

                    let expected_start = reference
                        .reader(&mut reference_cache)
                        .line_start_at_or_before(offset)
                        .unwrap();
                    let actual_start = document
                        .reader(&mut partial_cache)
                        .line_start_at_or_before(offset)
                        .unwrap();
                    assert_eq!(
                        actual_start,
                        expected_start,
                        "ending={ending:?}, split={split}, offset={}",
                        offset.get()
                    );
                }

                finish_index(&mut document).unwrap();
                assert!(document.line_index_complete());
                let mut complete_cache = DocumentCache::default();
                for offset in probes.iter().copied() {
                    assert!(document.line_index_covers(offset));
                    let expected_position = reference
                        .reader(&mut reference_cache)
                        .line_position(offset)
                        .unwrap();
                    let actual_position = document
                        .reader(&mut complete_cache)
                        .line_position(offset)
                        .unwrap();
                    assert_eq!(
                        actual_position,
                        expected_position,
                        "ending={ending:?}, split={split}, offset={}",
                        offset.get()
                    );
                }
            }
        }
    }

    #[test]
    fn file_backed_line_scans_match_contiguous_coordinates() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("lines.txt");
        let text = "\u{feff}a\r\né\rb\nc";
        fs::write(&path, text).unwrap();
        let document = load_indexed(path).unwrap();
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
                reader.line_position(offset).unwrap().unwrap(),
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
    fn unindexed_line_starts_are_found_from_bounded_backward_windows() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("partial-lines.txt");
        let mut text = String::from("prefix\r\n");
        let long_line_start = text.len();
        text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES + 17));
        let cr = text.len();
        text.push_str("\r\nlast");
        fs::write(&path, text).unwrap();

        let document = load(path).unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        assert_eq!(
            reader
                .line_start_at_or_before(SourceOffset::from_usize(cr + 1))
                .unwrap(),
            SourceOffset::from_usize(long_line_start)
        );
        assert_eq!(
            reader
                .line_start_at_or_before(SourceOffset::from_usize(cr + 2))
                .unwrap(),
            SourceOffset::from_usize(cr + 2)
        );
        assert_eq!(reader.line_position(reader.source_end()).unwrap(), None);
    }

    #[test]
    fn incremental_graphemes_match_contiguous_segmentation_at_every_small_window_size() {
        let text = "\u{feff}a\r\ne\u{301}🇷🇸🇮🇴👩\u{200d}🔬क्\u{200d}ष\u{200b}z";
        let memory = Document::from_text(Path::new("memory.txt"), text.to_owned());
        let directory = tempdir().unwrap();
        let path = directory.path().join("file.txt");
        fs::write(&path, text).unwrap();
        let file = load(path).unwrap();
        let expected: Vec<_> = text[UTF8_BOM_BYTES..]
            .grapheme_indices(true)
            .map(|(relative, grapheme)| {
                let start = SourceOffset::new(UTF8_BOM_BYTES as u64)
                    .checked_add(relative)
                    .unwrap();
                (
                    start,
                    start.checked_add(grapheme.len()).unwrap(),
                    grapheme.to_owned(),
                )
            })
            .collect();

        for (backend, document) in [("memory", &memory), ("file", &file)] {
            for window_bytes in 1..=8 {
                let mut cache = DocumentCache::with_window_bytes(window_bytes);
                let actual = {
                    let mut reader = document.reader(&mut cache);
                    let mut graphemes = reader.graphemes(reader.source_start()).unwrap();
                    let mut actual = Vec::new();
                    while let Some(grapheme) = graphemes.next_grapheme().unwrap() {
                        actual.push((
                            grapheme.start(),
                            grapheme.end(),
                            grapheme
                                .text()
                                .expect("test graphemes fit the limit")
                                .to_owned(),
                        ));
                    }
                    actual
                };
                assert_eq!(actual, expected, "{backend} window size {window_bytes}");
                assert_eq!(
                    cache.metrics().grapheme_utf8_validated_bytes(),
                    cache.metrics().grapheme_window_returned_bytes()
                );
            }
        }
    }

    #[test]
    fn ascii_graphemes_validate_each_cached_byte_once() {
        let text = "a".repeat(SOURCE_WINDOW_BYTES);
        let document = Document::from_text(Path::new("ascii.txt"), text);
        let mut cache = DocumentCache::default();
        let grapheme_count = {
            let mut reader = document.reader(&mut cache);
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            let mut count = 0;
            while let Some(grapheme) = graphemes.next_grapheme().unwrap() {
                assert_eq!(grapheme.text(), Some("a"));
                count += 1;
            }
            count
        };

        assert_eq!(grapheme_count, SOURCE_WINDOW_BYTES);
        assert_eq!(cache.metrics().grapheme_emissions(), SOURCE_WINDOW_BYTES);
        assert_eq!(cache.metrics().grapheme_window_calls(), 1);
        assert_eq!(
            cache.metrics().grapheme_window_returned_bytes(),
            SOURCE_WINDOW_BYTES
        );
        assert_eq!(
            cache.metrics().grapheme_utf8_validated_bytes(),
            SOURCE_WINDOW_BYTES
        );
    }

    #[test]
    fn nearby_grapheme_cursors_reuse_the_loaded_window() {
        let document = Document::from_text(Path::new("cache.txt"), "abcdefgh".to_owned());
        let mut cache = DocumentCache::with_window_bytes(4);
        let mut reader = document.reader(&mut cache);

        {
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("a")
            );
        }
        assert_eq!(reader.cache.grapheme.as_bytes(), b"abcd");
        assert_eq!(reader.cache.grapheme_start, SourceOffset::ZERO);
        assert_eq!(reader.cache.grapheme_end, SourceOffset::new(4));

        {
            let mut graphemes = reader.graphemes(SourceOffset::new(1)).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("b")
            );
        }
        assert_eq!(reader.cache.grapheme.as_bytes(), b"abcd");
        assert_eq!(reader.cache.grapheme_start, SourceOffset::ZERO);
        assert_eq!(reader.cache.grapheme_end, SourceOffset::new(4));
    }

    #[test]
    fn document_identity_exhaustion_does_not_wrap() {
        let counter = AtomicU64::new(u64::MAX - 1);
        let last = DocumentId::try_next(&counter).unwrap();

        assert_eq!(last.get(), u64::MAX - 1);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        assert_eq!(DocumentId::try_next(&counter), None);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn grapheme_cache_identity_survives_document_moves() {
        let document = Document::from_text(Path::new("move.txt"), "abcdefgh".to_owned());
        let mut cache = DocumentCache::with_window_bytes(4);
        let id = {
            let mut reader = document.reader(&mut cache);
            let id = reader.document_id();
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("a")
            );
            id
        };

        cache.reset_metrics();
        let document = Box::new(document);
        {
            let mut reader = document.reader(&mut cache);
            assert_eq!(reader.document_id(), id);
            let mut graphemes = reader.graphemes(SourceOffset::new(1)).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("b")
            );
        }

        assert_eq!(cache.metrics().grapheme_window_calls(), 0);
    }

    #[test]
    fn grapheme_cache_rejects_another_document_with_the_same_range() {
        let first = Document::from_text(Path::new("first.txt"), "abcd".to_owned());
        let second = Document::from_text(Path::new("second.txt"), "wxyz".to_owned());
        let mut cache = DocumentCache::with_window_bytes(4);

        let first_id = {
            let mut reader = first.reader(&mut cache);
            let id = reader.document_id();
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("a")
            );
            id
        };
        cache.reset_metrics();
        let second_id = {
            let mut reader = second.reader(&mut cache);
            let id = reader.document_id();
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            assert_eq!(
                graphemes.next_grapheme().unwrap().unwrap().text(),
                Some("w")
            );
            id
        };

        assert_ne!(first_id, second_id);
        assert_eq!(cache.metrics().grapheme_window_calls(), 1);
    }

    #[test]
    fn incremental_grapheme_segmentation_advances_linearly() {
        let combining_count = (256 * 1024 - 1) / '\u{301}'.len_utf8();
        let mut cluster = String::with_capacity(1 + combining_count * '\u{301}'.len_utf8());
        cluster.push('a');
        cluster.extend(std::iter::repeat_n('\u{301}', combining_count));
        let cluster_len = cluster.len();
        cluster.push('z');

        let document = Document::from_text(Path::new("linear-grapheme.txt"), cluster);
        let mut cache = DocumentCache::with_window_bytes(257);
        {
            let mut reader = document.reader(&mut cache);
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            let first = graphemes.next_grapheme().unwrap().unwrap();
            assert_eq!(first.end(), SourceOffset::from_usize(cluster_len));
            assert!(first.text().is_some());
        }

        assert!(cache.metrics().segmentation_runs() > 1);
        assert_eq!(cache.metrics().segmentation_advanced_bytes(), cluster_len);
        assert_eq!(
            cache.metrics().grapheme_window_returned_bytes(),
            cluster_len + 1
        );
        assert_eq!(
            cache.metrics().grapheme_utf8_validated_bytes(),
            cluster_len + 1
        );
    }

    fn exact_limit_grapheme() -> String {
        let mut grapheme = String::with_capacity(MAX_GRAPHEME_BYTES);
        grapheme.push('é');
        grapheme.extend(std::iter::repeat_n(
            '\u{301}',
            (MAX_GRAPHEME_BYTES - 'é'.len_utf8()) / '\u{301}'.len_utf8(),
        ));
        assert_eq!(grapheme.len(), MAX_GRAPHEME_BYTES);
        grapheme
    }

    #[test]
    fn exact_limit_graphemes_keep_text_before_a_following_scalar() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("exact-limit.txt");
        let mut text = exact_limit_grapheme();
        text.push('🙂');
        fs::write(&path, &text).unwrap();
        let memory = Document::from_text(Path::new("exact-limit-memory.txt"), text.clone());
        let file = load(path).unwrap();

        for (backend, document) in [("memory", memory), ("file", file)] {
            let mut cache = DocumentCache::default();
            {
                let mut reader = document.reader(&mut cache);
                let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
                let first = graphemes.next_grapheme().unwrap().unwrap();
                assert_eq!(first.start(), SourceOffset::ZERO, "{backend}");
                assert_eq!(
                    first.end(),
                    SourceOffset::from_usize(MAX_GRAPHEME_BYTES),
                    "{backend}"
                );
                assert_eq!(
                    first.text().map(str::len),
                    Some(MAX_GRAPHEME_BYTES),
                    "{backend}"
                );
                let second = graphemes.next_grapheme().unwrap().unwrap();
                assert_eq!(second.text(), Some("🙂"), "{backend}");
                assert!(graphemes.next_grapheme().unwrap().is_none(), "{backend}");
            }
            assert!(cache.grapheme.len() <= MAX_GRAPHEME_BYTES + UTF8_BOUNDARY_SLOP_BYTES + 1);
        }
    }

    #[test]
    fn exact_limit_graphemes_at_eof_need_no_lookahead() {
        let document =
            Document::from_text(Path::new("exact-limit-eof.txt"), exact_limit_grapheme());
        let mut cache = DocumentCache::default();
        {
            let mut reader = document.reader(&mut cache);
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            let first = graphemes.next_grapheme().unwrap().unwrap();
            assert_eq!(first.text().map(str::len), Some(MAX_GRAPHEME_BYTES));
            assert!(graphemes.next_grapheme().unwrap().is_none());
        }
        assert_eq!(cache.grapheme.len(), MAX_GRAPHEME_BYTES);
        assert_eq!(
            cache.metrics().grapheme_window_returned_bytes(),
            MAX_GRAPHEME_BYTES
        );
    }

    #[test]
    fn exact_limit_continuations_split_before_the_lookahead() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("continued-limit.txt");
        let mut text = exact_limit_grapheme();
        text.push('\u{301}');
        text.push('z');
        fs::write(&path, &text).unwrap();
        let memory = Document::from_text(Path::new("continued-limit-memory.txt"), text.clone());
        let file = load(path).unwrap();

        for (backend, document) in [("memory", memory), ("file", file)] {
            let source_end = document.source_end();
            let mut cache = DocumentCache::default();
            let mut reader = document.reader(&mut cache);
            let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
            let first = graphemes.next_grapheme().unwrap().unwrap();
            assert_eq!(first.text(), None, "{backend}");
            let first_end = first.end();
            assert_eq!(
                first_end,
                SourceOffset::from_usize(MAX_GRAPHEME_BYTES),
                "{backend}"
            );
            assert!(
                graphemes.reader.cache.grapheme.len()
                    <= MAX_GRAPHEME_BYTES + UTF8_BOUNDARY_SLOP_BYTES + 1,
                "{backend}"
            );
            let mut next_start = first_end;
            let mut saw_final = false;
            while let Some(grapheme) = graphemes.next_grapheme().unwrap() {
                assert_eq!(grapheme.start(), next_start, "{backend}");
                saw_final |= grapheme.text() == Some("z");
                next_start = grapheme.end();
            }
            assert!(saw_final, "{backend}");
            assert_eq!(next_start, source_end, "{backend}");
        }
    }

    #[test]
    fn four_byte_extenders_are_retained_after_the_last_bounded_boundary() {
        let mut text = String::with_capacity(MAX_GRAPHEME_BYTES + UTF8_BOUNDARY_SLOP_BYTES + 2);
        text.push('a');
        text.extend(std::iter::repeat_n(
            '\u{301}',
            (MAX_GRAPHEME_BYTES - 2) / '\u{301}'.len_utf8(),
        ));
        assert_eq!(text.len(), MAX_GRAPHEME_BYTES - 1);
        text.push('\u{1f3fb}');
        text.push('z');

        let document = Document::from_text(Path::new("four-byte-lookahead.txt"), text);
        let source_end = document.source_end();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut graphemes = reader.graphemes(SourceOffset::ZERO).unwrap();
        let first = graphemes.next_grapheme().unwrap().unwrap();
        assert_eq!(first.text(), None);
        let first_end = first.end();
        assert_eq!(first_end, SourceOffset::from_usize(MAX_GRAPHEME_BYTES - 1));
        assert!(
            graphemes.reader.cache.grapheme.len()
                <= MAX_GRAPHEME_BYTES + UTF8_BOUNDARY_SLOP_BYTES + 1
        );
        let mut next_start = first_end;
        let mut saw_final = false;
        while let Some(grapheme) = graphemes.next_grapheme().unwrap() {
            assert_eq!(grapheme.start(), next_start);
            saw_final |= grapheme.text() == Some("z");
            next_start = grapheme.end();
        }
        assert!(saw_final);
        assert_eq!(next_start, source_end);
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
