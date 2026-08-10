use std::{
    env,
    ffi::OsString,
    fs, io,
    os::{
        fd::{AsFd as _, OwnedFd},
        unix::{ffi::OsStringExt as _, fs::MetadataExt as _},
    },
    path::{Component, Path, PathBuf},
};

use rustix::fs::{AtFlags, Mode, OFlags};

#[derive(Debug, Clone)]
pub(super) struct PathIdentity {
    exact: PathBuf,
    normalized: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedPathLocation {
    parent: FileIdentity,
    leaf: OsString,
}

/// Captures one cwd-relative pathname without changing its resolution semantics.
#[derive(Debug)]
pub(super) struct BoundPath {
    open_path: PathBuf,
    identity: PathIdentity,
}

impl BoundPath {
    pub(super) fn capture(path: &Path) -> io::Result<Self> {
        if path.is_absolute() {
            return Ok(Self::from_absolute(path.to_path_buf()));
        }
        let current_dir = env::current_dir()?;
        Ok(Self::from_absolute(current_dir.join(path)))
    }

    fn from_absolute(open_path: PathBuf) -> Self {
        debug_assert!(open_path.is_absolute());
        let identity = PathIdentity::from_absolute(open_path.clone());
        Self {
            open_path,
            identity,
        }
    }

    pub(super) fn open_path(&self) -> &Path {
        &self.open_path
    }

    pub(super) const fn identity(&self) -> &PathIdentity {
        &self.identity
    }

    pub(super) fn into_identity(self) -> PathIdentity {
        self.identity
    }
}

impl PathIdentity {
    fn from_absolute(exact: PathBuf) -> Self {
        debug_assert!(exact.is_absolute());
        let normalized = normalize_absolute(&exact);
        Self { exact, normalized }
    }

    pub(super) fn exactly_matches(&self, other: &Self) -> bool {
        self.exact == other.exact
    }

    pub(super) fn normalized_matches(&self, other: &Self) -> bool {
        self.normalized == other.normalized
    }
}

impl FileIdentity {
    pub(super) const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    pub(super) fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self::new(metadata.dev(), metadata.ino())
    }
}

#[derive(Debug)]
pub(super) struct ResolvedPath {
    parent: fs::File,
    location: ResolvedPathLocation,
    identity: PathIdentity,
}

#[derive(Debug)]
pub(super) struct ResolvedPathAnchor {
    parent: fs::File,
    location: ResolvedPathLocation,
}

impl ResolvedPath {
    pub(super) fn existing(path: &Path) -> io::Result<Self> {
        let resolved = fs::canonicalize(path)?;
        let (parent, leaf) = split_absolute(&resolved)?;
        Self::from_parent(parent, leaf)
    }

    pub(super) fn parent(path: &Path) -> io::Result<Self> {
        let (parent, leaf) = split_absolute(path)?;
        Self::from_parent(fs::canonicalize(parent)?, leaf)
    }

    fn from_parent(parent: PathBuf, leaf: OsString) -> io::Result<Self> {
        let identity = PathIdentity::from_absolute(parent.join(&leaf));
        let descriptor = rustix::fs::open(&parent, directory_open_flags(), Mode::empty())
            .map_err(io::Error::from)?;
        let parent = fs::File::from(descriptor);
        let metadata = parent.metadata()?;
        let location = ResolvedPathLocation {
            parent: FileIdentity::from_metadata(&metadata),
            leaf: leaf.clone(),
        };
        Ok(Self {
            parent,
            location,
            identity,
        })
    }

    pub(super) fn open(&self, flags: OFlags, mode: Mode) -> io::Result<OwnedFd> {
        rustix::fs::openat(
            self.parent.as_fd(),
            self.location.leaf.as_os_str(),
            flags,
            mode,
        )
        .map_err(io::Error::from)
    }

    pub(super) fn into_anchor(self) -> ResolvedPathAnchor {
        ResolvedPathAnchor {
            parent: self.parent,
            location: self.location,
        }
    }

