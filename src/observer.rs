use std::{
    fmt,
    fs::File,
    io::{self, Write as _},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    time::Instant,
};

use rustix::fs::{Mode, OFlags};

use crate::{
    document::FileIdentity,
    error::{ExternalSignal, LogError},
};

const EVENT_BUFFER_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InputKind {
    Path,
    StandardInput,
}

impl InputKind {
    const fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::StandardInput => "stdin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionOutcome {
    Normal,
    Signal(ExternalSignal),
    Error,
}

#[derive(Debug)]
pub(super) struct Observer {
    state: State,
}

#[derive(Debug)]
enum State {
    Disabled,
    Pending { path: PathBuf, started: Instant },
    Active(ActiveObserver),
    Finished,
}

#[derive(Debug)]
struct ActiveObserver {
    file: File,
    path: PathBuf,
    started: Instant,
    terminal_sessions: u64,
    suspensions: u64,
}

impl Observer {
    pub(super) fn new(path: Option<PathBuf>) -> Self {
        let state = path.map_or(State::Disabled, |path| State::Pending {
            path,
            started: Instant::now(),
        });
        Self { state }
    }

    pub(super) fn start(
        &mut self,
        input: InputKind,
        source_bytes: u64,
        input_identity: Option<FileIdentity>,
    ) -> Result<(), LogError> {
        let state = std::mem::replace(&mut self.state, State::Finished);
        let State::Pending { path, started } = state else {
            self.state = state;
            return Ok(());
        };
        let mut active = ActiveObserver::open(path, started, input_identity)?;
        active.write_schema()?;
        active.write_start(input, source_bytes)?;
        self.state = State::Active(active);
        Ok(())
    }

    pub(super) fn terminal_session(&mut self) {
        if let State::Active(active) = &mut self.state {
            active.terminal_sessions = active.terminal_sessions.saturating_add(1);
        }
    }

    pub(super) fn suspension(&mut self) {
        if let State::Active(active) = &mut self.state {
            active.suspensions = active.suspensions.saturating_add(1);
        }
    }

    pub(super) fn finish(&mut self, outcome: SessionOutcome) -> Result<(), LogError> {
        let state = std::mem::replace(&mut self.state, State::Finished);
        match state {
            State::Active(mut active) => active.write_summary(outcome),
            State::Disabled | State::Pending { .. } | State::Finished => Ok(()),
        }
    }
}

