use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use rustix::fs::{Mode, OFlags};

use crate::error::{LoadError, sanitize_os};

pub const MAX_FILE_BYTES: usize = 33_554_432;
const READ_CHUNK_BYTES: usize = 16 * 1024;

#[derive(Debug)]
pub(super) struct Document {
    text: String,
    display_path: String,
    display_name: String,
}

impl Document {
    pub(super) fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn len_bytes(&self) -> u32 {
        u32::try_from(self.text.len()).expect("normalized text is bounded to 32 MiB")
    }

    pub(super) fn display_path(&self) -> &str {
        &self.display_path
    }

    pub(super) fn display_name(&self) -> &str {
        &self.display_name
    }

    #[cfg(test)]
    pub(super) fn from_normalized(path: &Path, text: String) -> Self {
        Self::new(path, text)
    }

    fn new(path: &Path, text: String) -> Self {
        let display_path = sanitize_os(path.as_os_str());
        let display_name = sanitize_os(path.file_name().unwrap_or(path.as_os_str()));
        Self {
            text,
            display_path,
            display_name,
        }
    }
}

pub(super) fn load(path: PathBuf) -> Result<Document, LoadError> {
    let raw = read_raw(path, MAX_FILE_BYTES)?;
    if let Err(error) = std::str::from_utf8(&raw.bytes) {
        return Err(LoadError::InvalidUtf8 {
            path: raw.path,
            offset: error.valid_up_to(),
        });
    }

    let text = normalize_valid_utf8(raw.bytes);
    Ok(Document::new(&raw.path, text))
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

fn normalize_valid_utf8(mut bytes: Vec<u8>) -> String {
    let mut read = usize::from(bytes.starts_with(&[0xef, 0xbb, 0xbf])) * 3;
    let mut write = 0;

    while read < bytes.len() {
        if bytes[read] == b'\r' {
            bytes[write] = b'\n';
            write += 1;
            read += 1;
            if read < bytes.len() && bytes[read] == b'\n' {
                read += 1;
            }
        } else {
            bytes[write] = bytes[read];
            write += 1;
            read += 1;
        }
    }

    bytes.truncate(write);
    String::from_utf8(bytes).expect("normalization preserves validated UTF-8")
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Cursor};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn loads_symlinks_and_normalizes_valid_utf8_in_order() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let link = directory.path().join("link.txt");
        fs::write(&source, b"\xef\xbb\xbfone\r\ntwo\rthree\n\xef\xbb\xbf").unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();

        let document = load(link).unwrap();
        assert_eq!(document.text(), "one\ntwo\nthree\n\u{feff}");
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
        let document = Document::from_normalized(&path, "text".to_owned());
        assert_eq!(document.display_path(), "/tmp/book.txt");
        assert_eq!(document.text(), "text");
    }
}
