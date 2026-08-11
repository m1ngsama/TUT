mod input;
mod signals;
mod view;

use std::{
    io::{self, Stdout},
    mem::size_of,
    time::Duration,
};

use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
        is_raw_mode_enabled,
    },
};
use ratatui::{
    Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, buffer::Cell, layout::Rect,
};
use signals::SuspendOutcome;
pub(super) use signals::{ProcessSessionLease, SignalHandlers, SignalState};

use crate::{
    app::{Action, App, BackgroundWork, Geometry, Outcome},
    error::{ExternalSignal, RunOutcome, TutError},
    observer::{DisabledRecorder, Observer, RuntimeOperation, RuntimeRecorder},
};

const MAX_POLL: Duration = Duration::from_millis(100);
const BACKGROUND_POLL: Duration = Duration::ZERO;
const MAX_TERMINAL_CELLS: u64 = 512 * 1024;
const MAX_TERMINAL_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TerminalSize {
    width: u16,
    height: u16,
}

impl TerminalSize {
    fn new(width: u16, height: u16) -> Result<Self, TutError> {
        let cells = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or_else(|| terminal_size_error(width, height))?;
        let cell_bytes = u64::try_from(size_of::<Cell>())
            .expect("terminal cell sizes fit unsigned 64-bit integers");
        let buffer_bytes = cells
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_mul(cell_bytes))
            .ok_or_else(|| terminal_size_error(width, height))?;
        if cells > MAX_TERMINAL_CELLS || buffer_bytes > MAX_TERMINAL_BUFFER_BYTES {
            return Err(terminal_size_error(width, height));
        }
        Ok(Self { width, height })
    }

    const fn geometry(self) -> Geometry {
        Geometry::new(self.width, self.height)
    }

    const fn area(self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    #[cfg(test)]
    fn buffer_bytes(self) -> u64 {
        u64::from(self.width)
            * u64::from(self.height)
            * 2
            * u64::try_from(size_of::<Cell>())
                .expect("terminal cell sizes fit unsigned 64-bit integers")
    }
}

fn terminal_cell_limit() -> u64 {
    let cell_bytes =
        u64::try_from(size_of::<Cell>()).expect("terminal cell sizes fit unsigned 64-bit integers");
    MAX_TERMINAL_CELLS.min(MAX_TERMINAL_BUFFER_BYTES / (2 * cell_bytes))
}

fn terminal_size_error(columns: u16, rows: u16) -> TutError {
    TutError::TerminalTooLarge {
        columns,
        rows,
        cell_limit: terminal_cell_limit(),
    }
}

pub(super) fn acquire_session() -> Result<ProcessSessionLease, TutError> {
    ProcessSessionLease::acquire().ok_or(TutError::Busy)
}

pub(super) fn install_signal_handlers(
    lease: ProcessSessionLease,
) -> Result<SignalHandlers, TutError> {
    SignalHandlers::install(lease).map_err(|source| TutError::Io {
        operation: "install signal handlers",
        source,
    })
}

trait TerminalDriver {
    fn raw_mode_enabled(&mut self) -> io::Result<bool>;
    fn size(&mut self) -> io::Result<(u16, u16)>;
    fn resize(&mut self, size: TerminalSize) -> io::Result<()>;
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn enter_alternate_screen(&mut self) -> io::Result<()>;
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn draw(&mut self, app: &mut App) -> Result<(), TutError>;
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
    fn suspend(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
}

trait SuspendControl {
    fn suspend<F>(&mut self, stop: F) -> io::Result<SuspendOutcome>
    where
        F: FnOnce() -> io::Result<()>;
}

impl SuspendControl for SignalHandlers {
    fn suspend<F>(&mut self, stop: F) -> io::Result<SuspendOutcome>
    where
        F: FnOnce() -> io::Result<()>,
    {
        SignalHandlers::suspend(self, stop)
    }
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

fn preflight_terminal<T: TerminalDriver>(
    driver: &mut T,
    signals: &SignalState,
) -> Result<(), Primary> {
    check_control(signals)?;
    let result = driver.raw_mode_enabled();
    check_control(signals)?;
    let enabled = result.map_err(|source| {
        Primary::Error(TutError::Io {
            operation: "query raw mode",
            source,
        })
    })?;
    if enabled {
        return Err(Primary::Error(TutError::TerminalInUse));
    }
    Ok(())
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
) -> Result<TerminalSize, Primary> {
    check_control(signals)?;
    let size_result = driver.size();
    check_control(signals)?;
    let (width, height) = size_result.map_err(|source| {
        Primary::Error(TutError::Io {
            operation: "query terminal size",
            source,
        })
    })?;
    let validated = TerminalSize::new(width, height);
    check_control(signals)?;
    let size = validated.map_err(Primary::Error)?;
    let resize_result = app.update(Action::Resize(size.geometry()));
    check_control(signals)?;
    resize_result.map_err(Primary::Error)?;
    Ok(size)
}

fn resize_terminal<T: TerminalDriver>(
    driver: &mut T,
    size: TerminalSize,
    signals: &SignalState,
) -> Result<(), Primary> {
    check_control(signals)?;
    let result = driver.resize(size);
    check_control(signals)?;
    result.map_err(|source| {
        Primary::Error(TutError::Io {
            operation: "resize terminal viewport",
            source,
        })
    })
}

fn run_session<T: TerminalDriver, R: RuntimeRecorder>(
    app: &mut App,
    session: &mut TerminalSession<'_, T>,
    signals: &SignalState,
    recorder: &mut R,
) -> Primary {
    if let Err(primary) = preflight_terminal(session.driver, signals) {
        return primary;
    }
    let size = match refresh_geometry(app, session.driver, signals) {
        Ok(size) => size,
        Err(primary) => return primary,
    };
    if let Err(primary) = session.initialize(signals) {
        return primary;
    }
    recorder.terminal_session();
    if let Err(primary) = resize_terminal(session.driver, size, signals) {
        return primary;
    }
    event_loop(app, session.driver, signals, recorder)
}

fn run_with_recorder<T: TerminalDriver, R: RuntimeRecorder, S: SuspendControl>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
    suspend: &mut S,
    recorder: &mut R,
) -> Result<RunOutcome, TutError> {
    let mut session = TerminalSession::new(driver);
    loop {
        let primary = run_session(app, &mut session, signals, recorder);
        let restoration = session.restore();
        let primary = promote_termination(primary, signals);
        if !matches!(primary, Primary::Suspend) {
            return finish(primary, restoration);
        }
        if let Some(restoration) = restoration {
            return Err(restoration);
        }
        if let Some(signal) = signals.received() {
            return Ok(RunOutcome::Signal(signal));
        }
        let result = suspend.suspend(|| session.driver.suspend());
        if let Some(signal) = signals.received() {
            return Ok(RunOutcome::Signal(signal));
        }
        let outcome = result.map_err(|source| TutError::Io {
            operation: "suspend process",
            source,
        })?;
        if outcome == SuspendOutcome::Cancelled {
            continue;
        }
        recorder.suspension();
    }
}

#[cfg(test)]
struct ImmediateSuspendControl {
    signals: SignalState,
}

#[cfg(test)]
impl SuspendControl for ImmediateSuspendControl {
    fn suspend<F>(&mut self, stop: F) -> io::Result<SuspendOutcome>
    where
        F: FnOnce() -> io::Result<()>,
    {
        if !self.signals.take_suspend() {
            return Ok(SuspendOutcome::Cancelled);
        }
        stop()?;
        Ok(SuspendOutcome::Continued)
    }
}

#[cfg(test)]
fn run_with_driver<T: TerminalDriver>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
) -> Result<RunOutcome, TutError> {
    let mut recorder = DisabledRecorder;
    let mut suspend = ImmediateSuspendControl {
        signals: signals.clone(),
    };
    run_with_recorder(app, driver, signals, &mut suspend, &mut recorder)
}

