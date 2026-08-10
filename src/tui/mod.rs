mod input;
mod view;

use std::{
    io::{self, Stdout},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use signal_hook::{SigId, low_level};

use crate::{
    app::{Action, App, Geometry, Outcome},
    error::{ExternalSignal, RunOutcome, TutError},
    observer::Observer,
};

const MAX_POLL: Duration = Duration::from_millis(100);
const BACKGROUND_POLL: Duration = Duration::from_millis(1);

#[derive(Default)]
struct PendingSignals {
    termination: AtomicUsize,
    suspend: AtomicBool,
}

#[derive(Clone, Default)]
pub(super) struct SignalState(Arc<PendingSignals>);

impl SignalState {
    fn empty() -> Self {
        Self::default()
    }

    pub(super) fn received(&self) -> Option<ExternalSignal> {
        match self.0.termination.load(Ordering::SeqCst) {
            signal if signal == signal_hook::consts::signal::SIGHUP as usize => {
                Some(ExternalSignal::Hangup)
            }
            signal if signal == signal_hook::consts::signal::SIGINT as usize => {
                Some(ExternalSignal::Interrupt)
            }
            signal if signal == signal_hook::consts::signal::SIGQUIT as usize => {
                Some(ExternalSignal::Quit)
            }
            signal if signal == signal_hook::consts::signal::SIGTERM as usize => {
                Some(ExternalSignal::Terminate)
            }
            _ => None,
        }
    }

    fn suspend_requested(&self) -> bool {
        self.0.suspend.load(Ordering::SeqCst)
    }

    fn take_suspend(&self) -> bool {
        self.0.suspend.swap(false, Ordering::SeqCst)
    }

    #[cfg(test)]
    fn store_raw(&self, signal: usize) {
        let _ = self
            .0
            .termination
            .compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn store_suspend(&self) {
        self.0.suspend.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn store_continue(&self) {
        self.0.suspend.store(false, Ordering::SeqCst);
    }
}

pub(super) struct SignalHandlers {
    state: SignalState,
    ids: Vec<SigId>,
}

impl SignalHandlers {
    fn install() -> io::Result<Self> {
        use signal_hook::consts::signal::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP};

        let state = SignalState::empty();
        let mut ids = Vec::new();
        for (signal, value) in [
            (SIGHUP, SIGHUP as usize),
            (SIGINT, SIGINT as usize),
            (SIGQUIT, SIGQUIT as usize),
            (SIGTERM, SIGTERM as usize),
        ] {
            let pending = Arc::clone(&state.0);
            // SAFETY: The handler performs only a non-panicking atomic compare-exchange.
            let registration = unsafe {
                low_level::register(signal, move || {
                    let _ = pending.termination.compare_exchange(
                        0,
                        value,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    );
                })
            };
            match registration {
                Ok(id) => ids.push(id),
                Err(error) => {
                    for id in ids {
                        low_level::unregister(id);
                    }
                    return Err(error);
                }
            }
        }
        for (signal, suspended) in [(SIGTSTP, true), (SIGCONT, false)] {
            let pending = Arc::clone(&state.0);
            // SAFETY: The handler performs only a non-panicking atomic store.
            let registration = unsafe {
                low_level::register(signal, move || {
                    pending.suspend.store(suspended, Ordering::SeqCst);
                })
            };
            match registration {
                Ok(id) => ids.push(id),
                Err(error) => {
                    for id in ids {
                        low_level::unregister(id);
                    }
                    return Err(error);
                }
            }
        }
        Ok(Self { state, ids })
    }

    pub(super) fn state(&self) -> &SignalState {
        &self.state
    }
}

impl Drop for SignalHandlers {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            low_level::unregister(id);
        }
    }
}

pub(super) fn install_signal_handlers() -> Result<SignalHandlers, TutError> {
    SignalHandlers::install().map_err(|source| TutError::Io {
        operation: "install signal handlers",
        source,
    })
}

trait TerminalDriver {
    fn size(&mut self) -> io::Result<(u16, u16)>;
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn enter_alternate_screen(&mut self) -> io::Result<()>;
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn draw(&mut self, app: &mut App) -> Result<(), TutError>;
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
    fn force_redraw(&mut self) -> io::Result<()>;
    fn suspend(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
}

struct TerminalSession<'a, T: TerminalDriver> {
    driver: &'a mut T,
    raw_cleanup: bool,
    alternate_cleanup: bool,
    cursor_cleanup: bool,
}