impl ActiveObserver {
    fn open(
        path: PathBuf,
        started: Instant,
        input_identity: Option<FileIdentity>,
    ) -> Result<Self, LogError> {
        if path == Path::new("-") {
            return Err(LogError::StandardStream);
        }
        let descriptor = rustix::fs::open(
            &path,
            OFlags::WRONLY
                | OFlags::APPEND
                | OFlags::CREATE
                | OFlags::CLOEXEC
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|source| LogError::Open {
            path: path.clone(),
            source: io::Error::from(source),
        })?;
        let file = File::from(descriptor);
        let metadata = file.metadata().map_err(|source| LogError::Inspect {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(LogError::NotRegular(path));
        }
        if input_identity == Some(FileIdentity::from_metadata(&metadata)) {
            return Err(LogError::InputConflict(path));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(LogError::InsecurePermissions(path));
        }
        Ok(Self {
            file,
            path,
            started,
            terminal_sessions: 0,
            suspensions: 0,
        })
    }

    fn write_schema(&mut self) -> Result<(), LogError> {
        self.write_event(format_args!("schema version=1\n"))
    }

    fn write_start(&mut self, input: InputKind, source_bytes: u64) -> Result<(), LogError> {
        let input = input.name();
        self.write_event(format_args!(
            "session_start input={input} source_bytes={source_bytes}\n"
        ))
    }

    fn write_summary(&mut self, outcome: SessionOutcome) -> Result<(), LogError> {
        let elapsed = self.started.elapsed().as_micros();
        let terminal_sessions = self.terminal_sessions;
        let suspensions = self.suspensions;
        match outcome {
            SessionOutcome::Normal => self.write_event(format_args!(
                "session_summary outcome=normal elapsed_us={elapsed} terminal_sessions={terminal_sessions} suspensions={suspensions}\n"
            )),
            SessionOutcome::Signal(signal) => {
                let signal = signal.name();
                self.write_event(format_args!(
                    "session_summary outcome=signal signal={signal} elapsed_us={elapsed} terminal_sessions={terminal_sessions} suspensions={suspensions}\n"
                ))
            }
            SessionOutcome::Error => self.write_event(format_args!(
                "session_summary outcome=error elapsed_us={elapsed} terminal_sessions={terminal_sessions} suspensions={suspensions}\n"
            )),
        }
    }

    fn write_event(&mut self, arguments: fmt::Arguments<'_>) -> Result<(), LogError> {
        let mut event = EventBuffer::new();
        fmt::write(&mut event, arguments)
            .map_err(|_| LogError::EventTooLarge(self.path.clone()))?;
        self.file
            .write_all(event.as_bytes())
            .map_err(|source| LogError::Write {
                path: self.path.clone(),
                source,
            })
    }
}

struct EventBuffer {
    bytes: [u8; EVENT_BUFFER_BYTES],
    len: usize,
}

impl EventBuffer {
    const fn new() -> Self {
        Self {
            bytes: [0; EVENT_BUFFER_BYTES],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for EventBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(fmt::Error)?;
        let target = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        target.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt as _};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn disabled_observers_have_no_files_or_events() {
        let mut observer = Observer::new(None);
        observer
            .start(InputKind::Path, 10, None)
            .expect("disabled observers start without I/O");
        observer.terminal_session();
        observer.suspension();
        observer
            .finish(SessionOutcome::Normal)
            .expect("disabled observers finish without I/O");
        assert!(matches!(observer.state, State::Disabled | State::Finished));
    }

    #[test]
    fn active_observers_append_private_ascii_session_events() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.log");
        let mut observer = Observer::new(Some(path.clone()));
        observer.start(InputKind::StandardInput, 42, None).unwrap();
        let State::Active(active) = &observer.state else {
            panic!("observer is active after starting");
        };
        assert!(
            rustix::io::fcntl_getfd(&active.file)
                .unwrap()
                .contains(rustix::io::FdFlags::CLOEXEC)
        );
        observer.terminal_session();
        observer.suspension();
        observer.terminal_session();
        observer.finish(SessionOutcome::Normal).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.is_ascii());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("schema version=1\nsession_start input=stdin source_bytes=42\n"));
        assert!(text.contains("session_summary outcome=normal elapsed_us="));
        assert!(text.ends_with(" terminal_sessions=2 suspensions=1\n"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn observer_targets_must_be_regular_and_distinct_from_the_input() {
        let directory = tempdir().unwrap();
        let mut directory_target = Observer::new(Some(directory.path().to_path_buf()));
        assert!(matches!(
            directory_target.start(InputKind::Path, 0, None),
            Err(LogError::Open { .. } | LogError::NotRegular(_))
        ));

        let input = directory.path().join("input.txt");
        fs::write(&input, "text").unwrap();
        let identity = FileIdentity::from_metadata(&fs::metadata(&input).unwrap());
        let mut conflict = Observer::new(Some(input.clone()));
        assert!(matches!(
            conflict.start(InputKind::Path, 4, Some(identity)),
            Err(LogError::InputConflict(path)) if path == input
        ));

        let hardlink = directory.path().join("hardlink.log");
        fs::hard_link(&input, &hardlink).unwrap();
        let mut linked = Observer::new(Some(hardlink.clone()));
        assert!(matches!(
            linked.start(InputKind::Path, 4, Some(identity)),
            Err(LogError::InputConflict(path)) if path == hardlink
        ));

        let symlink = directory.path().join("symlink.log");
        std::os::unix::fs::symlink(&input, &symlink).unwrap();
        let mut linked = Observer::new(Some(symlink));
        assert!(matches!(
            linked.start(InputKind::Path, 4, Some(identity)),
            Err(LogError::Open { .. })
        ));

        let mut standard_stream = Observer::new(Some(PathBuf::from("-")));
        assert!(matches!(
            standard_stream.start(InputKind::Path, 0, None),
            Err(LogError::StandardStream)
        ));
    }

    #[test]
    fn existing_logs_must_be_private_and_are_appended() {
        let directory = tempdir().unwrap();
        let insecure = directory.path().join("insecure.log");
        fs::write(&insecure, "existing\n").unwrap();
        fs::set_permissions(&insecure, fs::Permissions::from_mode(0o644)).unwrap();
        let mut observer = Observer::new(Some(insecure.clone()));
        assert!(matches!(
            observer.start(InputKind::Path, 0, None),
            Err(LogError::InsecurePermissions(path)) if path == insecure
        ));
        assert_eq!(fs::read_to_string(&insecure).unwrap(), "existing\n");

        let private = directory.path().join("private.log");
        fs::write(&private, "existing\n").unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o600)).unwrap();
        let mut observer = Observer::new(Some(private.clone()));
        observer.start(InputKind::Path, 0, None).unwrap();
        observer.finish(SessionOutcome::Normal).unwrap();
        let contents = fs::read_to_string(private).unwrap();
        assert!(contents.starts_with("existing\nschema version=1\n"));
        assert!(contents.ends_with(" terminal_sessions=0 suspensions=0\n"));
    }

    #[test]
    fn summary_write_failures_are_reported_without_dynamic_event_storage() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.log");
        let mut observer = Observer::new(Some(path.clone()));
        observer.start(InputKind::Path, 1, None).unwrap();
        let State::Active(active) = &mut observer.state else {
            panic!("observer is active after starting");
        };
        active.file = File::open(&path).unwrap();
        assert!(matches!(
            observer.finish(SessionOutcome::Error),
            Err(LogError::Write { path: failed, .. }) if failed == path
        ));
    }

    #[test]
    fn event_buffers_reject_oversized_events() {
        use fmt::Write as _;

        let mut buffer = EventBuffer::new();
        assert!(buffer.write_str(&"x".repeat(EVENT_BUFFER_BYTES)).is_ok());
        assert!(buffer.write_str("x").is_err());
    }
}