fn event_loop<T: TerminalDriver, R: RuntimeRecorder>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
    recorder: &mut R,
) -> Primary {
    let mut redraw = true;
    // Pending is a static shell. Background phases may request redraws while it stays visually
    // identical, so only input, resize, or a completed frame starts another pending episode.
    let mut pending_drawn = false;

    loop {
        if let Err(primary) = check_control(signals) {
            return primary;
        }
        if redraw && app.frame_ready() {
            if let Err(primary) = draw_frame(app, driver, signals, recorder) {
                return primary;
            }
            redraw = false;
            pending_drawn = false;
        }

        let background_work = app.background_work();
        let timeout = if background_work.is_some() {
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
            if redraw && !app.frame_ready() && !pending_drawn {
                if let Err(primary) = draw_frame(app, driver, signals, recorder) {
                    return primary;
                }
                redraw = false;
                pending_drawn = true;
            }
            if let Some(work) = background_work {
                match advance_background(app, signals, recorder, work) {
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
        recorder.event();
        if let Err(primary) = check_control(signals) {
            return primary;
        }

        let mut input_changed = false;
        if let Event::Resize(width, height) = event {
            let validated = TerminalSize::new(width, height);
            if let Err(primary) = check_control(signals) {
                return primary;
            }
            let size = match validated {
                Ok(size) => size,
                Err(error) => return Primary::Error(error),
            };
            let started = recorder.begin_operation();
            let result = app.update(Action::Resize(size.geometry()));
            recorder.finish_operation(RuntimeOperation::Action, started);
            if let Err(primary) = check_control(signals) {
                return primary;
            }
            match result {
                Ok(Outcome::Changed | Outcome::Unchanged) => {
                    if let Err(primary) = resize_terminal(driver, size, signals) {
                        return primary;
                    }
                    redraw = true;
                    pending_drawn = false;
                    input_changed = true;
                }
                Ok(Outcome::Interrupt) => return Primary::Signal(ExternalSignal::Interrupt),
                Ok(Outcome::Quit) => return Primary::Normal,
                Err(error) => return Primary::Error(error),
            }
        } else if let Some(action) = input::map_event(
            app.mode(),
            app.repeat_active(),
            app.terminal_too_small(),
            event,
        ) {
            let started = recorder.begin_operation();
            let result = app.update(action);
            recorder.finish_operation(RuntimeOperation::Action, started);
            match result {
                Ok(Outcome::Changed) => {
                    redraw = true;
                    pending_drawn = false;
                    input_changed = true;
                }
                Ok(Outcome::Unchanged) => {}
                Ok(Outcome::Interrupt) => return Primary::Signal(ExternalSignal::Interrupt),
                Ok(Outcome::Quit) => return Primary::Normal,
                Err(error) => return Primary::Error(error),
            }
        }
        if input_changed && redraw && !app.frame_ready() && !pending_drawn {
            if let Err(primary) = draw_frame(app, driver, signals, recorder) {
                return primary;
            }
            redraw = false;
            pending_drawn = true;
        }
        if let Some(work) = app.background_work() {
            match advance_background(app, signals, recorder, work) {
                Ok(changed) => redraw |= changed,
                Err(primary) => return primary,
            }
        }
    }
}

fn draw_frame<T: TerminalDriver, R: RuntimeRecorder>(
    app: &mut App,
    driver: &mut T,
    signals: &SignalState,
    recorder: &mut R,
) -> Result<(), Primary> {
    check_control(signals)?;
    let started = recorder.begin_operation();
    let result = driver.draw(app);
    recorder.finish_operation(RuntimeOperation::Draw, started);
    check_control(signals)?;
    result.map_err(Primary::Error)
}

fn advance_background<R: RuntimeRecorder>(
    app: &mut App,
    signals: &SignalState,
    recorder: &mut R,
    work: BackgroundWork,
) -> Result<bool, Primary> {
    check_control(signals)?;
    let started = recorder.begin_operation();
    let result = app.advance_background();
    recorder.finish_operation(RuntimeOperation::Background(work), started);
    check_control(signals)?;
    result.map_err(Primary::Error)
}

struct CrosstermDriver {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl CrosstermDriver {
    fn new() -> io::Result<Self> {
        let options = TerminalOptions {
            viewport: Viewport::Fixed(Rect::default()),
        };
        Ok(Self {
            terminal: Terminal::with_options(CrosstermBackend::new(io::stdout()), options)?,
        })
    }
}

impl TerminalDriver for CrosstermDriver {
    fn raw_mode_enabled(&mut self) -> io::Result<bool> {
        is_raw_mode_enabled()
    }

    fn size(&mut self) -> io::Result<(u16, u16)> {
        let size = self.terminal.size()?;
        Ok((size.width, size.height))
    }

    fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
        self.terminal.resize(size.area())
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
        let state = app.view_state()?;
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

    fn suspend(&mut self) -> io::Result<()> {
        // SAFETY: SIGSTOP is valid and raise has no Rust memory-safety requirements.
        let result = unsafe { libc::raise(libc::SIGSTOP) };
        if result == 0 {
            Ok(())
        } else if result == -1 {
            Err(io::Error::last_os_error())
        } else {
            Err(io::Error::from_raw_os_error(result))
        }
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
    handlers: &mut SignalHandlers,
    observer: &mut Observer,
) -> Result<RunOutcome, TutError> {
    let signals = handlers.state().clone();
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
    if let Some(metrics) = observer.runtime_metrics() {
        run_with_recorder(app, &mut driver, &signals, handlers, metrics)
    } else {
        let mut recorder = DisabledRecorder;
        run_with_recorder(app, &mut driver, &signals, handlers, &mut recorder)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell as StateCell, collections::VecDeque, fs, path::Path, rc::Rc};

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use libc::{SIGHUP, SIGTERM};
    use ratatui::backend::TestBackend;
    use tempfile::tempdir;

    use super::*;
    use crate::app::{ViewState, app_from_text};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DrawKind {
        Pending,
        Reader,
        Help,
    }

    struct FakeDriver {
        calls: Vec<&'static str>,
        failures: Vec<&'static str>,
        raw_modes: VecDeque<bool>,
        sizes: VecDeque<(u16, u16)>,
        resizes: Vec<TerminalSize>,
        events: VecDeque<Event>,
        inject_on: Option<&'static str>,
        inject_suspend_on: Option<&'static str>,
        inject_continue_on: Option<&'static str>,
        signals: SignalState,
        poll_timeouts: Vec<Duration>,
        signal_handlers_active: Option<Rc<StateCell<bool>>>,
        draws: Vec<DrawKind>,
        quit_after_reader_draw: bool,
    }

    impl FakeDriver {
        fn new(signals: &SignalState) -> Self {
            Self {
                calls: Vec::new(),
                failures: Vec::new(),
                raw_modes: VecDeque::new(),
                sizes: VecDeque::new(),
                resizes: Vec::new(),
                events: VecDeque::new(),
                inject_on: None,
                inject_suspend_on: None,
                inject_continue_on: None,
                signals: signals.clone(),
                poll_timeouts: Vec::new(),
                signal_handlers_active: None,
                draws: Vec::new(),
                quit_after_reader_draw: false,
            }
        }

        fn call(&mut self, name: &'static str) -> io::Result<()> {
            self.calls.push(name);
            if let Some(active) = &self.signal_handlers_active {
                if name == "suspend" {
                    assert!(!active.get());
                } else if matches!(
                    name,
                    "raw_enabled" | "size" | "enable_raw" | "enter_alt" | "hide_cursor"
                ) {
                    assert!(active.get());
                }
            }
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

    struct FakeSuspendControl {
        signals: SignalState,
        active: Rc<StateCell<bool>>,
        calls: Vec<&'static str>,
        inject_termination: bool,
        fail_rearm: bool,
    }

    impl FakeSuspendControl {
        fn new(signals: &SignalState, active: Rc<StateCell<bool>>) -> Self {
            Self {
                signals: signals.clone(),
                active,
                calls: Vec::new(),
                inject_termination: false,
                fail_rearm: false,
            }
        }
    }

    impl SuspendControl for FakeSuspendControl {
        fn suspend<F>(&mut self, stop: F) -> io::Result<SuspendOutcome>
        where
            F: FnOnce() -> io::Result<()>,
        {
            self.calls.push("prepare_signal_stop");
            if self.inject_termination {
                self.signals.store_raw(SIGTERM as usize);
                self.calls.push("cancel_signal_stop");
                return Ok(SuspendOutcome::Cancelled);
            }
            if !self.signals.take_suspend() {
                self.calls.push("cancel_signal_stop");
                return Ok(SuspendOutcome::Cancelled);
            }
            self.active.set(false);
            self.calls.push("inherit_signal_handlers");
            let stop_result = stop();
            self.calls.push("rearm_signal_handlers");
            if self.fail_rearm {
                return Err(io::Error::other("rearm_signal_handlers"));
            }
            self.active.set(true);
            stop_result?;
            Ok(SuspendOutcome::Continued)
        }
    }

    impl TerminalDriver for FakeDriver {
        fn raw_mode_enabled(&mut self) -> io::Result<bool> {
            self.call("raw_enabled")?;
            Ok(self.raw_modes.pop_front().unwrap_or(false))
        }

        fn size(&mut self) -> io::Result<(u16, u16)> {
            self.call("size")?;
            Ok(self.sizes.pop_front().unwrap_or((20, 4)))
        }

        fn resize(&mut self, size: TerminalSize) -> io::Result<()> {
            self.call("resize")?;
            self.resizes.push(size);
            Ok(())
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

        fn draw(&mut self, app: &mut App) -> Result<(), TutError> {
            let kind = match app.view_state()? {
                ViewState::Pending(_) => DrawKind::Pending,
                ViewState::Reader(_) => DrawKind::Reader,
                ViewState::Help { .. } => DrawKind::Help,
            };
            self.draws.push(kind);
            self.call("draw").map_err(|source| TutError::Io {
                operation: "draw terminal frame",
                source,
            })?;
            if self.quit_after_reader_draw && kind == DrawKind::Reader {
                self.quit_after_reader_draw = false;
                self.events.push_back(quit_event());
            }
            Ok(())
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

    #[derive(Default)]
    struct TraceRecorder {
        next_stamp: u64,
        operations: Vec<(RuntimeOperation, u64)>,
        events: u64,
        terminal_sessions: u64,
        suspensions: u64,
        suspend_after_render: Option<SignalState>,
        terminate_after_render: Option<SignalState>,
        terminate_after_search: Option<SignalState>,
    }

    impl RuntimeRecorder for TraceRecorder {
        type Stamp = u64;

        fn begin_operation(&mut self) -> Self::Stamp {
            let stamp = self.next_stamp;
            self.next_stamp = self.next_stamp.saturating_add(1);
            stamp
        }

        fn finish_operation(&mut self, operation: RuntimeOperation, started: Self::Stamp) {
            self.operations.push((operation, started));
            if matches!(
                operation,
                RuntimeOperation::Background(BackgroundWork::Render)
            ) && let Some(signals) = self.suspend_after_render.take()
            {
                signals.store_suspend();
            }
            if matches!(
                operation,
                RuntimeOperation::Background(BackgroundWork::Render)
            ) && let Some(signals) = self.terminate_after_render.take()
            {
                signals.store_raw(SIGTERM as usize);
            }
            if matches!(
                operation,
                RuntimeOperation::Background(BackgroundWork::Search)
            ) && let Some(signals) = self.terminate_after_search.take()
            {
                signals.store_raw(SIGTERM as usize);
            }
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

    fn quit_event() -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn interrupt_event() -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn help_event() -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn document_end_event() -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char('G'),
            modifiers: KeyModifiers::SHIFT,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn character_event(character: char) -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char(character),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn repeated_character_event(character: char) -> Event {
        Event::Key(KeyEvent {
            code: KeyCode::Char(character),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Repeat,
            state: KeyEventState::NONE,
        })
    }

    fn app() -> App {
        app_from_text(Path::new("/tmp/book.txt"), "body".to_owned())
    }

    fn ready_app() -> App {
        let mut app = app();
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        while !app.frame_ready() {
            app.advance_background().unwrap();
        }
        app
    }

    fn merge_pending_app() -> App {
        let mut app = app_from_text(Path::new("/tmp/highlights.txt"), "x x".to_owned());
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        while !app.frame_ready() {
            app.advance_background().unwrap();
        }
        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('x')).unwrap();
        app.update(Action::SearchCommit).unwrap();
        while app.has_background_work() {
            app.advance_background().unwrap();
        }
        app.render_state().unwrap();
        assert!(app.pending_highlight_cursors().is_some());
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.pending_highlight_cursors(), Some((0, 0, None)));
        app
    }

    fn run_with_test_recorder<T: TerminalDriver, R: RuntimeRecorder>(
        app: &mut App,
        driver: &mut T,
        signals: &SignalState,
        recorder: &mut R,
    ) -> Result<RunOutcome, TutError> {
        let mut suspend = ImmediateSuspendControl {
            signals: signals.clone(),
        };
        run_with_recorder(app, driver, signals, &mut suspend, recorder)
    }

    #[test]
    fn terminal_size_limits_bound_cells_and_double_buffers() {
        let boundary = TerminalSize::new(4096, 128).unwrap();
        assert_eq!(terminal_cell_limit(), MAX_TERMINAL_CELLS);
        assert_eq!(
            boundary.buffer_bytes(),
            MAX_TERMINAL_CELLS * 2 * u64::try_from(size_of::<Cell>()).unwrap()
        );
        assert!(boundary.buffer_bytes() <= MAX_TERMINAL_BUFFER_BYTES);
        assert!(TerminalSize::new(1000, 300).is_ok());
        assert!(TerminalSize::new(0, u16::MAX).is_ok());
        assert!(matches!(
            TerminalSize::new(4096, 129),
            Err(TutError::TerminalTooLarge {
                columns: 4096,
                rows: 129,
                cell_limit: MAX_TERMINAL_CELLS,
            })
        ));
        assert!(matches!(
            TerminalSize::new(u16::MAX, u16::MAX),
            Err(TutError::TerminalTooLarge {
                columns: u16::MAX,
                rows: u16::MAX,
                cell_limit: MAX_TERMINAL_CELLS,
            })
        ));
    }

    #[test]
    fn fixed_terminals_do_not_autoresize_during_draw() {
        let fixed = Rect::new(0, 0, 10, 5);
        let options = TerminalOptions {
            viewport: Viewport::Fixed(fixed),
        };
        let mut terminal = Terminal::with_options(TestBackend::new(10, 5), options).unwrap();
        terminal.backend_mut().resize(20, 10);
        let mut rendered = Rect::default();

        terminal
            .draw(|frame| {
                rendered = frame.area();
            })
            .unwrap();

        assert_eq!(rendered, fixed);
        assert_eq!(terminal.get_frame().area(), fixed);
        assert_eq!(terminal.size().unwrap(), (20, 10).into());
    }

    #[test]
    fn oversized_startup_geometry_precedes_terminal_mutation() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((u16::MAX, u16::MAX));
        let mut application = app();

        let error = run_with_driver(&mut application, &mut driver, &signals).unwrap_err();

        assert!(matches!(error, TutError::TerminalTooLarge { .. }));
        assert_eq!(
            error.message(),
            "terminal size 65535x65535 exceeds the 524288-cell limit"
        );
        assert_eq!(driver.calls, ["raw_enabled", "size"]);
        assert!(driver.resizes.is_empty());
        assert!(application.terminal_too_small());
    }

    #[test]
    fn signals_preempt_oversized_startup_geometry() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((u16::MAX, u16::MAX));
        driver.inject_on = Some("size");

        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert_eq!(driver.calls, ["raw_enabled", "size"]);
        assert!(driver.resizes.is_empty());
    }

    #[test]
    fn owned_raw_mode_is_rejected_without_terminal_mutation() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.raw_modes.push_back(true);

        let error = run_with_driver(&mut app(), &mut driver, &signals).unwrap_err();

        assert!(matches!(error, TutError::TerminalInUse));
        assert_eq!(driver.calls, ["raw_enabled"]);
        assert!(driver.resizes.is_empty());
    }

    #[test]
    fn terminating_signal_during_raw_mode_preflight_wins() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.raw_modes.push_back(true);
        driver.inject_on = Some("raw_enabled");

        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert_eq!(driver.calls, ["raw_enabled"]);
        assert!(driver.resizes.is_empty());
    }

    #[test]
    fn raw_mode_query_failure_precedes_terminal_mutation() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.failures.push("raw_enabled");

        let error = run_with_driver(&mut app(), &mut driver, &signals).unwrap_err();

        assert_eq!(error.message(), "failed to query raw mode: raw_enabled");
        assert_eq!(driver.calls, ["raw_enabled"]);
        assert!(driver.resizes.is_empty());
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
        let mut app = ready_app();
        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(
            &driver.calls[..7],
            [
                "raw_enabled",
                "size",
                "enable_raw",
                "enter_alt",
                "hide_cursor",
                "resize",
                "draw"
            ]
        );
        assert_eq!(driver.resizes, [TerminalSize::new(20, 4).unwrap()]);
        assert_eq!(driver.poll_timeouts, [MAX_POLL]);
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn event_loop_consumes_count_once_then_repeated_motion_is_single_step() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.extend([
            character_event('1'),
            character_event('2'),
            repeated_character_event('j'),
            repeated_character_event('j'),
            quit_event(),
        ]);
        let mut recorder = TraceRecorder::default();
        let text = (0..30).map(|line| format!("{line}\n")).collect::<String>();
        let mut app = app_from_text(Path::new("/tmp/repeat.txt"), text);
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        while !app.frame_ready() {
            app.advance_background().unwrap();
        }

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );
        assert!(!app.repeat_active());
        assert_eq!(recorder.events, 5);
        assert_eq!(
            recorder
                .operations
                .iter()
                .filter(|(operation, _)| *operation == RuntimeOperation::Action)
                .count(),
            5
        );
        assert!(
            recorder
                .operations
                .iter()
                .filter(|(operation, _)| {
                    *operation == RuntimeOperation::Background(BackgroundWork::Viewport)
                })
                .count()
                <= 2
        );
        while app.has_background_work() {
            app.advance_background().unwrap();
        }
        assert_eq!(app.render_state().unwrap().current_line, Some(14));
    }

    #[test]
    fn suspension_and_resume_preserve_an_active_repeat_prefix() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.extend([(20, 4), (20, 4)]);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        let mut app = ready_app();
        app.update(Action::RepeatDigit(7)).unwrap();

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(app.repeat_status().unwrap().value(), 7);
        assert_eq!(driver.draws, [DrawKind::Reader, DrawKind::Reader]);
    }

    #[test]
    fn sequential_sessions_each_acquire_and_release_raw_mode() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);

        for _ in 0..2 {
            driver.events.push_back(quit_event());
            assert_eq!(
                run_with_driver(&mut ready_app(), &mut driver, &signals).unwrap(),
                RunOutcome::Normal
            );
        }

        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "raw_enabled")
                .count(),
            2
        );
        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "enable_raw")
                .count(),
            2
        );
        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "disable_raw")
                .count(),
            2
        );
    }

    #[test]
    fn queued_quit_preempts_the_initial_render_job() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        let mut recorder = TraceRecorder::default();
        let mut app = app();

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );

        assert!(driver.draws.is_empty());
        assert!(!driver.calls.contains(&"draw"));
        assert_eq!(recorder.operations, [(RuntimeOperation::Action, 0)]);
        assert!(app.has_background_work());
    }

    #[test]
    fn initial_multistep_render_draws_one_pending_before_the_reader() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((4096, 4));
        driver.quit_after_reader_draw = true;
        let mut recorder = TraceRecorder::default();
        let mut app = app_from_text(Path::new("/tmp/pending-render.txt"), "x".repeat(4096));

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );

        assert_eq!(driver.draws, [DrawKind::Pending, DrawKind::Reader]);
        let render_steps = recorder
            .operations
            .iter()
            .filter(|(operation, _)| {
                *operation == RuntimeOperation::Background(BackgroundWork::Render)
            })
            .count();
        assert!(render_steps > 1);
        assert_eq!(
            recorder.operations.first().unwrap().0,
            RuntimeOperation::Draw
        );
        assert_eq!(
            recorder.operations[recorder.operations.len() - 2].0,
            RuntimeOperation::Draw
        );
        assert_eq!(
            recorder.operations.last().unwrap().0,
            RuntimeOperation::Action
        );
    }

    #[test]
    fn document_end_before_the_first_reader_draws_one_pending_for_all_background_stages() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((16, 4));
        driver.events.push_back(document_end_event());
        driver.quit_after_reader_draw = true;
        let mut recorder = TraceRecorder::default();
        let mut app = app_from_text(
            Path::new("/tmp/pending-document-end.txt"),
            "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2 + 17),
        );

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );

        assert_eq!(driver.draws, [DrawKind::Pending, DrawKind::Reader]);
        let pending_draw = recorder
            .operations
            .iter()
            .position(|(operation, _)| *operation == RuntimeOperation::Draw)
            .unwrap();
        let first_viewport_step = recorder
            .operations
            .iter()
            .position(|(operation, _)| {
                *operation == RuntimeOperation::Background(BackgroundWork::Viewport)
            })
            .unwrap();
        assert_eq!(recorder.operations[0].0, RuntimeOperation::Action);
        assert!(pending_draw < first_viewport_step);
        assert!(
            recorder
                .operations
                .iter()
                .filter(|(operation, _)| {
                    *operation == RuntimeOperation::Background(BackgroundWork::Viewport)
                })
                .count()
                > 1
        );
    }

    #[test]
    fn pending_draw_failure_preempts_background_work() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((4096, 4));
        driver.failures.push("draw");
        let mut recorder = TraceRecorder::default();
        let mut app = app_from_text(Path::new("/tmp/pending-failure.txt"), "x".repeat(4096));

        let error =
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap_err();

        assert!(matches!(
            error,
            TutError::Io {
                operation: "draw terminal frame",
                ..
            }
        ));
        assert_eq!(driver.draws, [DrawKind::Pending]);
        assert_eq!(recorder.operations, [(RuntimeOperation::Draw, 0)]);
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn pending_is_redrawn_once_after_suspend_and_resume() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.extend([(4096, 4), (4096, 4)]);
        driver.quit_after_reader_draw = true;
        let mut recorder = TraceRecorder {
            suspend_after_render: Some(signals.clone()),
            ..TraceRecorder::default()
        };
        let mut app = app_from_text(Path::new("/tmp/pending-resume.txt"), "x".repeat(4096));

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );

        assert_eq!(
            driver.draws,
            [DrawKind::Pending, DrawKind::Pending, DrawKind::Reader]
        );
        assert_eq!(recorder.terminal_sessions, 2);
        assert_eq!(recorder.suspensions, 1);
        assert!(
            recorder
                .operations
                .iter()
                .filter(|(operation, _)| {
                    *operation == RuntimeOperation::Background(BackgroundWork::Render)
                })
                .count()
                > 1
        );
    }

    #[test]
    fn queued_quit_preempts_the_next_render_step() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((4096, 4));
        driver.events.extend([Event::FocusGained, quit_event()]);
        let mut recorder = TraceRecorder::default();
        let mut app = app_from_text(Path::new("/tmp/queued-render.txt"), "x".repeat(4096));

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(
            recorder.operations,
            [
                (RuntimeOperation::Background(BackgroundWork::Render), 0),
                (RuntimeOperation::Action, 1)
            ]
        );
        assert!(!driver.calls.contains(&"draw"));
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
    }

    #[test]
    fn help_draws_before_the_initial_reader_frame_is_complete_and_closes_to_pending() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((4096, 4));
        driver
            .events
            .extend([help_event(), quit_event(), quit_event()]);
        let mut recorder = TraceRecorder::default();
        let mut app = app_from_text(Path::new("/tmp/help-render.txt"), "x".repeat(4096));

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );

        assert_eq!(driver.draws, [DrawKind::Help, DrawKind::Pending]);
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
        assert!(matches!(
            app.mode(),
            crate::app::Mode::Content(crate::app::ContentMode::Reading)
        ));
        assert_eq!(
            recorder.operations,
            [
                (RuntimeOperation::Action, 0),
                (RuntimeOperation::Background(BackgroundWork::Render), 1),
                (RuntimeOperation::Draw, 2),
                (RuntimeOperation::Action, 3),
                (RuntimeOperation::Draw, 4),
                (RuntimeOperation::Background(BackgroundWork::Render), 5),
                (RuntimeOperation::Action, 6),
            ]
        );
    }

    #[test]
    fn keyboard_interrupt_restores_terminal_and_reports_sigint() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(interrupt_event());

        assert_eq!(
            run_with_driver(&mut app(), &mut driver, &signals).unwrap(),
            RunOutcome::Signal(ExternalSignal::Interrupt)
        );
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn runtime_recorder_observes_only_tui_work_boundaries() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("large.txt");
        fs::write(&path, "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.extend([Event::FocusGained, quit_event()]);
        let mut recorder = TraceRecorder::default();

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(recorder.events, 2);
        assert_eq!(recorder.terminal_sessions, 1);
        assert_eq!(recorder.suspensions, 0);
        assert_eq!(
            recorder.operations,
            vec![
                (RuntimeOperation::Background(BackgroundWork::Render), 0),
                (RuntimeOperation::Draw, 1),
                (RuntimeOperation::Action, 2),
            ]
        );
    }

    #[test]
    fn runtime_recorder_accumulates_across_suspension() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        let mut recorder = TraceRecorder::default();
        let mut app = ready_app();

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(recorder.events, 1);
        assert_eq!(recorder.terminal_sessions, 2);
        assert_eq!(recorder.suspensions, 1);
        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "raw_enabled")
                .count(),
            2
        );
        assert_eq!(
            recorder.operations,
            vec![
                (RuntimeOperation::Draw, 0),
                (RuntimeOperation::Draw, 1),
                (RuntimeOperation::Action, 2),
            ]
        );
    }

    #[test]
    fn signal_handlers_rearm_before_terminal_reinitialization() {
        let signals = SignalState::empty();
        let active = Rc::new(StateCell::new(true));
        let mut driver = FakeDriver::new(&signals);
        driver.signal_handlers_active = Some(Rc::clone(&active));
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        let mut suspend = FakeSuspendControl::new(&signals, active);
        let mut recorder = DisabledRecorder;

        assert_eq!(
            run_with_recorder(
                &mut ready_app(),
                &mut driver,
                &signals,
                &mut suspend,
                &mut recorder,
            )
            .unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(
            suspend.calls,
            [
                "prepare_signal_stop",
                "inherit_signal_handlers",
                "rearm_signal_handlers"
            ]
        );
        assert_eq!(
            driver.calls.iter().filter(|call| **call == "size").count(),
            2
        );
    }

    #[test]
    fn termination_before_signal_stop_commit_cancels_suspension() {
        let signals = SignalState::empty();
        let active = Rc::new(StateCell::new(true));
        let mut driver = FakeDriver::new(&signals);
        driver.signal_handlers_active = Some(Rc::clone(&active));
        driver.inject_suspend_on = Some("poll");
        let mut suspend = FakeSuspendControl::new(&signals, active);
        suspend.inject_termination = true;
        let mut recorder = DisabledRecorder;

        assert_eq!(
            run_with_recorder(
                &mut ready_app(),
                &mut driver,
                &signals,
                &mut suspend,
                &mut recorder,
            )
            .unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert_eq!(suspend.calls, ["prepare_signal_stop", "cancel_signal_stop"]);
        assert!(!driver.calls.contains(&"suspend"));
    }

    #[test]
    fn failed_signal_rearm_prevents_terminal_reinitialization() {
        let signals = SignalState::empty();
        let active = Rc::new(StateCell::new(true));
        let mut driver = FakeDriver::new(&signals);
        driver.signal_handlers_active = Some(Rc::clone(&active));
        driver.inject_suspend_on = Some("poll");
        let mut suspend = FakeSuspendControl::new(&signals, active);
        suspend.fail_rearm = true;
        let mut recorder = DisabledRecorder;

        let error = run_with_recorder(
            &mut ready_app(),
            &mut driver,
            &signals,
            &mut suspend,
            &mut recorder,
        )
        .unwrap_err();

        assert_eq!(
            error.message(),
            "failed to suspend process: rearm_signal_handlers"
        );
        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "enable_raw")
                .count(),
            1
        );
        assert_eq!(driver.calls.last(), Some(&"suspend"));
    }

    #[test]
    fn suspension_restores_then_reinitializes_and_forces_a_redraw() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        driver.inject_suspend_on = Some("poll");
        let mut app = ready_app();

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        let transition = [
            "poll",
            "show_cursor",
            "leave_alt",
            "disable_raw",
            "suspend",
            "raw_enabled",
            "size",
            "enable_raw",
            "enter_alt",
            "hide_cursor",
            "resize",
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
    fn suspension_preserves_pending_frames_only_for_the_same_geometry() {
        fn remaining_render_steps(app: &mut App) -> usize {
            let mut steps = 0;
            while !app.frame_ready() {
                assert_eq!(app.background_work(), Some(BackgroundWork::Render));
                app.advance_background().unwrap();
                steps += 1;
            }
            steps
        }

        fn suspend_after_one_step(resume_width: u16) -> App {
            let signals = SignalState::empty();
            let mut driver = FakeDriver::new(&signals);
            driver.sizes.extend([(4096, 4), (resume_width, 4)]);
            driver.events.extend([Event::FocusGained, quit_event()]);
            let mut recorder = TraceRecorder {
                suspend_after_render: Some(signals.clone()),
                ..TraceRecorder::default()
            };
            let mut app = app_from_text(Path::new("/tmp/partial-frame.txt"), "x".repeat(4096));

            assert_eq!(
                run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
                RunOutcome::Normal
            );
            assert_eq!(recorder.suspensions, 1);
            assert_eq!(
                recorder
                    .operations
                    .iter()
                    .filter(|(operation, _)| matches!(
                        operation,
                        RuntimeOperation::Background(BackgroundWork::Render)
                    ))
                    .count(),
                1
            );
            app
        }

        let mut preserved = suspend_after_one_step(4096);
        assert_eq!(remaining_render_steps(&mut preserved), 4);

        let mut discarded = suspend_after_one_step(2048);
        let discarded_steps = remaining_render_steps(&mut discarded);
        let mut fresh = app_from_text(Path::new("/tmp/fresh-frame.txt"), "x".repeat(4096));
        fresh
            .update(Action::Resize(Geometry::new(2048, 4)))
            .unwrap();
        assert_eq!(discarded_steps, remaining_render_steps(&mut fresh));
    }

    #[test]
    fn resumed_session_rechecks_raw_mode_before_reinitialization() {
        let signals = SignalState::empty();
        let active = Rc::new(StateCell::new(true));
        let mut driver = FakeDriver::new(&signals);
        driver.signal_handlers_active = Some(Rc::clone(&active));
        driver.raw_modes.extend([false, true]);
        driver.inject_suspend_on = Some("poll");
        let mut suspend = FakeSuspendControl::new(&signals, active);
        let mut recorder = TraceRecorder::default();

        let error = run_with_recorder(
            &mut app(),
            &mut driver,
            &signals,
            &mut suspend,
            &mut recorder,
        )
        .unwrap_err();

        assert!(matches!(error, TutError::TerminalInUse));
        assert_eq!(recorder.terminal_sessions, 1);
        assert_eq!(recorder.suspensions, 1);
        assert_eq!(
            suspend.calls,
            [
                "prepare_signal_stop",
                "inherit_signal_handlers",
                "rearm_signal_handlers"
            ]
        );
        assert_eq!(driver.resizes, [TerminalSize::new(20, 4).unwrap()]);
        assert!(driver.calls.ends_with(&[
            "show_cursor",
            "leave_alt",
            "disable_raw",
            "suspend",
            "raw_enabled"
        ]));
        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "enable_raw")
                .count(),
            1
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
        assert_eq!(driver.resizes.len(), 2);
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
    fn oversized_active_resize_restores_without_resizing_again() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(Event::Resize(u16::MAX, u16::MAX));
        let mut app = ready_app();

        let error = run_with_driver(&mut app, &mut driver, &signals).unwrap_err();

        assert!(matches!(error, TutError::TerminalTooLarge { .. }));
        assert_eq!(driver.resizes, [TerminalSize::new(20, 4).unwrap()]);
        assert_eq!(
            driver.calls.iter().filter(|call| **call == "draw").count(),
            1
        );
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn accepted_active_resize_updates_driver_before_redrawing() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.extend([Event::Resize(40, 10), quit_event()]);
        let mut app = ready_app();

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(
            driver.resizes,
            [
                TerminalSize::new(20, 4).unwrap(),
                TerminalSize::new(40, 10).unwrap()
            ]
        );
        assert!(
            driver
                .calls
                .windows(3)
                .any(|calls| calls == ["read", "resize", "draw"])
        );
    }

    #[test]
    fn same_size_resize_resets_the_fixed_viewport_and_redraws() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.extend([Event::Resize(20, 4), quit_event()]);
        let mut app = ready_app();

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert_eq!(
            driver.resizes,
            [
                TerminalSize::new(20, 4).unwrap(),
                TerminalSize::new(20, 4).unwrap()
            ]
        );
        assert_eq!(
            driver.calls.iter().filter(|call| **call == "draw").count(),
            2
        );
    }

    #[test]
    fn oversized_geometry_after_suspend_precedes_reinitialization() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.extend([(20, 4), (u16::MAX, u16::MAX)]);
        driver.inject_suspend_on = Some("poll");

        let error = run_with_driver(&mut app(), &mut driver, &signals).unwrap_err();

        assert!(matches!(error, TutError::TerminalTooLarge { .. }));
        assert_eq!(driver.resizes, [TerminalSize::new(20, 4).unwrap()]);
        assert_eq!(
            driver
                .calls
                .iter()
                .filter(|call| **call == "enable_raw")
                .count(),
            1
        );
        assert_eq!(driver.calls.last(), Some(&"size"));
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
    fn signal_preempts_initial_render_work() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("large.txt");
        fs::write(&path, "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.inject_on = Some("poll");
        let mut recorder = TraceRecorder::default();

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert!(driver.draws.is_empty());
        assert!(recorder.operations.is_empty());
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
        assert_eq!(driver.poll_timeouts, vec![BACKGROUND_POLL]);
        assert!(
            driver
                .calls
                .ends_with(&["show_cursor", "leave_alt", "disable_raw"])
        );
    }

    #[test]
    fn signal_after_one_render_step_prevents_partial_frame_draw() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.sizes.push_back((4096, 4));
        let mut recorder = TraceRecorder {
            terminate_after_render: Some(signals.clone()),
            ..TraceRecorder::default()
        };
        let mut app = app_from_text(Path::new("/tmp/signal-render.txt"), "x".repeat(4096));

        assert_eq!(
            run_with_test_recorder(&mut app, &mut driver, &signals, &mut recorder).unwrap(),
            RunOutcome::Signal(ExternalSignal::Terminate)
        );
        assert_eq!(
            recorder.operations,
            [
                (RuntimeOperation::Draw, 0),
                (RuntimeOperation::Background(BackgroundWork::Render), 1)
            ]
        );
        assert_eq!(driver.draws, [DrawKind::Pending]);
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
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
        driver.events.extend([
            Event::FocusGained,
            Event::FocusGained,
            Event::FocusGained,
            quit_event(),
        ]);

        assert_eq!(
            run_with_driver(&mut app, &mut driver, &signals).unwrap(),
            RunOutcome::Normal
        );
        assert!(!app.has_background_work());
        assert_eq!(
            driver.poll_timeouts,
            [BACKGROUND_POLL, BACKGROUND_POLL, BACKGROUND_POLL, MAX_POLL,]
        );
    }

    #[test]
    fn queued_quit_preempts_a_highlight_merge_step() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.push_back(quit_event());
        let mut recorder = TraceRecorder::default();
        let mut app = merge_pending_app();
        let before = app.pending_highlight_cursors();

        assert!(matches!(
            event_loop(&mut app, &mut driver, &signals, &mut recorder),
            Primary::Normal
        ));
        assert_eq!(app.pending_highlight_cursors(), before);
        assert!(recorder.operations.iter().all(|(operation, _)| !matches!(
            operation,
            RuntimeOperation::Background(BackgroundWork::Search)
        )));
        assert_eq!(recorder.events, 1);
    }

    #[test]
    fn continuous_terminal_events_do_not_starve_highlight_phases() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.events.extend([
            Event::FocusGained,
            Event::FocusGained,
            Event::FocusGained,
            Event::FocusGained,
            Event::FocusGained,
            quit_event(),
        ]);
        let mut recorder = TraceRecorder::default();
        let mut app = merge_pending_app();

        assert!(matches!(
            event_loop(&mut app, &mut driver, &signals, &mut recorder),
            Primary::Normal
        ));
        assert_eq!(app.pending_highlight_cursors(), None);
        assert_eq!(app.published_highlight_count(), 2);
        assert_eq!(
            recorder
                .operations
                .iter()
                .filter(|(operation, _)| matches!(
                    operation,
                    RuntimeOperation::Background(BackgroundWork::Search)
                ))
                .count(),
            4
        );
    }

    #[test]
    fn queued_signal_preempts_a_highlight_merge_step() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        driver.inject_on = Some("poll");
        let mut recorder = TraceRecorder::default();
        let mut app = merge_pending_app();
        let before = app.pending_highlight_cursors();

        assert!(matches!(
            event_loop(&mut app, &mut driver, &signals, &mut recorder),
            Primary::Signal(ExternalSignal::Terminate)
        ));
        assert_eq!(app.pending_highlight_cursors(), before);
        assert!(recorder.operations.iter().all(|(operation, _)| !matches!(
            operation,
            RuntimeOperation::Background(BackgroundWork::Search)
        )));
    }

    #[test]
    fn one_event_loop_turn_runs_at_most_one_highlight_phase() {
        let signals = SignalState::empty();
        let mut driver = FakeDriver::new(&signals);
        let mut recorder = TraceRecorder {
            terminate_after_search: Some(signals.clone()),
            ..TraceRecorder::default()
        };
        let mut app = merge_pending_app();
        let before = app.pending_highlight_cursors();

        assert!(matches!(
            event_loop(&mut app, &mut driver, &signals, &mut recorder),
            Primary::Signal(ExternalSignal::Terminate)
        ));
        assert_ne!(app.pending_highlight_cursors(), before);
        assert_eq!(
            recorder
                .operations
                .iter()
                .filter(|(operation, _)| matches!(
                    operation,
                    RuntimeOperation::Background(BackgroundWork::Search)
                ))
                .count(),
            1
        );
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
