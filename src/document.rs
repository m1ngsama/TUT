use std::{
    fs::File,
    io,
    num::NonZeroUsize,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
};

use rustix::fs::{Mode, OFlags};

use crate::{
    error::{LoadError, sanitize_os},
    line_index::{LineIndex, LineIndexError, LinePosition},
    source::{SourceOffset, SourceText, WindowRequest},
};

pub const MAX_FILE_BYTES: usize = 33_554_432;
const SOURCE_WINDOW_BYTES: usize = 64 * 1024;
const UTF8_BOM_BYTES: usize = 3;
const UTF8_BOUNDARY_SLOP_BYTES: usize = 3;

#[derive(Debug)]
pub(super) struct Document {
    store: DocumentStore,
    line_index: LineIndex,
    display_path: String,
    display_name: String,
}

impl Document {
    pub(super) fn source(&self) -> SourceText<'_> {
        self.store.source()
    }

    pub(super) fn display_path(&self) -> &str {
        &self.display_path
    }

    pub(super) fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(super) fn line_position(&self, offset: SourceOffset) -> Option<LinePosition> {
        self.line_index.position(self.source(), offset)
    }

    #[cfg(test)]
    pub(super) fn from_text(path: &Path, text: String) -> Self {
        Self::new(path, text).expect("test documents fit the line-index budget")
    }

    #[cfg(test)]
    fn new(path: &Path, source: String) -> Result<Self, LoadError> {
        let store = DocumentStore::in_memory(source);
        let line_index = build_line_index(&store, path)?;
        Ok(Self::from_parts(path, store, line_index))
    }

    fn from_indexed(path: &Path, source: String, line_index: LineIndex) -> Self {
        Self::from_parts(path, DocumentStore::in_memory(source), line_index)
    }

    fn from_parts(path: &Path, store: DocumentStore, line_index: LineIndex) -> Self {
        Self {
            store,
            line_index,
            display_path: sanitize_os(path.as_os_str()),
            display_name: sanitize_os(path.file_name().unwrap_or(path.as_os_str())),
        }
    }
}

#[derive(Debug)]
enum DocumentStore {
    InMemory(InMemoryStore),
}

impl DocumentStore {
    fn in_memory(source: String) -> Self {
        Self::InMemory(InMemoryStore::new(source))
    }

    fn source(&self) -> SourceText<'_> {
        match self {
            Self::InMemory(store) => store.source(),
        }
    }

    #[cfg(test)]
    fn window(&self, request: WindowRequest) -> io::Result<SourceText<'_>> {
        match self {
            Self::InMemory(store) => store.window(request),
        }
    }
}

#[derive(Debug)]
struct InMemoryStore {
    source: String,
    content_start: usize,
}

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
        }
    }

    fn source(&self) -> SourceText<'_> {
        SourceText::with_start(
            &self.source[self.content_start..],
            SourceOffset::from_usize(self.content_start),
        )
        .expect("an in-memory source span fits in u64 coordinates")
    }

    #[cfg(test)]
    fn window(&self, request: WindowRequest) -> io::Result<SourceText<'_>> {
        self.source().window(request).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid source window request")
        })
    }
}

pub(super) fn load(path: PathBuf) -> Result<Document, LoadError> {
    let store = FileStore::open(path, MAX_FILE_BYTES)?;
    let (source, line_index) = store.read_source_and_index(MAX_FILE_BYTES)?;
    Ok(Document::from_indexed(&store.path, source, line_index))
}

#[cfg(test)]
fn build_line_index(store: &DocumentStore, path: &Path) -> Result<LineIndex, LoadError> {
    let source = store.source();
    let mut index = LineIndex::new(source.start(), source.end())
        .map_err(|error| map_line_index_error(path, error))?;
    let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
    let mut cursor = source.start();

    while cursor < source.end() {
        let window = store
            .window(WindowRequest::new(cursor, target))
            .map_err(|source| LoadError::Read {
                path: path.to_path_buf(),
                source,
            })?;
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

struct FileStore {
    file: File,
    path: PathBuf,
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

        Ok(Self { file, path })
    }

    fn read_source_and_index(&self, limit: usize) -> Result<(String, LineIndex), LoadError> {
        let source_len = self.bounded_len(limit)?;
        let source_end = SourceOffset::from_usize(source_len);
        let content_start = self.content_start(source_len)?;
        let content_start = SourceOffset::from_usize(content_start);
        let mut line_index = LineIndex::new(content_start, source_end)
            .map_err(|error| map_line_index_error(&self.path, error))?;
        let mut source = String::new();
        source
            .try_reserve_exact(source_len)
            .map_err(|_| LoadError::Allocation("file buffer"))?;
        let mut window_cache = Vec::new();
        let target = NonZeroUsize::new(SOURCE_WINDOW_BYTES).expect("source window size is nonzero");
        let mut cursor = SourceOffset::ZERO;

        while cursor < source_end {
            let window = self.window(
                WindowRequest::new(cursor, target),
                source_end,
                &mut window_cache,
            )?;
            let indexed_start = window.start().max(content_start);
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
            source.push_str(window.as_str());
            cursor = window.end();
        }

        line_index
            .finish()
            .map_err(|error| map_line_index_error(&self.path, error))?;
        let final_len = self.bounded_len(limit)?;
        if final_len != source_len {
            return Err(LoadError::Read {
                path: self.path.clone(),
                source: io::Error::new(io::ErrorKind::InvalidData, "file changed while reading"),
            });
        }
        Ok((source, line_index))
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
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn loads_symlinks_and_preserves_source_coordinates() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let link = directory.path().join("link.txt");
        fs::write(&source, b"\xef\xbb\xbfone\r\ntwo\rthree\n\xef\xbb\xbf").unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let document = load(link).unwrap();
        let source = document.source();
        assert_eq!(source.as_str(), "one\r\ntwo\rthree\n\u{feff}");
        assert_eq!(source.start(), SourceOffset::new(3));
        assert_eq!(source.end(), SourceOffset::new(21));
        let position = document.line_position(SourceOffset::new(18)).unwrap();
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
        let error = store.read_source_and_index(4).unwrap_err();
        assert!(matches!(error, LoadError::TooLarge { limit: 4, .. }));
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
        assert_eq!(document.source().as_str(), text);
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
        let position = document
            .line_position(SourceOffset::from_usize(SOURCE_WINDOW_BYTES + 1))
            .unwrap();

        assert_eq!((position.current(), position.total()), (2, 2));
    }
}