    pub(super) const fn location(&self) -> &ResolvedPathLocation {
        &self.location
    }

    pub(super) const fn identity(&self) -> &PathIdentity {
        &self.identity
    }
}

impl ResolvedPathAnchor {
    pub(super) fn current_leaf_matches(&self, file: &impl std::os::fd::AsFd) -> io::Result<bool> {
        let entry = match rustix::fs::statat(
            self.parent.as_fd(),
            self.location.leaf.as_os_str(),
            AtFlags::empty(),
        ) {
            Ok(entry) => entry,
            Err(rustix::io::Errno::NOENT) => return Ok(false),
            Err(error) => return Err(io::Error::from(error)),
        };
        let opened = rustix::fs::fstat(file).map_err(io::Error::from)?;
        Ok(entry.st_dev == opened.st_dev && entry.st_ino == opened.st_ino)
    }

    pub(super) const fn location(&self) -> &ResolvedPathLocation {
        &self.location
    }
}

fn split_absolute(path: &Path) -> io::Result<(PathBuf, OsString)> {
    debug_assert!(path.is_absolute());
    use std::os::unix::ffi::OsStrExt as _;

    let bytes = path.as_os_str().as_bytes();
    let Some(separator) = bytes.iter().rposition(|byte| *byte == b'/') else {
        return Err(invalid_path());
    };
    let leaf = &bytes[separator + 1..];
    if leaf.is_empty() || leaf == b"." || leaf == b".." {
        return Err(invalid_path());
    }
    let parent = if separator == 0 {
        OsString::from("/")
    } else {
        OsString::from_vec(bytes[..separator].to_vec())
    };
    Ok((PathBuf::from(parent), OsString::from_vec(leaf.to_vec())))
}

fn invalid_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "path has no final file name")
}

#[cfg(target_os = "linux")]
fn directory_open_flags() -> OFlags {
    OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC
}

#[cfg(target_os = "macos")]
fn directory_open_flags() -> OFlags {
    OFlags::from_bits_retain(libc::O_SEARCH as u32) | OFlags::DIRECTORY | OFlags::CLOEXEC
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn directory_open_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC
}

