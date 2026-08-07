use std::{
    fs::File,
    io::{self, Read},
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

use rustix::fs::{Mode, OFlags};

use crate::{
    error::{LoadError, sanitize_os},
    line_index::{LineIndex, LineIndexError, LinePosition},
    source::{SourceOffset, SourceText, WindowRequest},
};

pub const MAX_FILE_BYTES: usize = 33_554_432;
const READ_CHUNK_BYTES: usize = 16 * 1024;
const INDEX_WINDOW_BYTES: usize = 64 * 1024;
const UTF8_BOM_BYTES: usize = 3;

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

    fn new(path: &Path, source: String) -> Result<Self, LoadError> {
        let display_path = sanitize_os(path.as_os_str());
        let display_name = sanitize_os(path.file_name().unwrap_or(path.as_os_str()));
        let store = DocumentStore::in_memory(source);
        let line_index = build_line_index(&store, path)?;
        Ok(Self {
            store,
            line_index,
            display_path,
            display_name,
        })
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

    fn window<'a>(
        &'a self,
        request: WindowRequest,
        scratch: &'a mut Vec<u8>,
    ) -> io::Result<SourceText<'a>> {
        match self {
            Self::InMemory(store) => store.window(request, scratch),
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

    fn window<'a>(
        &'a self,
        request: WindowRequest,
        _scratch: &'a mut Vec<u8>,
    ) -> io::Result<SourceText<'a>> {
        self.source().window(request).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid source window request")
        })
    }
}

pub(super) fn load(path: PathBuf) -> Result<Document, LoadError> {
    let raw = read_raw(path, MAX_FILE_BYTES)?;
    let source = String::from_utf8(raw.bytes).map_err(|error| LoadError::InvalidUtf8 {
        path: raw.path.clone(),
        offset: error.utf8_error().valid_up_to(),
    })?;
    Document::new(&raw.path, source)
}

fn build_line_index(store: &DocumentStore, path: &Path) -> Result<LineIndex, LoadError> {
    let source = store.source();
    let mut index = LineIndex::new(source.start(), source.end())
        .map_err(|error| map_line_index_error(path, error))?;
    let target = NonZeroUsize::new(INDEX_WINDOW_BYTES).expect("index window size is nonzero");
    let mut scratch = Vec::new();
    let mut cursor = source.start();

    while cursor < source.end() {
        let window = store
            .window(WindowRequest::new(cursor, target), &mut scratch)
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

struct RawDocument {
    path: PathBuf,
    bytes: Vec<u8>,
}

fn read_raw(path: PathBuf, limit: usize) -> Result<RawDocument, LoadError> {
    let descriptor = rustix::fs::open(
        &path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|source| LoadError::Open {
        path: path.clone(),
        source: io::Error::from(source),
    })?;

    let mut file = File::from(descriptor);
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

    let known_len = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    let bytes = read_bounded(&mut file, known_len, &path, limit)?;
    Ok(RawDocument { path, bytes })
}

fn read_bounded(
    mut reader: impl Read,
    known_len: usize,
    path: &Path,
    limit: usize,
) -> Result<Vec<u8>, LoadError> {
    let maximum_read = limit
        .checked_add(1)
        .ok_or(LoadError::Allocation("bounded file buffer"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(known_len.min(maximum_read))
        .map_err(|_| LoadError::Allocation("file buffer"))?;

    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    while bytes.len() < maximum_read {
        let request = (maximum_read - bytes.len()).min(chunk.len());
        let count = loop {
            match reader.read(&mut chunk[..request]) {
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
            break;
        }
        bytes
            .try_reserve(count)
            .map_err(|_| LoadError::Allocation("file buffer"))?;
        bytes.extend_from_slice(&chunk[..count]);
    }

    if bytes.len() > limit {
        return Err(LoadError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Cursor};

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
        let path = Path::new("growing.txt");
        let error = read_bounded(Cursor::new(b"12345"), 1, path, 4).unwrap_err();
        assert!(matches!(error, LoadError::TooLarge { limit: 4, .. }));
    }

    #[test]
    fn rejects_directories() {
        let directory = tempdir().unwrap();
        assert!(matches!(
            read_raw(directory.path().to_path_buf(), 16),
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
        assert!(matches!(read_raw(fifo, 16), Err(LoadError::NotRegular(_))));
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
        let mut text = "a".repeat(INDEX_WINDOW_BYTES - 1);
        text.push_str("\r\nb");
        let document = Document::from_text(Path::new("book.txt"), text);
        let position = document
            .line_position(SourceOffset::from_usize(INDEX_WINDOW_BYTES + 1))
            .unwrap();

        assert_eq!((position.current(), position.total()), (2, 2));
    }
}
