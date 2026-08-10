use std::{
    fmt, io, mem, ptr,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[cfg(test)]
use std::sync::Arc;

use crate::error::ExternalSignal;

const TERMINATION_SIGNALS: [libc::c_int; 4] =
    [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM];
const MANAGED_SIGNALS: [libc::c_int; 6] = [
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGTSTP,
    libc::SIGCONT,
];

struct PendingSignals {
    termination: AtomicUsize,
    suspend: AtomicBool,
}

impl PendingSignals {
    const fn new() -> Self {
        Self {
            termination: AtomicUsize::new(0),
            suspend: AtomicBool::new(false),
        }
    }

    fn reset(&self) {
        self.termination.store(0, Ordering::SeqCst);
        self.suspend.store(false, Ordering::SeqCst);
    }
}

static PROCESS_SIGNALS: PendingSignals = PendingSignals::new();
static PROCESS_SESSION_LEASE: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
enum PendingSignalsRef {
    Process,
    #[cfg(test)]
    Owned(Arc<PendingSignals>),
}

#[derive(Clone)]
pub(crate) struct SignalState(PendingSignalsRef);

impl SignalState {
    #[cfg(test)]
    pub(super) fn empty() -> Self {
        Self(PendingSignalsRef::Owned(Arc::new(PendingSignals::new())))
    }

    fn process() -> Self {
        Self(PendingSignalsRef::Process)
    }

    fn pending(&self) -> &PendingSignals {
        match &self.0 {
            PendingSignalsRef::Process => &PROCESS_SIGNALS,
            #[cfg(test)]
            PendingSignalsRef::Owned(pending) => pending,
        }
    }

    pub(crate) fn received(&self) -> Option<ExternalSignal> {
        match self.pending().termination.load(Ordering::SeqCst) {
            signal if signal == libc::SIGHUP as usize => Some(ExternalSignal::Hangup),
            signal if signal == libc::SIGINT as usize => Some(ExternalSignal::Interrupt),
            signal if signal == libc::SIGQUIT as usize => Some(ExternalSignal::Quit),
            signal if signal == libc::SIGTERM as usize => Some(ExternalSignal::Terminate),
            _ => None,
        }
    }

    pub(super) fn suspend_requested(&self) -> bool {
        self.pending().suspend.load(Ordering::SeqCst)
    }

    pub(super) fn take_suspend(&self) -> bool {
        self.pending().suspend.swap(false, Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(super) fn store_raw(&self, signal: usize) {
        let _ = self.pending().termination.compare_exchange(
            0,
            signal,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }

    #[cfg(test)]
    pub(super) fn store_suspend(&self) {
        self.pending().suspend.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(super) fn store_continue(&self) {
        self.pending().suspend.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct ProcessSessionLease {
    held: bool,
}

impl ProcessSessionLease {
    pub(crate) fn acquire() -> Option<Self> {
        PROCESS_SESSION_LEASE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Self { held: true })
    }

    fn release(&mut self) {
        if self.held {
            self.held = false;
            PROCESS_SESSION_LEASE.store(false, Ordering::SeqCst);
        }
    }
}

impl Drop for ProcessSessionLease {
    fn drop(&mut self) {
        self.release();
    }
}

struct SignalMask {
    previous: Option<libc::sigset_t>,
}

impl SignalMask {
    fn block(signals: &[libc::c_int]) -> io::Result<Self> {
        // SAFETY: sigset_t is an opaque C value initialized by sigemptyset before use.
        let mut blocked = unsafe { mem::zeroed::<libc::sigset_t>() };
        // SAFETY: blocked points to writable storage for a sigset_t.
        if unsafe { libc::sigemptyset(&mut blocked) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for signal in signals {
            // SAFETY: blocked remains initialized and every signal is a supported constant.
            if unsafe { libc::sigaddset(&mut blocked, *signal) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }

        // SAFETY: pthread_sigmask initializes previous and only reads blocked during the call.
        let mut previous = unsafe { mem::zeroed::<libc::sigset_t>() };
        // SAFETY: both signal-set pointers are valid for the duration of the call.
        let error = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        Ok(Self {
            previous: Some(previous),
        })
    }

    fn restore(&mut self) -> io::Result<()> {
        let Some(previous) = self.previous.as_ref() else {
            return Ok(());
        };
        // SAFETY: previous was initialized by pthread_sigmask and remains borrowed for the call.
        let error = unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, previous, ptr::null_mut()) };
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        self.previous = None;
        Ok(())
    }
}

impl Drop for SignalMask {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

extern "C" fn process_signal_handler(signal: libc::c_int) {
    match signal {
        libc::SIGHUP | libc::SIGINT | libc::SIGQUIT | libc::SIGTERM => {
            let _ = PROCESS_SIGNALS.termination.compare_exchange(
                0,
                signal as usize,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
        libc::SIGTSTP => PROCESS_SIGNALS.suspend.store(true, Ordering::SeqCst),
        libc::SIGCONT => PROCESS_SIGNALS.suspend.store(false, Ordering::SeqCst),
        _ => {}
    }
}

fn managed_action() -> io::Result<libc::sigaction> {
    // SAFETY: libc::sigaction contains only C scalar, pointer, and sigset_t fields on Unix.
    let mut action = unsafe { mem::zeroed::<libc::sigaction>() };
    action.sa_sigaction = process_signal_handler as *const () as libc::sighandler_t;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: sa_mask points to writable storage within action.
    if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
        return Err(io::Error::last_os_error());
    }
    for signal in MANAGED_SIGNALS {
        // SAFETY: sa_mask remains initialized and signal is a supported constant.
        if unsafe { libc::sigaddset(&mut action.sa_mask, signal) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(action)
}

fn install_action(
    signal: libc::c_int,
    action: &libc::sigaction,
    previous: *mut libc::sigaction,
) -> io::Result<()> {
    // SAFETY: action is initialized, and previous is either null or points to writable storage.
    if unsafe { libc::sigaction(signal, action, previous) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn combine_errors(primary: io::Error, cleanup: io::Result<()>) -> io::Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => io::Error::other(TransitionError { primary, cleanup }),
    }
}

fn combine_results(first: io::Result<()>, second: io::Result<()>) -> io::Result<()> {
    match first {
        Ok(()) => second,
        Err(primary) => Err(combine_errors(primary, second)),
    }
}

#[derive(Debug)]
struct TransitionError {
    primary: io::Error,
    cleanup: io::Error,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}; signal-state cleanup failed: {}",
            self.primary, self.cleanup
        )
    }
}

impl std::error::Error for TransitionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.primary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SuspendOutcome {
    Cancelled,
    Continued,
}

pub(crate) struct SignalHandlers {
    state: SignalState,
    inherited: [libc::sigaction; MANAGED_SIGNALS.len()],
    action: libc::sigaction,
    installed: bool,
    lease: Option<ProcessSessionLease>,
}

impl SignalHandlers {
    pub(crate) fn install(mut lease: ProcessSessionLease) -> io::Result<Self> {
        PROCESS_SIGNALS.reset();
        let mut mask = match SignalMask::block(&MANAGED_SIGNALS) {
            Ok(mask) => mask,
            Err(error) => {
                lease.release();
                return Err(error);
            }
        };
        let action = match managed_action() {
            Ok(action) => action,
            Err(error) => {
                let cleanup = mask.restore();
                if cleanup.is_ok() {
                    lease.release();
                } else {
                    mem::forget(lease);
                }
                return Err(combine_errors(error, cleanup));
            }
        };
        // SAFETY: each element is output storage initialized by sigaction before it is read.
        let mut inherited = unsafe { [mem::zeroed::<libc::sigaction>(); MANAGED_SIGNALS.len()] };
        let mut installed = 0;
        for (index, signal) in MANAGED_SIGNALS.into_iter().enumerate() {
            if let Err(primary) = install_action(signal, &action, &mut inherited[index]) {
                let cleanup = restore_prefix(&inherited, installed);
                let mask_cleanup = mask.restore();
                let cleanup = combine_results(cleanup, mask_cleanup);
                if cleanup.is_ok() {
                    lease.release();
                } else {
                    mem::forget(lease);
                }
                return Err(combine_errors(primary, cleanup));
            }
            installed += 1;
        }
        if let Err(primary) = mask.restore() {
            let disposition_cleanup = restore_prefix(&inherited, installed);
            let mask_cleanup = mask.restore();
            let cleanup = combine_results(disposition_cleanup, mask_cleanup);
            if cleanup.is_ok() {
                lease.release();
            } else {
                mem::forget(lease);
            }
            return Err(combine_errors(primary, cleanup));
        }

        Ok(Self {
            state: SignalState::process(),
            inherited,
            action,
            installed: true,
            lease: Some(lease),
        })
    }

    pub(crate) fn state(&self) -> &SignalState {
        &self.state
    }

    fn set_inherited_termination(&self) -> io::Result<()> {
        let mut first = None;
        for (index, signal) in TERMINATION_SIGNALS.into_iter().enumerate() {
            retain_first(
                &mut first,
                install_action(signal, &self.inherited[index], ptr::null_mut()),
            );
        }
        first.map_or(Ok(()), Err)
    }

    fn set_active_termination(&self) -> io::Result<()> {
        let mut first = None;
        for signal in TERMINATION_SIGNALS {
            retain_first(
                &mut first,
                install_action(signal, &self.action, ptr::null_mut()),
            );
        }
        first.map_or(Ok(()), Err)
    }

    fn take_active_termination(&mut self) -> io::Result<()> {
        let mut first = None;
        for (index, signal) in TERMINATION_SIGNALS.into_iter().enumerate() {
            retain_first(
                &mut first,
                install_action(signal, &self.action, &mut self.inherited[index]),
            );
        }
        first.map_or(Ok(()), Err)
    }

    pub(super) fn suspend<F>(&mut self, stop: F) -> io::Result<SuspendOutcome>
    where
        F: FnOnce() -> io::Result<()>,
    {
        let mut mask = SignalMask::block(&TERMINATION_SIGNALS)?;
        if self.state.received().is_some() || !self.state.suspend_requested() {
            mask.restore()?;
            return Ok(SuspendOutcome::Cancelled);
        }
        if let Err(primary) = self.set_inherited_termination() {
            let disposition_cleanup = self.set_active_termination();
            let mask_cleanup = mask.restore();
            let cleanup = combine_results(disposition_cleanup, mask_cleanup);
            return Err(combine_errors(primary, cleanup));
        }
        if self.state.received().is_some() || !self.state.take_suspend() {
            let primary = self.take_active_termination();
            let mask_result = mask.restore();
            combine_results(primary, mask_result)?;
            return Ok(SuspendOutcome::Cancelled);
        }
        if let Err(primary) = mask.restore() {
            let disposition_cleanup = self.take_active_termination();
            let mask_cleanup = mask.restore();
            return Err(combine_errors(
                primary,
                combine_results(disposition_cleanup, mask_cleanup),
            ));
        }

        let stop_result = stop();
        let mut resume_mask = match SignalMask::block(&TERMINATION_SIGNALS) {
            Ok(mask) => mask,
            Err(rearm) => {
                return Err(match stop_result {
                    Ok(()) => rearm,
                    Err(primary) => combine_errors(primary, Err(rearm)),
                });
            }
        };
        let disposition_rearm = self.take_active_termination();
        let mask_rearm = resume_mask.restore();
        let rearm = combine_results(disposition_rearm, mask_rearm);
        match (stop_result, rearm) {
            (Ok(()), Ok(())) => Ok(SuspendOutcome::Continued),
            (Err(primary), cleanup) => Err(combine_errors(primary, cleanup)),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    pub(crate) fn restore(&mut self) -> io::Result<()> {
        if !self.installed {
            return Ok(());
        }
        let mut mask = SignalMask::block(&MANAGED_SIGNALS)?;
        let cleanup = restore_prefix(&self.inherited, MANAGED_SIGNALS.len());
        let mask_result = mask.restore();
        combine_results(cleanup, mask_result)?;
        self.installed = false;
        Ok(())
    }
}

impl Drop for SignalHandlers {
    fn drop(&mut self) {
        if self.restore().is_err()
            && let Some(lease) = self.lease.take()
        {
            mem::forget(lease);
        }
    }
}

fn restore_prefix(
    inherited: &[libc::sigaction; MANAGED_SIGNALS.len()],
    count: usize,
) -> io::Result<()> {
    let mut first = None;
    for index in 0..count {
        retain_first(
            &mut first,
            install_action(MANAGED_SIGNALS[index], &inherited[index], ptr::null_mut()),
        );
    }
    first.map_or(Ok(()), Err)
}

fn retain_first(first: &mut Option<io::Error>, result: io::Result<()>) {
    if let Err(error) = result
        && first.is_none()
    {
        *first = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, fs, thread};

    use tempfile::tempdir;

    use super::*;
    use crate::{RunResult, TutError};

    #[cfg(target_os = "linux")]
    const LIBC_MANAGED_ACTION_FLAGS: libc::c_int = 0x0400_0000;
    #[cfg(not(target_os = "linux"))]
    const LIBC_MANAGED_ACTION_FLAGS: libc::c_int = 0;

    extern "C" fn inherited_handler(_: libc::c_int) {}

    struct ActionGuard {
        signal: libc::c_int,
        action: libc::sigaction,
    }

    impl Drop for ActionGuard {
        fn drop(&mut self) {
            let _ = install_action(self.signal, &self.action, ptr::null_mut());
        }
    }

    fn current_action(signal: libc::c_int) -> libc::sigaction {
        // SAFETY: action is output storage initialized by sigaction.
        let mut action = unsafe { mem::zeroed::<libc::sigaction>() };
        // SAFETY: action points to writable storage and a null action only queries state.
        assert_eq!(
            unsafe { libc::sigaction(signal, ptr::null(), &mut action) },
            0
        );
        action
    }

    fn same_action(left: &libc::sigaction, right: &libc::sigaction) -> bool {
        left.sa_sigaction == right.sa_sigaction
            && left.sa_flags & !LIBC_MANAGED_ACTION_FLAGS
                == right.sa_flags & !LIBC_MANAGED_ACTION_FLAGS
            && MANAGED_SIGNALS
                .into_iter()
                .chain([libc::SIGUSR1])
                .all(|signal| {
                    // SAFETY: left's mask was initialized by sigaction and signal is in range.
                    let left_member = unsafe { libc::sigismember(&left.sa_mask, signal) };
                    // SAFETY: right's mask was initialized by sigaction and signal is in range.
                    let right_member = unsafe { libc::sigismember(&right.sa_mask, signal) };
                    left_member == right_member
                })
    }

    fn inherited_action(handler: libc::sighandler_t) -> libc::sigaction {
        // SAFETY: libc::sigaction contains only C scalar, pointer, and sigset_t fields on Unix.
        let mut action = unsafe { mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = handler;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: sa_mask points to writable storage within action.
        assert_eq!(unsafe { libc::sigemptyset(&mut action.sa_mask) }, 0);
        // SAFETY: sa_mask is initialized and SIGUSR1 is supported on both target platforms.
        assert_eq!(
            unsafe { libc::sigaddset(&mut action.sa_mask, libc::SIGUSR1) },
            0
        );
        action
    }

    fn assert_busy(result: Result<RunResult, TutError>) {
        assert!(matches!(result, Err(TutError::Busy)));
    }

    #[test]
    fn open_lease_and_signal_handlers_restore_sequentially() {
        let inherited = MANAGED_SIGNALS.map(current_action);

        for iteration in 0..2 {
            let lease = ProcessSessionLease::acquire().unwrap();
            assert!(ProcessSessionLease::acquire().is_none());
            if iteration == 0 {
                assert_eq!(
                    crate::run([OsString::from("--help")]).unwrap(),
                    RunResult::Help
                );
                assert_eq!(
                    crate::run([OsString::from("--version")]).unwrap(),
                    RunResult::Version
                );
                assert_busy(
                    thread::spawn(|| crate::run([OsString::from("-")]))
                        .join()
                        .unwrap(),
                );

                let directory = tempdir().unwrap();
                let input = directory.path().join("input.txt");
                let log = directory.path().join("session.log");
                fs::write(&input, "text").unwrap();
                let arguments = [
                    OsString::from("--log-file"),
                    log.clone().into_os_string(),
                    input.into_os_string(),
                ];
                assert_busy(thread::spawn(move || crate::run(arguments)).join().unwrap());
                assert!(!log.exists());
            }

            let mut handlers = SignalHandlers::install(lease).unwrap();
            assert!(
                MANAGED_SIGNALS
                    .into_iter()
                    .map(current_action)
                    .all(|action| action.sa_sigaction == handlers.action.sa_sigaction)
            );
            handlers.restore().unwrap();
            assert!(ProcessSessionLease::acquire().is_none());
            for (signal, expected) in MANAGED_SIGNALS.into_iter().zip(&inherited) {
                assert!(same_action(&current_action(signal), expected));
            }
        }

        for (signal, action) in [
            (libc::SIGTERM, inherited_action(libc::SIG_IGN)),
            (
                libc::SIGHUP,
                inherited_action(inherited_handler as *const () as libc::sighandler_t),
            ),
        ] {
            let baseline = current_action(signal);
            let guard = ActionGuard {
                signal,
                action: baseline,
            };
            install_action(signal, &action, ptr::null_mut()).unwrap();
            let replacement = (signal == libc::SIGHUP).then(|| inherited_action(libc::SIG_IGN));
            let lease = ProcessSessionLease::acquire().unwrap();
            let mut handlers = SignalHandlers::install(lease).unwrap();
            handlers.state.store_suspend();
            assert_eq!(
                handlers
                    .suspend(|| {
                        assert!(same_action(&current_action(signal), &action));
                        if let Some(replacement) = &replacement {
                            install_action(signal, replacement, ptr::null_mut()).unwrap();
                        }
                        Ok(())
                    })
                    .unwrap(),
                SuspendOutcome::Continued
            );
            handlers.restore().unwrap();
            assert!(same_action(
                &current_action(signal),
                replacement.as_ref().unwrap_or(&action)
            ));
            drop(guard);
            assert!(same_action(&current_action(signal), &baseline));
        }
    }
}