fn normalize_absolute(path: &Path) -> PathBuf {
    debug_assert!(path.is_absolute());
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(name) => normalized.push(name),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        io::{Read as _, Write as _},
        os::unix::{ffi::OsStringExt as _, fs::PermissionsExt as _},
    };

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn binding_preserves_open_semantics_and_collapses_only_the_identity() {
        let path = Path::new("/workspace/link/../input.txt");
        let bound = BoundPath::capture(path).unwrap();
        let direct = BoundPath::capture(Path::new("/workspace/input.txt")).unwrap();

        assert_eq!(bound.open_path(), path);
        assert!(bound.identity().normalized_matches(direct.identity()));
    }

    #[test]
    fn lexical_identity_preserves_non_utf8_components() {
        let alias = PathBuf::from(OsString::from_vec(b"/workspace/\xff/./input.txt".to_vec()));
        let direct = PathBuf::from(OsString::from_vec(b"/workspace/\xff/input.txt".to_vec()));
        let bound = BoundPath::capture(&alias).unwrap();
        let direct = BoundPath::capture(&direct).unwrap();

        assert_eq!(bound.open_path(), alias);
        assert!(bound.identity().normalized_matches(direct.identity()));
    }

    #[test]
    fn relative_paths_capture_one_absolute_open_path_and_identity() {
        let relative = Path::new("lexical-component/../input.txt");
        let current_dir = env::current_dir().unwrap();
        let bound = BoundPath::capture(relative).unwrap();
        let direct = BoundPath::capture(&current_dir.join("input.txt")).unwrap();

        assert_eq!(bound.open_path(), current_dir.join(relative));
        assert!(bound.identity().normalized_matches(direct.identity()));
    }

    #[test]
    fn parent_components_cannot_escape_the_root() {
        let bound = BoundPath::capture(Path::new("/../../input.txt")).unwrap();
        let direct = BoundPath::capture(Path::new("/input.txt")).unwrap();
        assert!(bound.identity().normalized_matches(direct.identity()));
    }

    #[test]
    fn resolved_locations_follow_directory_symlinks_and_parent_components() {
        let directory = tempdir().unwrap();
        let real = directory.path().join("real");
        let child = real.join("child");
        fs::create_dir_all(&child).unwrap();
        let input = real.join("input.txt");
        fs::write(&input, "input").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();

        let through_alias = ResolvedPath::existing(&alias.join("..").join("input.txt")).unwrap();
        let direct = ResolvedPath::parent(&input).unwrap();

        assert_eq!(through_alias.location(), direct.location());
        assert!(
            through_alias
                .identity()
                .normalized_matches(direct.identity())
        );
    }

    #[test]
    fn open_stays_bound_to_the_compared_parent_descriptor() {
        let directory = tempdir().unwrap();
        let original = directory.path().join("original");
        let moved = directory.path().join("moved");
        fs::create_dir(&original).unwrap();
        let path = original.join("session.log");
        let resolved = ResolvedPath::parent(&path).unwrap();

        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        let descriptor = resolved
            .open(
                OFlags::WRONLY | OFlags::CREATE | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .unwrap();
        let mut file = fs::File::from(descriptor);
        file.write_all(b"bound").unwrap();

        assert_eq!(fs::read(moved.join("session.log")).unwrap(), b"bound");
        assert!(!path.exists());
    }

    #[test]
    fn post_open_slot_lookup_follows_the_current_input_leaf() {
        let directory = tempdir().unwrap();
        let log = directory.path().join("session.log");
        fs::write(&log, "log").unwrap();
        let alias = directory.path().join("input-link.txt");
        std::os::unix::fs::symlink(&log, &alias).unwrap();
        let resolved_input = ResolvedPath::parent(&alias).unwrap().into_anchor();
        let file = fs::File::open(&log).unwrap();

        assert!(resolved_input.current_leaf_matches(&file).unwrap());
    }

    #[test]
    fn resolved_parent_preserves_non_utf8_leaf_names() {
        let directory = tempdir().unwrap();
        let leaf = OsString::from_vec(b"session-\xff.log".to_vec());
        let path = directory.path().join(&leaf);
        let resolved = ResolvedPath::parent(&path).unwrap();

        assert_eq!(resolved.location.leaf, leaf);
    }

    #[test]
    fn resolved_parent_needs_search_but_not_read_permission() {
        let directory = tempdir().unwrap();
        let search_only = directory.path().join("search-only");
        fs::create_dir(&search_only).unwrap();
        fs::set_permissions(&search_only, fs::Permissions::from_mode(0o300)).unwrap();
        let path = search_only.join("session.log");

        let result = ResolvedPath::parent(&path).and_then(|resolved| {
            resolved
                .open(
                    OFlags::WRONLY | OFlags::CREATE | OFlags::CLOEXEC,
                    Mode::RUSR | Mode::WUSR,
                )
                .map(drop)
        });
        fs::set_permissions(&search_only, fs::Permissions::from_mode(0o700)).unwrap();

        result.unwrap();
        assert!(path.exists());
    }

    #[test]
    fn resolved_input_needs_search_but_not_read_permission() {
        let directory = tempdir().unwrap();
        let search_only = directory.path().join("search-only");
        fs::create_dir(&search_only).unwrap();
        let path = search_only.join("input.txt");
        fs::write(&path, "input").unwrap();
        fs::set_permissions(&search_only, fs::Permissions::from_mode(0o100)).unwrap();

        let result = ResolvedPath::existing(&path).and_then(|resolved| {
            let descriptor = resolved.open(
                OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::empty(),
            )?;
            let mut text = String::new();
            fs::File::from(descriptor).read_to_string(&mut text)?;
            Ok(text)
        });
        fs::set_permissions(&search_only, fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(result.unwrap(), "input");
    }
}
