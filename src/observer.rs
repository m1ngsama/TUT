use std::{
    fmt,
    fs::File,
    io::Write as _,
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use rustix::fs::{Mode, OFlags};

use crate::{
    app::BackgroundWork,
    document::InputIdentity,
    error::{ExternalSignal, LogError},
    path_binding::{BoundPath, ResolvedPath},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RuntimeOperation {
    Draw,
    Action,
    Background(BackgroundWork),
}

pub(super) trait RuntimeRecorder {
    type Stamp;

    fn begin_operation(&mut self) -> Self::Stamp;
    fn finish_operation(&mut self, operation: RuntimeOperation, started: Self::Stamp);
    fn event(&mut self);
    fn terminal_session(&mut self);
    fn suspension(&mut self);

    #[cfg(test)]
    fn capacity_sample(&mut self, _app: &crate::app::App) {}
}

#[derive(Debug, Default)]
pub(super) struct DisabledRecorder;

impl RuntimeRecorder for DisabledRecorder {
    type Stamp = ();

    #[inline]
    fn begin_operation(&mut self) {}

    #[inline]
    fn finish_operation(&mut self, _operation: RuntimeOperation, _started: Self::Stamp) {}

    #[inline]
    fn event(&mut self) {}

    #[inline]
    fn terminal_session(&mut self) {}

    #[inline]
    fn suspension(&mut self) {}
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Timing {
    calls: u64,
    total_us: u64,
    max_us: u64,
}

impl Timing {
    fn record(&mut self, elapsed_us: u64) {
        self.calls = self.calls.saturating_add(1);
        self.total_us = self.total_us.saturating_add(elapsed_us);
        self.max_us = self.max_us.max(elapsed_us);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SessionMetrics {
    terminal_sessions: u32,
    suspensions: u32,
    events: u64,
    draw: Timing,
    action: Timing,
    background: Timing,
    line_steps: u64,
    viewport_steps: u64,
    render_steps: u64,
    search_steps: u64,
    background_max_kind: Option<BackgroundWork>,
}

impl SessionMetrics {
    fn record(&mut self, operation: RuntimeOperation, elapsed: Duration) {
        let elapsed_us = duration_micros(elapsed);
        match operation {
            RuntimeOperation::Draw => self.draw.record(elapsed_us),
            RuntimeOperation::Action => self.action.record(elapsed_us),
            RuntimeOperation::Background(work) => {
                let new_max = self.background.calls == 0 || elapsed_us > self.background.max_us;
                self.background.record(elapsed_us);
                match work {
                    BackgroundWork::LineIndex => {
                        self.line_steps = self.line_steps.saturating_add(1);
                    }
                    BackgroundWork::Viewport => {
                        self.viewport_steps = self.viewport_steps.saturating_add(1);
                    }
                    BackgroundWork::Render => {
                        self.render_steps = self.render_steps.saturating_add(1);
                    }
                    BackgroundWork::Search => {
                        self.search_steps = self.search_steps.saturating_add(1);
                    }
                }
                if new_max {
                    self.background_max_kind = Some(work);
                }
            }
        }
    }
}

impl RuntimeRecorder for SessionMetrics {
    type Stamp = Instant;

    fn begin_operation(&mut self) -> Self::Stamp {
        Instant::now()
    }

    fn finish_operation(&mut self, operation: RuntimeOperation, started: Self::Stamp) {
        self.record(operation, started.elapsed());
    }

    fn event(&mut self) {
        self.events = self.events.saturating_add(1);
    }

    fn terminal_session(&mut self) {
        self.terminal_sessions = self.terminal_sessions.saturating_add(1);
    }

    fn suspension(&mut self) {
        self.suspensions = self.suspensions.saturating_add(1);
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
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
    metrics: SessionMetrics,
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
        input_identity: Option<InputIdentity<'_>>,
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

    pub(super) fn runtime_metrics(&mut self) -> Option<&mut SessionMetrics> {
        match &mut self.state {
            State::Active(active) => Some(&mut active.metrics),
            State::Disabled | State::Pending { .. } | State::Finished => None,
        }
    }

    pub(super) fn finish(&mut self, outcome: SessionOutcome) -> Result<(), LogError> {
        let state = std::mem::replace(&mut self.state, State::Finished);
        match state {
            State::Active(mut active) => active.write_finish(outcome),
            State::Disabled | State::Pending { .. } | State::Finished => Ok(()),
        }
    }
}

impl ActiveObserver {
    fn open(
        path: PathBuf,
        started: Instant,
        input_identity: Option<InputIdentity<'_>>,
    ) -> Result<Self, LogError> {
        if path == Path::new("-") {
            return Err(LogError::StandardStream);
        }
        let bound = BoundPath::capture(&path).map_err(|source| LogError::Open {
            path: path.clone(),
            source,
        })?;
        let resolved =
            ResolvedPath::parent(bound.open_path()).map_err(|source| LogError::Open {
                path: path.clone(),
                source,
            })?;
        if input_identity.is_some_and(|input| {
            input.pathname_matches(bound.identity(), resolved.identity())
                || input.location_matches(resolved.location())
        }) {
            return Err(LogError::InputConflict(path));
        }
        // The compared pathname slot and the log open stay anchored to this directory fd.
        let descriptor = resolved
            .open(
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
                source,
            })?;
        let file = File::from(descriptor);
        if let Some(input) = input_identity
            && input
                .current_leaf_matches(&file)
                .map_err(|source| LogError::Inspect {
                    path: path.clone(),
                    source,
                })?
        {
            return Err(LogError::InputConflict(path));
        }
        let metadata = file.metadata().map_err(|source| LogError::Inspect {
            path: path.clone(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(LogError::NotRegular(path));
        }
        if input_identity.is_some_and(|input| input.file_matches(&metadata)) {
            return Err(LogError::InputConflict(path));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(LogError::InsecurePermissions(path));
        }
        Ok(Self {
            file,
            path,
            started,
            metrics: SessionMetrics::default(),
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

    fn write_runtime_summary(&mut self) -> Result<(), LogError> {
        let metrics = self.metrics;
        let frames = metrics.draw.calls;
        let events = metrics.events;
        let actions = metrics.action.calls;
        let draw_us = metrics.draw.total_us;
        let draw_max_us = metrics.draw.max_us;
        let action_us = metrics.action.total_us;
        let action_max_us = metrics.action.max_us;
        let line_steps = metrics.line_steps;
        let viewport_steps = metrics.viewport_steps;
        let render_steps = metrics.render_steps;
        let search_steps = metrics.search_steps;
        let background_us = metrics.background.total_us;
        let background_max_us = metrics.background.max_us;
        let background_max_kind = metrics
            .background_max_kind
            .map_or("none", background_work_name);
        self.write_event(format_args!(
            "runtime_summary frames={frames} events={events} actions={actions} draw_us={draw_us} draw_max_us={draw_max_us} action_us={action_us} action_max_us={action_max_us} line_steps={line_steps} viewport_steps={viewport_steps} render_steps={render_steps} search_steps={search_steps} background_us={background_us} background_max_us={background_max_us} background_max_kind={background_max_kind}\n"
        ))
    }

    fn write_finish(&mut self, outcome: SessionOutcome) -> Result<(), LogError> {
        let mut first = None;
        retain_first(&mut first, self.write_runtime_summary());
        retain_first(&mut first, self.write_summary(outcome));
        first.map_or(Ok(()), Err)
    }

    fn write_summary(&mut self, outcome: SessionOutcome) -> Result<(), LogError> {
        let elapsed = duration_micros(self.started.elapsed());
        let terminal_sessions = self.metrics.terminal_sessions;
        let suspensions = self.metrics.suspensions;
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

fn retain_first(first: &mut Option<LogError>, result: Result<(), LogError>) {
    if let Err(error) = result
        && first.is_none()
    {
        *first = Some(error);
    }
}

const fn background_work_name(work: BackgroundWork) -> &'static str {
    match work {
        BackgroundWork::LineIndex => "line",
        BackgroundWork::Viewport => "viewport",
        BackgroundWork::Render => "render",
        BackgroundWork::Search => "search",
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
    use std::{
        fs,
        os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    };

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn disabled_observers_have_no_files_or_events() {
        let mut observer = Observer::new(None);
        observer
            .start(InputKind::Path, 10, None)
            .expect("disabled observers start without I/O");
        assert!(observer.runtime_metrics().is_none());
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
        let metrics = observer.runtime_metrics().unwrap();
        metrics.terminal_session();
        metrics.suspension();
        metrics.terminal_session();
        metrics.event();
        metrics.event();
        metrics.record(RuntimeOperation::Draw, Duration::from_micros(11));
        metrics.record(RuntimeOperation::Action, Duration::from_micros(7));
        metrics.record(
            RuntimeOperation::Background(BackgroundWork::LineIndex),
            Duration::from_micros(5),
        );
        metrics.record(
            RuntimeOperation::Background(BackgroundWork::Search),
            Duration::from_micros(9),
        );
        observer.finish(SessionOutcome::Normal).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.is_ascii());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("schema version=1\nsession_start input=stdin source_bytes=42\n"));
        assert!(text.contains(
            "runtime_summary frames=1 events=2 actions=1 draw_us=11 draw_max_us=11 action_us=7 action_max_us=7 line_steps=1 viewport_steps=0 render_steps=0 search_steps=1 background_us=14 background_max_us=9 background_max_kind=search\n"
        ));
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
        let document = crate::document::load(input.clone()).unwrap();
        let identity = document.input_identity();
        let mut conflict = Observer::new(Some(input.clone()));
        assert!(matches!(
            conflict.start(InputKind::Path, 4, identity),
            Err(LogError::InputConflict(path)) if path == input
        ));

        let hardlink = directory.path().join("hardlink.log");
        fs::hard_link(&input, &hardlink).unwrap();
        let mut linked = Observer::new(Some(hardlink.clone()));
        assert!(matches!(
            linked.start(InputKind::Path, 4, identity),
            Err(LogError::InputConflict(path)) if path == hardlink
        ));

        let symlink = directory.path().join("symlink.log");
        std::os::unix::fs::symlink(&input, &symlink).unwrap();
        let mut linked = Observer::new(Some(symlink));
        assert!(matches!(
            linked.start(InputKind::Path, 4, identity),
            Err(LogError::Open { .. })
        ));

        let mut standard_stream = Observer::new(Some(PathBuf::from("-")));
        assert!(matches!(
            standard_stream.start(InputKind::Path, 0, None),
            Err(LogError::StandardStream)
        ));
    }

    #[test]
    fn replacing_the_input_path_cannot_turn_the_replacement_into_a_log() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.txt");
        let replacement = directory.path().join("replacement.txt");
        fs::write(&input, "original input").unwrap();
        fs::write(&replacement, "replacement must remain unchanged").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        let original_metadata = fs::metadata(&input).unwrap();
        let replacement_metadata = fs::metadata(&replacement).unwrap();
        assert_ne!(
            (original_metadata.dev(), original_metadata.ino()),
            (replacement_metadata.dev(), replacement_metadata.ino())
        );

        let document = crate::document::load(input.clone()).unwrap();
        fs::rename(&replacement, &input).unwrap();
        let expected = fs::read(&input).unwrap();
        let mut observer = Observer::new(Some(input.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity()
            ),
            Err(LogError::InputConflict(path)) if path == input
        ));
        assert_eq!(fs::read(&input).unwrap(), expected);
    }

    #[test]
    fn final_symlink_input_protects_the_resolved_replacement_slot() {
        let directory = tempdir().unwrap();
        let real = directory.path().join("real");
        fs::create_dir(&real).unwrap();
        let target = real.join("input.txt");
        fs::write(&target, "original input").unwrap();
        let input = directory.path().join("input-link.txt");
        std::os::unix::fs::symlink(&target, &input).unwrap();

        assert_resolved_replacement_rejected(&input, &target);
    }

    #[test]
    fn directory_symlink_input_protects_the_resolved_replacement_slot() {
        let directory = tempdir().unwrap();
        let real = directory.path().join("real");
        fs::create_dir(&real).unwrap();
        let target = real.join("input.txt");
        fs::write(&target, "original input").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();

        assert_resolved_replacement_rejected(&alias.join("input.txt"), &target);
    }

    #[test]
    fn symlink_parent_components_protect_the_kernel_resolved_slot() {
        let directory = tempdir().unwrap();
        let real = directory.path().join("real");
        let child = real.join("child");
        fs::create_dir_all(&child).unwrap();
        let target = real.join("input.txt");
        fs::write(&target, "original input").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();

        assert_resolved_replacement_rejected(&alias.join("..").join("input.txt"), &target);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn resolved_slot_identity_preserves_non_utf8_leaf_names() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

        let directory = tempdir().unwrap();
        let leaf = OsString::from_vec(b"input-\xff.txt".to_vec());
        let target = directory.path().join(leaf);
        fs::write(&target, "original input").unwrap();
        let input = directory.path().join("input-link.txt");
        std::os::unix::fs::symlink(&target, &input).unwrap();

        assert_resolved_replacement_rejected(&input, &target);
    }

    #[test]
    fn semantic_symlink_parent_aliases_do_not_trigger_lexical_false_positives() {
        let directory = tempdir().unwrap();
        let real = directory.path().join("real");
        let child = real.join("child");
        fs::create_dir_all(&child).unwrap();
        fs::write(real.join("input.txt"), "original input").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();
        let document = crate::document::load(alias.join("..").join("input.txt")).unwrap();
        let safe_log = directory.path().join("input.txt");
        let mut observer = Observer::new(Some(safe_log.clone()));

        observer
            .start(
                InputKind::Path,
                document.content_len(),
                document.input_identity(),
            )
            .unwrap();
        observer.finish(SessionOutcome::Normal).unwrap();

        assert!(
            fs::read_to_string(safe_log)
                .unwrap()
                .starts_with("schema version=1\n")
        );
    }

    #[test]
    fn exact_unresolved_path_rebinding_remains_rejected() {
        let directory = tempdir().unwrap();
        let real = directory.path().join("real");
        let child = real.join("child");
        fs::create_dir_all(&child).unwrap();
        fs::write(real.join("input.txt"), "original input").unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&child, &alias).unwrap();
        let input = alias.join("..").join("input.txt");
        let document = crate::document::load(input.clone()).unwrap();

        fs::rename(&real, directory.path().join("moved")).unwrap();
        fs::create_dir_all(&child).unwrap();
        fs::write(real.join("input.txt"), "replacement must remain unchanged").unwrap();
        fs::set_permissions(real.join("input.txt"), fs::Permissions::from_mode(0o600)).unwrap();
        let expected = fs::read(real.join("input.txt")).unwrap();
        let mut observer = Observer::new(Some(input.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity(),
            ),
            Err(LogError::InputConflict(path)) if path == input
        ));
        assert_eq!(fs::read(real.join("input.txt")).unwrap(), expected);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn case_folded_log_aliases_cannot_modify_resolved_replacements() {
        assert_macos_path_alias_replacement_rejected("Input.txt", "input.txt");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unicode_normalized_log_aliases_cannot_modify_resolved_replacements() {
        assert_macos_path_alias_replacement_rejected("Caf\u{e9}.txt", "Cafe\u{301}.txt");
    }

    #[cfg(target_os = "macos")]
    fn assert_macos_path_alias_replacement_rejected(input_name: &str, alias_name: &str) {
        let directory = tempdir().unwrap();
        let input = directory.path().join(input_name);
        let log_alias = directory.path().join(alias_name);
        fs::write(&input, "original input").unwrap();
        let Ok(alias_metadata) = fs::metadata(&log_alias) else {
            return;
        };
        let input_metadata = fs::metadata(&input).unwrap();
        if (alias_metadata.dev(), alias_metadata.ino())
            != (input_metadata.dev(), input_metadata.ino())
        {
            return;
        }
        let document = crate::document::load(input.clone()).unwrap();
        let replacement = directory.path().join("replacement.txt");
        fs::write(&replacement, "replacement must remain unchanged").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(&replacement, &input).unwrap();
        let expected = fs::read(&input).unwrap();
        let mut observer = Observer::new(Some(log_alias.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity(),
            ),
            Err(LogError::InputConflict(path)) if path == log_alias
        ));
        assert_eq!(fs::read(input).unwrap(), expected);
    }

    #[test]
    fn missing_resolved_input_leaf_does_not_block_a_distinct_log_slot() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.txt");
        fs::write(&input, "original input").unwrap();
        let document = crate::document::load(input.clone()).unwrap();
        fs::remove_file(&input).unwrap();
        let log = directory.path().join("session.log");
        let mut observer = Observer::new(Some(log.clone()));

        observer
            .start(
                InputKind::Path,
                document.content_len(),
                document.input_identity(),
            )
            .unwrap();
        observer.finish(SessionOutcome::Normal).unwrap();

        assert!(
            fs::read_to_string(log)
                .unwrap()
                .starts_with("schema version=1\n")
        );
    }

    #[test]
    fn current_input_leaf_symlink_cannot_redirect_into_a_direct_log_target() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.txt");
        fs::write(&input, "original input").unwrap();
        let document = crate::document::load(input.clone()).unwrap();
        fs::remove_file(&input).unwrap();
        let log = directory.path().join("replacement.log");
        fs::write(&log, "replacement must remain unchanged").unwrap();
        fs::set_permissions(&log, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&log, &input).unwrap();
        let expected = fs::read(&log).unwrap();
        let mut observer = Observer::new(Some(log.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity(),
            ),
            Err(LogError::InputConflict(path)) if path == log
        ));
        assert_eq!(fs::read(log).unwrap(), expected);
    }

    #[test]
    fn cross_directory_hardlink_cannot_alias_the_current_input_replacement() {
        let directory = tempdir().unwrap();
        let input_directory = directory.path().join("input");
        let log_directory = directory.path().join("logs");
        fs::create_dir(&input_directory).unwrap();
        fs::create_dir(&log_directory).unwrap();
        let input = input_directory.join("document.txt");
        fs::write(&input, "original input").unwrap();
        let document = crate::document::load(input.clone()).unwrap();

        let replacement = input_directory.join("replacement.txt");
        fs::write(&replacement, "replacement must remain unchanged").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(&replacement, &input).unwrap();
        let log = log_directory.join("session.log");
        fs::hard_link(&input, &log).unwrap();
        let expected = fs::read(&input).unwrap();
        let mut observer = Observer::new(Some(log.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity(),
            ),
            Err(LogError::InputConflict(path)) if path == log
        ));
        assert_eq!(fs::read(input).unwrap(), expected);
    }

    #[test]
    fn removed_input_path_is_rejected_before_log_creation() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.txt");
        fs::write(&input, "original input").unwrap();
        let document = crate::document::load(input.clone()).unwrap();
        fs::remove_file(&input).unwrap();
        let mut observer = Observer::new(Some(input.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity()
            ),
            Err(LogError::InputConflict(path)) if path == input
        ));
        assert!(!input.exists());
    }

    #[test]
    fn lexical_aliases_retain_the_input_path_binding_after_replacement() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.txt");
        let alias_directory = directory.path().join("alias-component");
        let input_alias = alias_directory.join("..").join("input.txt");
        let replacement = directory.path().join("replacement.txt");
        fs::create_dir(&alias_directory).unwrap();
        fs::write(&input, "original input").unwrap();
        fs::write(&replacement, "replacement through lexical alias").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();

        let document = crate::document::load(input_alias).unwrap();
        fs::rename(&replacement, &input).unwrap();
        let expected = fs::read(&input).unwrap();
        let mut observer = Observer::new(Some(input.clone()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity()
            ),
            Err(LogError::InputConflict(path)) if path == input
        ));
        assert_eq!(fs::read(&input).unwrap(), expected);
    }

    fn assert_resolved_replacement_rejected(input: &Path, target: &Path) {
        let replacement = target.with_extension("replacement");
        fs::write(&replacement, "replacement must remain unchanged").unwrap();
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600)).unwrap();
        let document = crate::document::load(input.to_path_buf()).unwrap();
        fs::rename(&replacement, target).unwrap();
        let expected = fs::read(target).unwrap();
        let mut observer = Observer::new(Some(target.to_path_buf()));

        assert!(matches!(
            observer.start(
                InputKind::Path,
                document.content_len(),
                document.input_identity()
            ),
            Err(LogError::InputConflict(path)) if path == target
        ));
        assert_eq!(fs::read(target).unwrap(), expected);
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
    fn finish_writes_retain_the_first_error() {
        let runtime = PathBuf::from("runtime.log");
        let session = PathBuf::from("session.log");
        let mut first = None;
        retain_first(&mut first, Ok(()));
        retain_first(&mut first, Err(LogError::EventTooLarge(runtime.clone())));
        retain_first(&mut first, Err(LogError::EventTooLarge(session)));
        assert!(matches!(
            first,
            Some(LogError::EventTooLarge(path)) if path == runtime
        ));
    }

    #[test]
    fn metrics_are_fixed_size_saturating_aggregates() {
        assert_eq!(std::mem::size_of::<DisabledRecorder>(), 0);
        assert!(std::mem::size_of::<SessionMetrics>() <= 128);
        assert_eq!(
            duration_micros(Duration::new(u64::MAX, 999_999_999)),
            u64::MAX
        );

        let mut metrics = SessionMetrics {
            events: u64::MAX,
            line_steps: u64::MAX,
            draw: Timing {
                calls: u64::MAX,
                total_us: u64::MAX,
                max_us: 0,
            },
            ..SessionMetrics::default()
        };
        metrics.event();
        metrics.record(RuntimeOperation::Draw, Duration::from_micros(1));
        metrics.record(
            RuntimeOperation::Background(BackgroundWork::LineIndex),
            Duration::from_micros(5),
        );
        metrics.record(
            RuntimeOperation::Background(BackgroundWork::Search),
            Duration::from_micros(5),
        );
        metrics.render_steps = u64::MAX;
        metrics.record(
            RuntimeOperation::Background(BackgroundWork::Render),
            Duration::from_micros(4),
        );
        assert_eq!(metrics.events, u64::MAX);
        assert_eq!(metrics.draw.calls, u64::MAX);
        assert_eq!(metrics.draw.total_us, u64::MAX);
        assert_eq!(metrics.draw.max_us, 1);
        assert_eq!(metrics.line_steps, u64::MAX);
        assert_eq!(metrics.render_steps, u64::MAX);
        assert_eq!(metrics.background_max_kind, Some(BackgroundWork::LineIndex));
    }

    #[test]
    fn maximum_runtime_summary_fits_one_event_and_precedes_session_summary() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("session.log");
        let mut observer = Observer::new(Some(path.clone()));
        observer.start(InputKind::Path, u64::MAX, None).unwrap();
        let metrics = observer.runtime_metrics().unwrap();
        *metrics = SessionMetrics {
            terminal_sessions: u32::MAX,
            suspensions: u32::MAX,
            events: u64::MAX,
            draw: Timing {
                calls: u64::MAX,
                total_us: u64::MAX,
                max_us: u64::MAX,
            },
            action: Timing {
                calls: u64::MAX,
                total_us: u64::MAX,
                max_us: u64::MAX,
            },
            background: Timing {
                calls: u64::MAX,
                total_us: u64::MAX,
                max_us: u64::MAX,
            },
            line_steps: u64::MAX,
            viewport_steps: u64::MAX,
            render_steps: u64::MAX,
            search_steps: u64::MAX,
            background_max_kind: Some(BackgroundWork::Viewport),
        };
        observer.finish(SessionOutcome::Normal).unwrap();

        let contents = fs::read_to_string(path).unwrap();
        let mut lines = contents.lines();
        assert_eq!(lines.next(), Some("schema version=1"));
        assert_eq!(
            lines.next(),
            Some("session_start input=path source_bytes=18446744073709551615")
        );
        let runtime = lines.next().unwrap();
        assert!(runtime.starts_with("runtime_summary "));
        assert_eq!(runtime.len() + 1, 468);
        assert!(runtime.is_ascii());
        assert!(lines.next().unwrap().starts_with("session_summary "));
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn event_buffers_reject_oversized_events() {
        use fmt::Write as _;

        let mut buffer = EventBuffer::new();
        assert!(buffer.write_str(&"x".repeat(EVENT_BUFFER_BYTES)).is_ok());
        assert!(buffer.write_str("x").is_err());
    }
}
