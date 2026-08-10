use std::{
    env, io,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PathIdentity(PathBuf);

/// Captures one cwd-relative resolution without changing later path-component semantics.
/// `open_path` is used for I/O; only `identity` receives lexical normalization.
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
        let identity = PathIdentity(normalize_absolute(&open_path));
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
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    use super::*;

    #[test]
    fn binding_preserves_open_semantics_and_collapses_only_the_identity() {
        let path = Path::new("/workspace/link/../input.txt");
        let bound = BoundPath::capture(path).unwrap();
        let direct = BoundPath::capture(Path::new("/workspace/input.txt")).unwrap();

        assert_eq!(bound.open_path(), path);
        assert_eq!(bound.identity(), direct.identity());
    }

    #[test]
    fn lexical_identity_preserves_non_utf8_components() {
        let alias = PathBuf::from(OsString::from_vec(b"/workspace/\xff/./input.txt".to_vec()));
        let direct = PathBuf::from(OsString::from_vec(b"/workspace/\xff/input.txt".to_vec()));
        let bound = BoundPath::capture(&alias).unwrap();
        let direct = BoundPath::capture(&direct).unwrap();

        assert_eq!(bound.open_path(), alias);
        assert_eq!(bound.identity(), direct.identity());
    }

    #[test]
    fn relative_paths_capture_one_absolute_open_path_and_identity() {
        let relative = Path::new("lexical-component/../input.txt");
        let current_dir = env::current_dir().unwrap();
        let bound = BoundPath::capture(relative).unwrap();
        let direct = BoundPath::capture(&current_dir.join("input.txt")).unwrap();

        assert_eq!(bound.open_path(), current_dir.join(relative));
        assert_eq!(bound.identity(), direct.identity());
    }

    #[test]
    fn parent_components_cannot_escape_the_root() {
        let bound = BoundPath::capture(Path::new("/../../input.txt")).unwrap();
        let direct = BoundPath::capture(Path::new("/input.txt")).unwrap();
        assert_eq!(bound.identity(), direct.identity());
    }
}