impl<'a, T: TerminalDriver> TerminalSession<'a, T> {
    fn new(driver: &'a mut T) -> Self {
        Self {
            driver,
            raw_cleanup: false,
            alternate_cleanup: false,
            cursor_cleanup: false,
        }
    }

    fn initialize(&mut self, signals: &SignalState) -> Result<(), Primary> {
        check_control(signals)?;

        self.raw_cleanup = true;
        let result = self.driver.enable_raw_mode();
        check_control(signals)?;
        result.map_err(|source| {
            Primary::Error(TutError::Io {
                operation: "enable raw mode",
                source,
            })
        })?;

        self.alternate_cleanup = true;
        let result = self.driver.enter_alternate_screen();
        check_control(signals)?;
        result.map_err(|source| {
            Primary::Error(TutError::Io {
                operation: "enter alternate screen",
                source,
            })
        })?;

        self.cursor_cleanup = true;
        let result = self.driver.hide_cursor();
        check_control(signals)?;
        result.map_err(|source| {
            Primary::Error(TutError::Io {
                operation: "hide cursor",
                source,
            })
        })?;

        Ok(())
    }

    fn restore(&mut self) -> Option<TutError> {
        let mut first = None;
        if self.cursor_cleanup {
            self.cursor_cleanup = false;
            retain_first(&mut first, "show cursor", self.driver.show_cursor());
        }
        if self.alternate_cleanup {
            self.alternate_cleanup = false;
            retain_first(
                &mut first,
                "leave alternate screen",
                self.driver.leave_alternate_screen(),
            );
        }
        if self.raw_cleanup {
            self.raw_cleanup = false;
            retain_first(
                &mut first,
                "disable raw mode",
                self.driver.disable_raw_mode(),
            );
        }
        first
    }
}

impl<T: TerminalDriver> Drop for TerminalSession<'_, T> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[derive(Debug)]
enum Primary {
    Normal,
    Suspend,
    Signal(ExternalSignal),
    Error(TutError),
}

fn check_control(signals: &SignalState) -> Result<(), Primary> {
    if let Some(signal) = signals.received() {
        return Err(Primary::Signal(signal));
    }
    if signals.suspend_requested() {
        return Err(Primary::Suspend);
    }
    Ok(())
}

fn retain_first(first: &mut Option<TutError>, operation: &'static str, result: io::Result<()>) {
    if let Err(source) = result
        && first.is_none()
    {
        *first = Some(TutError::Io { operation, source });
    }
}

fn promote_termination(primary: Primary, signals: &SignalState) -> Primary {
    match (primary, signals.received()) {
        (Primary::Normal | Primary::Suspend, Some(signal)) => Primary::Signal(signal),
        (primary, _) => primary,
    }
}

fn finish(primary: Primary, restoration: Option<TutError>) -> Result<RunOutcome, TutError> {
    match (primary, restoration) {
        (Primary::Normal, None) => Ok(RunOutcome::Normal),
        (Primary::Signal(signal), None) => Ok(RunOutcome::Signal(signal)),
        (Primary::Error(error), None) => Err(error),
        (Primary::Normal, Some(restoration)) => Err(restoration),
        (Primary::Signal(signal), Some(restoration)) => Err(TutError::SignalAndRestoration {
            signal,
            restoration: Box::new(restoration),
        }),
        (Primary::Error(primary), Some(restoration)) => Err(TutError::PrimaryAndRestoration {
            primary: Box::new(primary),
            restoration: Box::new(restoration),
        }),
        (Primary::Suspend, _) => unreachable!("suspension is handled before finishing"),
    }
}

fn refresh_geometry<T: TerminalDriver>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
) -> Result<(), Primary> {
    check_control(signals)?;
    let size_result = driver.size();
    check_control(signals)?;
    let (width, height) = size_result.map_err(|source| {
        Primary::Error(TutError::Io {
            operation: "query terminal size",
            source,
        })
    })?;
    let resize_result = app.update(Action::Resize(Geometry::new(width, height)));
    check_control(signals)?;
    resize_result.map_err(Primary::Error)?;
    Ok(())
}

fn run_session<T: TerminalDriver>(
    app: &mut App,
    session: &mut TerminalSession<'_, T>,
    signals: &SignalState,
    force_redraw: bool,
    observer: &mut Observer,
) -> Primary {
    if let Err(primary) = refresh_geometry(app, session.driver, signals) {
        return primary;
    }
    if let Err(primary) = session.initialize(signals) {
        return primary;
    }
    observer.terminal_session();
    if force_redraw {
        let result = session.driver.force_redraw();
        if let Err(primary) = check_control(signals) {
            return primary;
        }
        if let Err(source) = result {
            return Primary::Error(TutError::Io {
                operation: "reset terminal for redraw",
                source,
            });
        }
    }
    event_loop(app, session.driver, signals)
}

fn run_with_observer<T: TerminalDriver>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
    observer: &mut Observer,
) -> Result<RunOutcome, TutError> {
    let mut session = TerminalSession::new(driver);
    let mut resumed = false;
    loop {
        let primary = run_session(app, &mut session, signals, resumed, observer);
        let restoration = session.restore();
        let primary = promote_termination(primary, signals);
        if !matches!(primary, Primary::Suspend) {
            return finish(primary, restoration);
        }
        if let Some(restoration) = restoration {
            return Err(restoration);
        }
        if !signals.take_suspend() {
            resumed = true;
            continue;
        }
        if let Some(signal) = signals.received() {
            return Ok(RunOutcome::Signal(signal));
        }
        let result = session.driver.suspend();
        if let Some(signal) = signals.received() {
            return Ok(RunOutcome::Signal(signal));
        }
        result.map_err(|source| TutError::Io {
            operation: "suspend process",
            source,
        })?;
        observer.suspension();
        resumed = true;
    }
}

#[cfg(test)]
fn run_with_driver<T: TerminalDriver>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
) -> Result<RunOutcome, TutError> {
    let mut observer = Observer::new(None);
    run_with_observer(app, driver, signals, &mut observer)
}

fn event_loop<T: TerminalDriver>(app: &mut App, driver: &mut T, signals: &SignalState) -> Primary {
    let mut redraw = true;

    loop {
        if let Err(primary) = check_control(signals) {
            return primary;
        }
        if redraw {
            let result = driver.draw(app);
            if let Err(primary) = check_control(signals) {
                return primary;
            }
            if let Err(error) = result {
                return Primary::Error(error);
            }
            redraw = false;
        }

        let background_work = app.has_background_work();
        let timeout = if background_work {
            BACKGROUND_POLL
        } else {
            MAX_POLL
        };
        let ready = match driver.poll(timeout) {
            Ok(ready) => ready,
            Err(source) => {
                if let Err(primary) = check_control(signals) {
                    return primary;
                }
                return Primary::Error(TutError::Io {
                    operation: "poll terminal events",
                    source,
                });
            }
        };
        if let Err(primary) = check_control(signals) {
            return primary;
        }
        if !ready {
            if background_work {
                match advance_background(app, signals) {
                    Ok(changed) => redraw |= changed,
                    Err(primary) => return primary,
                }
            }
            continue;
        }

        let event = match driver.read() {
            Ok(event) => event,
            Err(source) => {
                if let Err(primary) = check_control(signals) {
                    return primary;
                }
                return Primary::Error(TutError::Io {
                    operation: "read terminal event",
                    source,
                });
            }
        };
        if let Err(primary) = check_control(signals) {
            return primary;
        }

        if let Some(action) = input::map_event(app.mode(), app.terminal_too_small(), event) {
            match app.update(action) {
                Ok(Outcome::Changed) => redraw = true,
                Ok(Outcome::Unchanged) => {}
                Ok(Outcome::Quit) => return Primary::Normal,
                Err(error) => return Primary::Error(error),
            }
        }
        if app.has_background_work() {
            match advance_background(app, signals) {
                Ok(changed) => redraw |= changed,
                Err(primary) => return primary,
            }
        }
    }
}

fn advance_background(app: &mut App, signals: &SignalState) -> Result<bool, Primary> {
    let result = app.advance_background();
    check_control(signals)?;
    result.map_err(Primary::Error)
}

struct CrosstermDriver {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl CrosstermDriver {
    fn new() -> io::Result<Self> {
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(io::stdout()))?,
        })
    }
}

impl TerminalDriver for CrosstermDriver {
    fn size(&mut self) -> io::Result<(u16, u16)> {
        let size = self.terminal.size()?;
        Ok((size.width, size.height))
    }

    fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn enter_alternate_screen(&mut self) -> io::Result<()> {
        execute!(self.terminal.backend_mut(), EnterAlternateScreen)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(self.terminal.backend_mut(), Hide)
    }

    fn draw(&mut self, app: &mut App) -> Result<(), TutError> {
        let state = app.render_state()?;
        let mut view_result = Ok(());
        self.terminal
            .draw(|frame| {
                view_result = view::render(frame, &state);
            })
            .map_err(|source| TutError::Io {
                operation: "draw terminal frame",
                source,
            })?;
        view_result
    }

    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }

    fn force_redraw(&mut self) -> io::Result<()> {
        let size = self.terminal.size()?;
        self.terminal.resize(size.into())
    }

    fn suspend(&mut self) -> io::Result<()> {
        low_level::emulate_default_handler(signal_hook::consts::signal::SIGTSTP)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(self.terminal.backend_mut(), Show)
    }

    fn leave_alternate_screen(&mut self) -> io::Result<()> {
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
    }

    fn disable_raw_mode(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }
}

pub(super) fn run(
    app: &mut App,
    signals: &SignalState,
    observer: &mut Observer,
) -> Result<RunOutcome, TutError> {
    let mut driver = match CrosstermDriver::new() {
        Ok(driver) => driver,
        Err(source) => {
            if let Some(signal) = signals.received() {
                return Ok(RunOutcome::Signal(signal));
            }
            return Err(TutError::Io {
                operation: "initialize terminal backend",
                source,
            });
        }
    };
    run_with_observer(app, &mut driver, signals, observer)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, fs, path::Path};

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use signal_hook::consts::signal::{SIGHUP, SIGTERM};
    use tempfile::tempdir;

    use super::*;
    use crate::app::app_from_text;

    struct FakeDriver {
        calls: Vec<&'static str>,
        failures: Vec<&'static str>,
        events: VecDeque<Event>,
        inject_on: Option<&'static str>,
        inject_suspend_on: Option<&'static str>,
        inject_continue_on: Option<&'static str>,
        signals: SignalState,
        poll_timeouts: Vec<Duration>,
    }

    impl FakeDriver {
        fn new(signals: &SignalState) -> Self {
            Self {
                calls: Vec::new(),
                failures: Vec::new(),
                events: VecDeque::new(),
                inject_on: None,
                inject_suspend_on: None,
                inject_continue_on: None,
                signals: signals.clone(),
                poll_timeouts: Vec::new(),
            }
        }

        fn call(&mut self, name: &'static str) -> io::Result<()> {
            self.calls.push(name);
            if self.inject_on == Some(name) {
                self.signals.store_raw(SIGTERM as usize);
            }
            if self.inject_suspend_on == Some(name) {
                self.inject_suspend_on = None;
                self.signals.store_suspend();
            }
            if self.inject_continue_on == Some(name) {
                self.inject_continue_on = None;
                self.signals.store_continue();
            }
            if self.failures.contains(&name) {
                Err(io::Error::other(name))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalDriver for FakeDriver {
        fn size(&mut self) -> io::Result<(u16, u16)> {
            self.call("size")?;
            Ok((20, 4))
        }

        fn enable_raw_mode(&mut self) -> io::Result<()> {
            self.call("enable_raw")
        }

        fn enter_alternate_screen(&mut self) -> io::Result<()> {
            self.call("enter_alt")
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            self.call("hide_cursor")
        }

        fn draw(&mut self, _app: &mut App) -> Result<(), TutError> {
            self.call("draw").map_err(|source| TutError::Io {
                operation: "draw terminal frame",
                source,
            })
        }

        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.poll_timeouts.push(timeout);
            self.call("poll")?;
            Ok(!self.events.is_empty())
        }

        fn read(&mut self) -> io::Result<Event> {
            self.call("read")?;
            self.events
                .pop_front()
                .ok_or_else(|| io::Error::other("empty event queue"))
        }

        fn force_redraw(&mut self) -> io::Result<()> {
            self.call("force_redraw")
        }

        fn suspend(&mut self) -> io::Result<()> {
            self.call("suspend")
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            self.call("show_cursor")
        }

        fn leave_alternate_screen(&mut self) -> io::Result<()> {
            self.call("leave_alt")
        }

        fn disable_raw_mode(&mut self) -> io::Result<()> {
            self.call("disable_raw")
        }
    }

    fn quit_event() -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn app() -> App {
        app_from_text(Path::new("/tmp/book.txt"), "body".to_owned())
    }

    #[test]
    fn partial_initialization_restores_every_marked_step_once() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.failures.push("enter_alt");
        let mut session = TerminalSession::new(&mut driver);
        assert!(session.initialize(&signals).is_err());
        assert!(session.restore().is_none());
        drop(session);
        assert_eq!(
            driver.calls,
            vec!["enable_raw", "enter_alt", "leave_alt", "disable_raw"]
        );
    }

    #[test]
    fn cleanup_keeps_first_error_and_attempts_later_steps() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        let mut session = TerminalSession::new(&mut driver);
        session.initialize(&signals).unwrap();
        session.driver.failures = vec!["show_cursor", "leave_alt", "disable_raw"];
        let error = session.restore().unwrap();
        assert_eq!(error.message(), "failed to show cursor: show_cursor");
        drop(session);
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn normal_quit_restores_terminal_in_reverse_order() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn suspension_restores_then_reinitializes_and_forces_a_redraw() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");

        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        let transition = [
            "poll",
            "show_cursor",
            "leave_alt",
            "disable_raw",
            "suspend",
            "size",
            "enable_raw",
            "enter_alt",
            "hide_cursor",
            "force_redraw",
            "draw",
        ];
        assert!(
            driver
                .calls
                .windows(transition.len())
                .any(|calls| calls == transition)
        );
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn failed_suspension_restoration_does_not_stop_the_process() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        driver.failures.push("show_cursor");

        let error = run_with_driver(&mut app(), &mut driver, &signals).unwrap_err();
        assert_eq!(error.message(), "failed to show cursor: show_cursor");
        assert!(!driver.calls.contains(&"suspend"));
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn continuation_during_restoration_cancels_the_pending_stop() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        driver.inject_continue_on = Some("show_cursor");

        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert!(!driver.calls.contains(&"suspend"));
        assert!(driver.calls.contains(&"force_redraw"));
    }

    #[test]
    fn terminating_signal_during_suspension_restoration_prevents_the_stop() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        driver.inject_on = Some("show_cursor");

        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert!(!driver.calls.contains(&"suspend"));
    }

    #[test]
    fn signal_before_size_returns_without_terminal_calls() {
        let signals = SignalState::empty();
        signals.store_raw(SIGTERM as usize);
        signals.store_raw(SIGHUP as usize);
        let mut driver = FakeDriver::new(&signals);
        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert!(driver.calls.is_empty());
    }

    #[test]
    fn signal_during_restoration_promotes_normal_quit() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_on = Some("show_cursor");
        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
    }

    #[test]
    fn signal_preempts_incremental_index_work() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("large.txt");
        fs::write(&path, "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.inject_on = Some("poll");

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert!(app.has_background_work());
        assert_eq!(driver.poll_timeouts, vec![BACKGROUND_POLL]);
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn terminal_events_do_not_starve_incremental_index_work() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("large.txt");
        fs::write(&path, "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver
            .events
            .extend([Event::FocusGained, Event::FocusGained, quit_event()]);

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert!(!app.has_background_work());
    }

    #[test]
    fn signal_and_restoration_failure_are_combined() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_on = Some("show_cursor");
        driver.failures.push("show_cursor");
        let error = run_with_driver(&mut app(), &mut driver, &signals).unwrap_err();
        assert!(matches!(error, TutError::SignalAndRestoration { .. }));
        assert_eq!(error.exit_code(), 1);
    }
}
