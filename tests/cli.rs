use std::{
    fs,
    process::{Command, Stdio},
};

use tempfile::{NamedTempFile, tempdir};

fn tut() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tut"));
    command.env_remove("TUT_LOG_FILE");
    command
}

#[test]
fn help_and_version_bypass_terminal_checks() {
    for (arguments, expected) in [
        (vec!["--help"], tut::HELP),
        (vec!["book.txt", "--unknown", "--help", "extra"], tut::HELP),
        (vec!["--version"], tut::VERSION_OUTPUT),
        (
            vec!["--unknown", "book.txt", "--version", "extra"],
            tut::VERSION_OUTPUT,
        ),
    ] {
        let output = tut().args(arguments).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, expected.as_bytes());
        assert!(output.stderr.is_empty());
    }

    let directory = tempdir().unwrap();
    let log = directory.path().join("help.log");
    let output = tut()
        .env("TUT_LOG_FILE", &log)
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!log.exists());
}

#[test]
fn invalid_invocations_use_gnu_diagnostics_and_exit_two() {
    for (arguments, message) in [
        (Vec::<&str>::new(), "missing file operand"),
        (vec!["a", "b"], "extra operand 'b'"),
        (vec!["--unknown"], "unrecognized option '--unknown'"),
        (
            vec!["--log-file"],
            "option '--log-file' requires an argument",
        ),
    ] {
        let output = tut().args(arguments).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            format!(
                "tut: {message}\n{}\nTry 'tut --help' for more information.\n",
                tut::USAGE
            )
        );
    }
}

#[test]
fn double_dash_accepts_a_leading_dash_filename() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("-book.txt"), "text").unwrap();
    let output = tut()
        .current_dir(directory.path())
        .args(["--", "-book.txt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"tut: interactive reading requires terminal input and output\n"
    );
}

#[test]
fn terminal_validation_precedes_file_access() {
    let missing = tut()
        .arg("/definitely/missing/tut-terminal-ordering")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        missing.stderr,
        b"tut: interactive reading requires terminal input and output\n"
    );

    let file = NamedTempFile::new().unwrap();
    fs::write(file.path(), "valid UTF-8").unwrap();
    let valid = tut().arg(file.path()).output().unwrap();
    assert_eq!(valid.status.code(), Some(1));
    assert!(valid.stdout.is_empty());
    assert_eq!(valid.stderr, missing.stderr);
}

#[test]
fn standard_input_requires_terminal_output() {
    let directory = tempdir().unwrap();
    let log = directory.path().join("session.log");
    let output = tut()
        .arg("-")
        .arg("--log-file")
        .arg(&log)
        .stdin(Stdio::piped())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"tut: interactive reading requires terminal input and output\n"
    );
    assert!(!log.exists());
}

#[cfg(unix)]
#[test]
fn diagnostics_escape_control_bytes_in_arguments() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let option = OsString::from_vec(b"--bad\n\x1b".to_vec());
    let output = tut().arg(option).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("tut: unrecognized option '--bad\\x0a\\x1b'\n"));
    assert_eq!(stderr.lines().count(), 3);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod pty {
    use std::{
        ffi::CString,
        fs::File,
        io::{self, Read, Write},
        os::unix::process::{CommandExt, ExitStatusExt},
        path::Path,
        process::{Child, ChildStderr, ChildStdin, Command, ExitStatus, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled};
    use rustix::{
        fs::{self, Mode, OFlags},
        process::{
            Pid, Signal, WaitOptions, getpgrp, ioctl_tiocsctty, kill_process, setsid, waitpid,
        },
        pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
        termios::{self, SpecialCodeIndex, Termios, Winsize},
    };
    use tempfile::{NamedTempFile, tempdir};

    use super::tut;

    const ENTER_ALT: &[u8] = b"\x1b[?1049h";
    const LEAVE_ALT: &[u8] = b"\x1b[?1049l";
    const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
    const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
    const NESTED_RAW_HELPER: &str = "TUT_TEST_NESTED_RAW_HELPER";
    const TIMEOUT: Duration = Duration::from_secs(5);

    fn reset_signal_state() -> io::Result<()> {
        // SAFETY: sigaction is fully initialized before each synchronous libc call.
        let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = libc::SIG_DFL;
        // SAFETY: sa_mask points to writable storage within action.
        if unsafe { libc::sigemptyset(&mut action.sa_mask) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
            // SAFETY: action is initialized and the signal is supported on both test targets.
            if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // SAFETY: sigset_t is initialized before it is passed to libc.
        let mut signals = unsafe { std::mem::zeroed::<libc::sigset_t>() };
        // SAFETY: signals points to writable sigset_t storage.
        if unsafe { libc::sigemptyset(&mut signals) } != 0 {
            return Err(io::Error::last_os_error());
        }
        for signal in [
            libc::SIGHUP,
            libc::SIGINT,
            libc::SIGQUIT,
            libc::SIGTERM,
            libc::SIGTSTP,
            libc::SIGCONT,
        ] {
            // SAFETY: signals remains initialized and each signal is supported on both targets.
            if unsafe { libc::sigaddset(&mut signals, signal) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // SAFETY: signals is initialized and the null output pointer requests no prior mask.
        if unsafe { libc::sigprocmask(libc::SIG_UNBLOCK, &signals, std::ptr::null_mut()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[derive(Clone, Copy)]
    enum TestInput<'a> {
        Path(&'a Path),
        StandardInput(&'a [u8]),
        OpenStandardInput,
        UnreadableStandardInput,
    }

    struct PtyChild {
        child: Child,
        master: File,
        probe: File,
        stderr: Option<ChildStderr>,
        initial: Termios,
        output: Vec<u8>,
        stderr_output: Vec<u8>,
        reaped: bool,
        command_master: Option<File>,
        _input: Option<ChildStdin>,
    }

    impl PtyChild {
        fn spawn(path: &Path) -> io::Result<Self> {
            Self::spawn_inner(TestInput::Path(path), None, None)
        }

        fn spawn_logged(
            path: &Path,
            cli_log: Option<&Path>,
            environment_log: Option<&Path>,
        ) -> io::Result<Self> {
            Self::spawn_inner(TestInput::Path(path), cli_log, environment_log)
        }

        fn spawn_standard_input(bytes: &[u8]) -> io::Result<Self> {
            Self::spawn_inner(TestInput::StandardInput(bytes), None, None)
        }

        fn spawn_standard_input_logged(bytes: &[u8], log: &Path) -> io::Result<Self> {
            Self::spawn_inner(TestInput::StandardInput(bytes), None, Some(log))
        }

        fn spawn_open_standard_input() -> io::Result<Self> {
            Self::spawn_inner(TestInput::OpenStandardInput, None, None)
        }

        fn spawn_open_standard_input_without_controlling_terminal() -> io::Result<Self> {
            Self::spawn_inner_with_controlling_terminal(
                TestInput::OpenStandardInput,
                None,
                None,
                false,
            )
        }

        fn spawn_standard_input_with_separate_stdout(bytes: &[u8]) -> io::Result<Self> {
            let (command_master, _command_slave, controlling_name, initial) = open_test_pty()?;
            let probe = command_master.try_clone()?;
            let (master, stdout, _stdout_name, _stdout_initial) = open_test_pty()?;

            let mut command = tut();
            command
                .arg("-")
                .stdin(Stdio::piped())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::piped());
            configure_child_session(&mut command, Some(controlling_name));
            Self::finish_spawn(
                command.spawn()?,
                TestInput::StandardInput(bytes),
                master,
                probe,
                initial,
                Some(command_master),
            )
        }

        fn spawn_unreadable_standard_input() -> io::Result<Self> {
            Self::spawn_inner(TestInput::UnreadableStandardInput, None, None)
        }

        fn spawn_nested_raw_helper(path: &Path) -> io::Result<Self> {
            let (master, terminal_input, controlling_name, initial) = open_test_pty()?;
            let stdout = terminal_input.try_clone()?;
            let probe = master.try_clone()?;
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args([
                    "--exact",
                    "pty::nested_crossterm_raw_owner_is_preserved",
                    "--nocapture",
                ])
                .env(NESTED_RAW_HELPER, path)
                .stdin(Stdio::from(terminal_input))
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::piped());
            configure_child_session(&mut command, Some(controlling_name));
            Self::finish_spawn(
                command.spawn()?,
                TestInput::Path(path),
                master,
                probe,
                initial,
                None,
            )
        }

        fn spawn_inner(
            input: TestInput<'_>,
            cli_log: Option<&Path>,
            environment_log: Option<&Path>,
        ) -> io::Result<Self> {
            Self::spawn_inner_with_controlling_terminal(input, cli_log, environment_log, true)
        }

        fn spawn_inner_with_controlling_terminal(
            input: TestInput<'_>,
            cli_log: Option<&Path>,
            environment_log: Option<&Path>,
            acquire_controlling_terminal: bool,
        ) -> io::Result<Self> {
            let (master, terminal_input, controlling_name, initial) = open_test_pty()?;
            let stdout = terminal_input.try_clone()?;
            let probe = master.try_clone()?;

            let mut command = tut();
            if let Some(path) = environment_log {
                command.env("TUT_LOG_FILE", path);
            }
            match input {
                TestInput::Path(path) => {
                    command.arg(path).stdin(Stdio::from(terminal_input));
                }
                TestInput::StandardInput(_) | TestInput::OpenStandardInput => {
                    command.arg("-").stdin(Stdio::piped());
                }
                TestInput::UnreadableStandardInput => {
                    let unreadable = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
                    command.arg("-").stdin(Stdio::from(unreadable));
                }
            }
            if let Some(path) = cli_log {
                command.arg("--log-file").arg(path);
            }
            command.stdout(Stdio::from(stdout)).stderr(Stdio::piped());
            configure_child_session(
                &mut command,
                acquire_controlling_terminal.then_some(controlling_name),
            );
            Self::finish_spawn(command.spawn()?, input, master, probe, initial, None)
        }

        fn finish_spawn(
            mut child: Child,
            input: TestInput<'_>,
            master: File,
            probe: File,
            initial: Termios,
            command_master: Option<File>,
        ) -> io::Result<Self> {
            let mut input_pipe = child.stdin.take();
            let retained_input = match input {
                TestInput::StandardInput(bytes) => {
                    input_pipe
                        .as_mut()
                        .expect("standard-input children retain their pipe")
                        .write_all(bytes)?;
                    None
                }
                TestInput::OpenStandardInput => input_pipe,
                TestInput::Path(_) | TestInput::UnreadableStandardInput => None,
            };
            let stderr = child.stderr.take();
            Ok(Self {
                child,
                master,
                probe,
                stderr,
                initial,
                output: Vec::new(),
                stderr_output: Vec::new(),
                reaped: false,
                command_master,
                _input: retained_input,
            })
        }

        fn write_command(&mut self, bytes: &[u8]) -> io::Result<()> {
            match &mut self.command_master {
                Some(master) => master.write_all(bytes),
                None => self.master.write_all(bytes),
            }
        }

        fn write_command_with_timeout(&mut self, mut bytes: &[u8]) -> io::Result<()> {
            let deadline = Instant::now() + TIMEOUT;
            while !bytes.is_empty() {
                if Instant::now() >= deadline {
                    self.terminate_and_reap();
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "PTY input timeout"));
                }
                let result = match &mut self.command_master {
                    Some(master) => master.write(bytes),
                    None => self.master.write(bytes),
                };
                match result {
                    Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
                    Ok(count) => bytes = &bytes[count..],
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        self.pump()?;
                        if self.child.try_wait()?.is_some() {
                            self.reaped = true;
                            self.read_stderr()?;
                            return Err(io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "PTY child exited before accepting terminal input",
                            ));
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }

        fn pump(&mut self) -> io::Result<()> {
            let mut buffer = [0_u8; 8192];
            loop {
                match self.master.read(&mut buffer) {
                    Ok(0) => return Ok(()),
                    Ok(count) => self.output.extend_from_slice(&buffer[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                    Err(error) if error.raw_os_error() == Some(5) => return Ok(()),
                    Err(error) => return Err(error),
                }
            }
        }

        fn wait_for(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.wait_for_after(0, bytes)
        }

        fn wait_for_after(&mut self, start: usize, bytes: &[u8]) -> io::Result<()> {
            self.wait_for_after_with_timeout(start, bytes, TIMEOUT)
        }

        fn wait_for_after_with_timeout(
            &mut self,
            start: usize,
            bytes: &[u8],
            timeout: Duration,
        ) -> io::Result<()> {
            let deadline = Instant::now() + timeout;
            loop {
                self.pump()?;
                if self
                    .output
                    .get(start..)
                    .unwrap_or_default()
                    .windows(bytes.len())
                    .any(|window| window == bytes)
                {
                    return Ok(());
                }
                if self.child.try_wait()?.is_some() {
                    self.reaped = true;
                    self.read_stderr()?;
                    return Err(io::Error::other(format!(
                        "child exited before expected terminal output: {}",
                        String::from_utf8_lossy(&self.stderr_output)
                    )));
                }
                if Instant::now() >= deadline {
                    self.terminate_and_reap();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "PTY output timeout",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn wait_until_stopped(&mut self) -> io::Result<()> {
            let pid = self.pid()?;
            let deadline = Instant::now() + TIMEOUT;
            loop {
                self.pump()?;
                match waitpid(Some(pid), WaitOptions::NOHANG | WaitOptions::UNTRACED)
                    .map_err(io::Error::from)?
                {
                    Some((_, status)) if status.stopped() => {
                        self.pump()?;
                        return Ok(());
                    }
                    Some(_) => {
                        self.reaped = true;
                        self.read_stderr()?;
                        return Err(io::Error::other("child exited before suspension"));
                    }
                    None => {}
                }
                if Instant::now() >= deadline {
                    self.terminate_and_reap();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "PTY suspension timeout",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn wait(&mut self) -> io::Result<ExitStatus> {
            let deadline = Instant::now() + TIMEOUT;
            loop {
                self.pump()?;
                if let Some(status) = self.child.try_wait()? {
                    self.reaped = true;
                    self.read_stderr()?;
                    self.pump()?;
                    return Ok(status);
                }
                if Instant::now() >= deadline {
                    self.terminate_and_reap();
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "PTY child timeout"));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        fn read_stderr(&mut self) -> io::Result<()> {
            if let Some(mut stderr) = self.stderr.take() {
                stderr.read_to_end(&mut self.stderr_output)?;
            }
            Ok(())
        }

        fn pid(&self) -> io::Result<Pid> {
            Pid::from_raw(
                i32::try_from(self.child.id())
                    .map_err(|_| io::Error::other("child pid overflow"))?,
            )
            .ok_or_else(|| io::Error::other("invalid child pid"))
        }

        fn signal(&self, signal: Signal) -> io::Result<()> {
            kill_process(self.pid()?, signal).map_err(io::Error::from)
        }

        fn assert_restored(&self) {
            let after = termios::tcgetattr(&self.probe).unwrap();
            assert_terminal_configuration(&after, &self.initial);
        }

        fn terminate_and_reap(&mut self) {
            if self.reaped {
                return;
            }
            match self.child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                }
            }
            self.reaped = true;
        }
    }

    impl Drop for PtyChild {
        fn drop(&mut self) {
            self.terminate_and_reap();
        }
    }

    fn configure_child_session(command: &mut Command, controlling_name: Option<CString>) {
        // The child setup uses only async-signal-safe descriptor and session operations.
        unsafe {
            command.pre_exec(move || {
                reset_signal_state()?;
                setsid().map_err(io::Error::from)?;
                if let Some(name) = &controlling_name {
                    let controlling = fs::open(name.as_c_str(), OFlags::RDWR, Mode::empty())
                        .map_err(io::Error::from)?;
                    ioctl_tiocsctty(&controlling).map_err(io::Error::from)?;
                    termios::tcsetpgrp(&controlling, getpgrp()).map_err(io::Error::from)?;
                    drop(controlling);
                }
                Ok(())
            });
        }
    }

    fn assert_terminal_configuration(actual: &Termios, expected: &Termios) {
        assert_eq!(actual.input_modes, expected.input_modes);
        assert_eq!(actual.output_modes, expected.output_modes);
        assert_eq!(actual.control_modes, expected.control_modes);
        let mut actual_local_modes = actual.local_modes;
        let mut expected_local_modes = expected.local_modes;
        // PENDIN and FLUSHO report transient line-discipline state rather than saved
        // terminal configuration, so the kernel may change them while settings restore.
        let state_modes = termios::LocalModes::PENDIN | termios::LocalModes::FLUSHO;
        actual_local_modes.remove(state_modes);
        expected_local_modes.remove(state_modes);
        assert_eq!(actual_local_modes, expected_local_modes);
        assert_eq!(actual.input_speed(), expected.input_speed());
        assert_eq!(actual.output_speed(), expected.output_speed());
        for index in [
            SpecialCodeIndex::VINTR,
            SpecialCodeIndex::VQUIT,
            SpecialCodeIndex::VERASE,
            SpecialCodeIndex::VKILL,
            SpecialCodeIndex::VEOF,
            SpecialCodeIndex::VTIME,
            SpecialCodeIndex::VMIN,
            SpecialCodeIndex::VSTART,
            SpecialCodeIndex::VSTOP,
            SpecialCodeIndex::VSUSP,
            SpecialCodeIndex::VEOL,
            SpecialCodeIndex::VEOL2,
        ] {
            assert_eq!(actual.special_codes[index], expected.special_codes[index]);
        }
        #[cfg(target_os = "linux")]
        assert_eq!(actual.line_discipline, expected.line_discipline);
    }

    fn assert_session_terminal_restored(pty: &PtyChild) {
        for sequence in [ENTER_ALT, HIDE_CURSOR, SHOW_CURSOR, LEAVE_ALT] {
            assert!(
                pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }

    fn open_test_pty() -> io::Result<(File, File, CString, Termios)> {
        let master_fd = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        grantpt(&master_fd)?;
        unlockpt(&master_fd)?;
        let slave_name = ptsname(&master_fd, Vec::new())?;
        let slave_fd = fs::open(
            slave_name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )?;
        termios::tcsetwinsize(
            &slave_fd,
            Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        let initial = termios::tcgetattr(&slave_fd)?;
        let flags = fs::fcntl_getfl(&master_fd)?;
        fs::fcntl_setfl(&master_fd, flags | OFlags::NONBLOCK)?;
        Ok((
            File::from(master_fd),
            File::from(slave_fd),
            slave_name,
            initial,
        ))
    }

    struct RawModeGuard;

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
        }
    }

    fn exercise_nested_raw_rejection(path: &Path) {
        let terminal = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .unwrap();
        let initial = termios::tcgetattr(&terminal).unwrap();
        enable_raw_mode().unwrap();
        let guard = RawModeGuard;
        assert!(is_raw_mode_enabled().unwrap());
        let owned = termios::tcgetattr(&terminal).unwrap();

        let error = tut::run([path.as_os_str().to_owned()]).unwrap_err();

        assert!(matches!(error, tut::TutError::TerminalInUse));
        assert!(is_raw_mode_enabled().unwrap());
        let after = termios::tcgetattr(&terminal).unwrap();
        assert_terminal_configuration(&after, &owned);
        drop(guard);
        assert!(!is_raw_mode_enabled().unwrap());
        let restored = termios::tcgetattr(&terminal).unwrap();
        assert_terminal_configuration(&restored, &initial);
    }

    #[test]
    fn nested_crossterm_raw_owner_is_preserved() {
        if let Some(path) = std::env::var_os(NESTED_RAW_HELPER) {
            exercise_nested_raw_rejection(Path::new(&path));
            return;
        }

        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "host-owned terminal\n").unwrap();
        let mut pty = PtyChild::spawn_nested_raw_helper(file.path()).unwrap();

        assert_eq!(pty.wait().unwrap().code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        for sequence in [ENTER_ALT, LEAVE_ALT, HIDE_CURSOR, SHOW_CURSOR] {
            assert!(
                !pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }

    #[test]
    fn normal_quit_and_keyboard_interrupt_restore_the_terminal() {
        let file = NamedTempFile::new().unwrap();
        let directory = tempdir().unwrap();
        std::fs::write(file.path(), "line one\nline two\n").unwrap();
        for (name, input, code, diagnostic, summary) in [
            (
                "quit",
                b"q".as_slice(),
                0,
                b"".as_slice(),
                "session_summary outcome=normal ",
            ),
            (
                "interrupt",
                b"\x03".as_slice(),
                130,
                b"tut: interrupted by SIGINT\n".as_slice(),
                "session_summary outcome=signal signal=SIGINT ",
            ),
        ] {
            let log = directory.path().join(format!("{name}.log"));
            let mut pty = PtyChild::spawn_logged(file.path(), None, Some(&log)).unwrap();
            pty.wait_for(HIDE_CURSOR).unwrap();
            pty.master.write_all(input).unwrap();
            let status = pty.wait().unwrap();
            assert_eq!(status.code(), Some(code));
            assert_eq!(pty.stderr_output, diagnostic);
            for sequence in [ENTER_ALT, HIDE_CURSOR, SHOW_CURSOR, LEAVE_ALT] {
                assert!(
                    pty.output
                        .windows(sequence.len())
                        .any(|window| window == sequence)
                );
            }
            pty.assert_restored();
            assert!(std::fs::read_to_string(log).unwrap().contains(summary));
        }
    }

    #[test]
    fn navigation_and_search_work_through_a_real_terminal() {
        let file = NamedTempFile::new().unwrap();
        let mut text = String::new();
        for line in 0..100 {
            match line {
                0 => text.push_str("START_SENTINEL\n"),
                30 => text.push_str("hit ALPHA_SENTINEL\n"),
                70 => text.push_str("hit BETA_SENTINEL\n"),
                99 => text.push_str("END_SENTINEL\n"),
                _ => text.push_str("ordinary line\n"),
            }
        }
        std::fs::write(file.path(), text).unwrap();

        let mut pty = PtyChild::spawn(file.path()).unwrap();
        pty.wait_for(b"START_SENTINEL").unwrap();
        pty.master.write_all(b"G").unwrap();
        pty.wait_for(b"END_SENTINEL").unwrap();
        pty.master.write_all(b"/hit\r").unwrap();
        pty.wait_for(b"ALPHA_SENTINEL").unwrap();
        pty.master.write_all(b"n").unwrap();
        pty.wait_for(b"BETA_SENTINEL").unwrap();
        pty.master.write_all(b"q").unwrap();

        let status = pty.wait().unwrap();
        assert_eq!(status.code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        pty.assert_restored();
    }

    #[test]
    fn oversized_terminal_sequence_hits_a_bound_and_restores_the_terminal() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "bounded input\n").unwrap();
        let directory = tempdir().unwrap();
        let log = directory.path().join("session.log");
        let mut pty = PtyChild::spawn_logged(file.path(), Some(&log), None).unwrap();
        pty.wait_for(HIDE_CURSOR).unwrap();

        let mut sequence = vec![b'1'; 64 * 1024 + 1];
        sequence[..2].copy_from_slice(b"\x1b[");
        pty.write_command_with_timeout(&sequence).unwrap();

        assert_eq!(pty.wait().unwrap().code(), Some(1));
        assert!(
            pty.stderr_output
                == b"tut: failed to poll terminal events: terminal input sequence exceeded 65536 bytes\n"
                || pty.stderr_output
                    == b"tut: failed to poll terminal events: terminal input sequence did not finish within 2 seconds\n",
            "unexpected terminal input bound: {:?}",
            String::from_utf8_lossy(&pty.stderr_output)
        );
        assert_session_terminal_restored(&pty);
        assert!(
            std::fs::read_to_string(log)
                .unwrap()
                .contains("session_summary outcome=error ")
        );
    }

    #[test]
    fn orphan_bracketed_paste_times_out_and_restores_the_terminal() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "bounded input\n").unwrap();
        let mut pty = PtyChild::spawn(file.path()).unwrap();
        pty.wait_for(HIDE_CURSOR).unwrap();

        let started = Instant::now();
        pty.master.write_all(b"\x1b[200~").unwrap();

        assert_eq!(pty.wait().unwrap().code(), Some(1));
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(1_500));
        assert!(
            elapsed < Duration::from_secs(4),
            "orphan paste timeout exceeded its scheduling allowance: {elapsed:?}"
        );
        assert_eq!(
            pty.stderr_output,
            b"tut: failed to poll terminal events: terminal input sequence did not finish within 2 seconds\n"
        );
        assert_session_terminal_restored(&pty);
    }

    #[test]
    fn internal_event_flood_and_unknown_csi_do_not_wedge_input() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "bounded input\n").unwrap();
        let mut pty = PtyChild::spawn(file.path()).unwrap();
        pty.wait_for(HIDE_CURSOR).unwrap();

        let mut input = Vec::with_capacity(4_097 * b"\x1b[1;1R".len() + 11);
        for _ in 0..4_097 {
            input.extend_from_slice(b"\x1b[1;1R");
        }
        input.extend_from_slice(b"\x1b[?997;1nq");
        pty.write_command_with_timeout(&input).unwrap();

        assert_eq!(pty.wait().unwrap().code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        assert_session_terminal_restored(&pty);
    }

    #[test]
    fn session_logs_are_private_typed_and_cli_configuration_wins() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("private-title.txt");
        let cli_log = directory.path().join("cli.log");
        let environment_log = directory.path().join("environment.log");
        std::fs::write(&input, "PRIVATE_CONTENT_SENTINEL\n").unwrap();

        let mut pty =
            PtyChild::spawn_logged(&input, Some(&cli_log), Some(&environment_log)).unwrap();
        pty.wait_for(b"PRIVATE_CONTENT_SENTINEL").unwrap();
        pty.master.write_all(b"q").unwrap();
        let status = pty.wait().unwrap();

        assert_eq!(status.code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        pty.assert_restored();
        assert!(!environment_log.exists());
        let log = std::fs::read_to_string(cli_log).unwrap();
        assert!(log.is_ascii());
        assert!(log.starts_with("schema version=1\nsession_start input=path source_bytes=25\n"));
        assert!(log.contains("session_summary outcome=normal elapsed_us="));
        assert!(log.ends_with(" terminal_sessions=1 suspensions=0\n"));
        assert!(!log.contains("private-title"));
        assert!(!log.contains("PRIVATE_CONTENT_SENTINEL"));
    }

    #[test]
    fn session_logs_cannot_modify_the_input_document() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "unchanged").unwrap();
        let mut pty = PtyChild::spawn_logged(file.path(), Some(file.path()), None).unwrap();
        let status = pty.wait().unwrap();

        assert_eq!(status.code(), Some(1));
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "unchanged");
        assert!(
            String::from_utf8_lossy(&pty.stderr_output)
                .starts_with("tut: session log is the input document:")
        );
        assert!(
            !pty.output
                .windows(ENTER_ALT.len())
                .any(|window| window == ENTER_ALT)
        );
        pty.assert_restored();
    }

    #[test]
    fn piped_standard_input_uses_the_controlling_terminal_for_commands() {
        let directory = tempdir().unwrap();
        let log = directory.path().join("session.log");
        let mut text = String::new();
        for line in 0..60 {
            match line {
                0 => text.push_str("PIPE_START_SENTINEL\n"),
                59 => text.push_str("PIPE_END_SENTINEL\n"),
                _ => text.push_str("ordinary piped line\n"),
            }
        }
        let source_bytes = text.len();

        let mut pty = PtyChild::spawn_standard_input_logged(text.as_bytes(), &log).unwrap();
        pty.wait_for(b"PIPE_START_SENTINEL").unwrap();
        pty.master.write_all(b"G").unwrap();
        pty.wait_for(b"PIPE_END_SENTINEL").unwrap();
        pty.master.write_all(b"q").unwrap();

        let status = pty.wait().unwrap();
        assert_eq!(status.code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        pty.assert_restored();
        let log = std::fs::read_to_string(log).unwrap();
        assert!(log.contains(&format!(
            "session_start input=stdin source_bytes={source_bytes}\n"
        )));
        assert!(log.ends_with(" terminal_sessions=1 suspensions=0\n"));
    }

    #[test]
    fn piped_standard_input_uses_the_controlling_pty_not_the_stdout_pty() {
        let mut text = String::new();
        for line in 0..60 {
            match line {
                0 => text.push_str("CONTROL_PTY_START_SENTINEL\n"),
                59 => text.push_str("CONTROL_PTY_END_SENTINEL\n"),
                _ => text.push_str("ordinary piped line\n"),
            }
        }

        let mut pty = PtyChild::spawn_standard_input_with_separate_stdout(text.as_bytes()).unwrap();
        pty.wait_for(b"CONTROL_PTY_START_SENTINEL").unwrap();

        pty.master.write_all(b"q").unwrap();
        thread::sleep(Duration::from_millis(150));
        assert!(
            pty.child.try_wait().unwrap().is_none(),
            "input written to the stdout PTY must not quit the child"
        );

        pty.write_command(b"G").unwrap();
        pty.wait_for(b"CONTROL_PTY_END_SENTINEL").unwrap();
        pty.write_command(b"q").unwrap();

        assert_eq!(pty.wait().unwrap().code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        pty.assert_restored();
    }

    #[test]
    fn missing_controlling_terminal_fails_before_reading_standard_input() {
        let mut pty = PtyChild::spawn_open_standard_input_without_controlling_terminal().unwrap();
        let status = pty.wait().unwrap();

        assert_eq!(status.code(), Some(1));
        assert_eq!(
            pty.stderr_output,
            b"tut: interactive reading requires terminal input and output\n"
        );
        for sequence in [ENTER_ALT, HIDE_CURSOR] {
            assert!(
                !pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }

    #[test]
    fn invalid_standard_input_fails_before_terminal_mutation() {
        let mut pty = PtyChild::spawn_standard_input(b"ok\xffbad").unwrap();
        let status = pty.wait().unwrap();
        assert_eq!(status.code(), Some(1));
        assert_eq!(
            pty.stderr_output,
            b"tut: invalid UTF-8 in standard input at byte 2\n"
        );
        for sequence in [ENTER_ALT, HIDE_CURSOR] {
            assert!(
                !pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }

    #[test]
    fn signals_terminate_a_blocked_standard_input_read() {
        let mut pty = PtyChild::spawn_open_standard_input().unwrap();
        thread::sleep(Duration::from_millis(50));
        pty.signal(Signal::TERM).unwrap();
        let status = pty.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        assert!(pty.stderr_output.is_empty());
        for sequence in [ENTER_ALT, HIDE_CURSOR] {
            assert!(
                !pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }

    #[test]
    fn unreadable_standard_input_fails_without_reading_from_the_terminal() {
        let mut pty = PtyChild::spawn_unreadable_standard_input().unwrap();
        let status = pty.wait().unwrap();
        assert_eq!(status.code(), Some(1));
        assert!(
            pty.stderr_output
                .starts_with(b"tut: cannot read standard input: ")
        );
        for sequence in [ENTER_ALT, HIDE_CURSOR] {
            assert!(
                !pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }

    #[test]
    fn external_signals_restore_before_reporting_the_signal() {
        let file = NamedTempFile::new().unwrap();
        let directory = tempdir().unwrap();
        std::fs::write(file.path(), "text").unwrap();
        for (signal, code, name, diagnostic) in [
            (
                Signal::HUP,
                129,
                "SIGHUP",
                b"tut: interrupted by SIGHUP\n".as_slice(),
            ),
            (
                Signal::INT,
                130,
                "SIGINT",
                b"tut: interrupted by SIGINT\n".as_slice(),
            ),
            (
                Signal::QUIT,
                131,
                "SIGQUIT",
                b"tut: interrupted by SIGQUIT\n".as_slice(),
            ),
            (
                Signal::TERM,
                143,
                "SIGTERM",
                b"tut: interrupted by SIGTERM\n".as_slice(),
            ),
        ] {
            let log = directory.path().join(format!("{name}.log"));
            let mut pty = PtyChild::spawn_logged(file.path(), None, Some(&log)).unwrap();
            pty.wait_for(HIDE_CURSOR).unwrap();
            pty.signal(signal).unwrap();
            let status = pty.wait().unwrap();
            assert_eq!(status.code(), Some(code));
            assert_eq!(pty.stderr_output, diagnostic);
            assert!(
                pty.output
                    .windows(SHOW_CURSOR.len())
                    .any(|window| window == SHOW_CURSOR)
            );
            assert!(
                pty.output
                    .windows(LEAVE_ALT.len())
                    .any(|window| window == LEAVE_ALT)
            );
            pty.assert_restored();
            let log = std::fs::read_to_string(log).unwrap();
            assert!(log.contains(&format!("session_summary outcome=signal signal={name} ")));
            assert!(log.ends_with(" terminal_sessions=1 suspensions=0\n"));
        }
    }

    #[test]
    fn suspension_restores_and_continuation_reenters_the_terminal() {
        let directory = tempdir().unwrap();
        let input = directory.path().join("input.txt");
        let log = directory.path().join("session.log");
        std::fs::write(&input, "SUSPEND_SENTINEL\n").unwrap();
        let mut pty = PtyChild::spawn_logged(&input, None, Some(&log)).unwrap();
        pty.wait_for(b"SUSPEND_SENTINEL").unwrap();

        let suspension_start = pty.output.len();
        pty.signal(Signal::TSTP).unwrap();
        pty.wait_until_stopped().unwrap();
        pty.wait_for_after(suspension_start, SHOW_CURSOR).unwrap();
        pty.wait_for_after(suspension_start, LEAVE_ALT).unwrap();
        pty.assert_restored();

        let continuation_start = pty.output.len();
        pty.signal(Signal::CONT).unwrap();
        pty.wait_for_after(continuation_start, ENTER_ALT).unwrap();
        pty.wait_for_after(continuation_start, HIDE_CURSOR).unwrap();
        pty.wait_for_after(continuation_start, b"SUSPEND_SENTINEL")
            .unwrap();
        pty.master.write_all(b"q").unwrap();

        let status = pty.wait().unwrap();
        assert_eq!(status.code(), Some(0));
        assert!(pty.stderr_output.is_empty());
        pty.assert_restored();
        let log = std::fs::read_to_string(log).unwrap();
        assert!(log.ends_with(" terminal_sessions=2 suspensions=1\n"));
    }

    #[test]
    fn stopped_reader_uses_inherited_default_termination_actions() {
        let input = NamedTempFile::new().unwrap();
        std::fs::write(input.path(), "STOPPED_SIGNAL_SENTINEL\n").unwrap();

        for signal in [Signal::HUP, Signal::INT, Signal::QUIT, Signal::TERM] {
            let mut pty = PtyChild::spawn(input.path()).unwrap();
            pty.wait_for(b"STOPPED_SIGNAL_SENTINEL").unwrap();

            let suspension_start = pty.output.len();
            pty.signal(Signal::TSTP).unwrap();
            pty.wait_until_stopped().unwrap();
            pty.wait_for_after(suspension_start, SHOW_CURSOR).unwrap();
            pty.wait_for_after(suspension_start, LEAVE_ALT).unwrap();
            pty.assert_restored();

            pty.signal(signal).unwrap();
            #[cfg(target_os = "linux")]
            {
                thread::sleep(Duration::from_millis(50));
                assert!(
                    pty.child.try_wait().unwrap().is_none(),
                    "a fatal signal must remain pending while Linux holds the group-stop"
                );
                pty.pump().unwrap();
                assert!(
                    !pty.output[suspension_start..]
                        .windows(ENTER_ALT.len())
                        .any(|window| window == ENTER_ALT)
                );
                pty.signal(Signal::CONT).unwrap();
            }
            let status = pty.wait().unwrap();
            assert_eq!(status.signal(), Some(signal.as_raw()));
            assert!(pty.stderr_output.is_empty());
            assert!(
                !pty.output[suspension_start..]
                    .windows(ENTER_ALT.len())
                    .any(|window| window == ENTER_ALT)
            );
            pty.assert_restored();
        }
    }

    #[test]
    fn maximum_grapheme_rendering_remains_responsive_to_signals_and_job_control() {
        const MAXIMUM_CLUSTERS: usize = 32;
        const PTY_CLUSTERS: usize = 8;
        let mut input = NamedTempFile::new().unwrap();
        let cluster_bytes = usize::try_from(tut::MAX_FILE_BYTES).unwrap() / MAXIMUM_CLUSTERS;
        let mut cluster = String::from('é');
        cluster.extend(std::iter::repeat_n(
            '\u{301}',
            (cluster_bytes - 'é'.len_utf8()) / '\u{301}'.len_utf8(),
        ));
        assert_eq!(cluster.len(), cluster_bytes);
        for _ in 0..PTY_CLUSTERS {
            input.as_file_mut().write_all(cluster.as_bytes()).unwrap();
        }
        input.as_file_mut().flush().unwrap();
        assert_eq!(
            input.as_file().metadata().unwrap().len(),
            u64::try_from(cluster_bytes * PTY_CLUSTERS).unwrap()
        );

        let mut terminated = PtyChild::spawn(input.path()).unwrap();
        terminated.wait_for(HIDE_CURSOR).unwrap();
        thread::sleep(Duration::from_millis(20));
        terminated.signal(Signal::TERM).unwrap();
        let status = terminated.wait().unwrap();
        assert_eq!(status.code(), Some(143));
        assert_eq!(terminated.stderr_output, b"tut: interrupted by SIGTERM\n");
        assert!(
            terminated
                .output
                .windows(SHOW_CURSOR.len())
                .any(|window| window == SHOW_CURSOR)
        );
        assert!(
            terminated
                .output
                .windows(LEAVE_ALT.len())
                .any(|window| window == LEAVE_ALT)
        );
        terminated.assert_restored();

        let mut suspended = PtyChild::spawn(input.path()).unwrap();
        suspended.wait_for(HIDE_CURSOR).unwrap();
        thread::sleep(Duration::from_millis(20));
        let suspension_start = suspended.output.len();
        suspended.signal(Signal::TSTP).unwrap();
        suspended.wait_until_stopped().unwrap();
        suspended
            .wait_for_after(suspension_start, SHOW_CURSOR)
            .unwrap();
        suspended
            .wait_for_after(suspension_start, LEAVE_ALT)
            .unwrap();
        suspended.assert_restored();

        let continuation_start = suspended.output.len();
        suspended.signal(Signal::CONT).unwrap();
        suspended
            .wait_for_after(continuation_start, ENTER_ALT)
            .unwrap();
        suspended
            .wait_for_after(continuation_start, HIDE_CURSOR)
            .unwrap();
        suspended
            .wait_for_after_with_timeout(
                continuation_start,
                "\u{fffd}".as_bytes(),
                Duration::from_secs(15),
            )
            .unwrap();
        suspended.master.write_all(b"q").unwrap();
        assert_eq!(suspended.wait().unwrap().code(), Some(0));
        assert!(suspended.stderr_output.is_empty());
        suspended.assert_restored();
    }

    #[test]
    fn file_errors_happen_before_terminal_mutation() {
        let directory = tempdir().unwrap();
        let oversize = directory.path().join("oversize.txt");
        File::create(&oversize)
            .unwrap()
            .set_len(tut::MAX_FILE_BYTES + 1)
            .unwrap();
        let missing = directory.path().join("missing.txt");

        for path in [&missing, &oversize] {
            let mut pty = PtyChild::spawn(path).unwrap();
            let status = pty.wait().unwrap();
            assert_eq!(status.code(), Some(1));
            assert!(!pty.stderr_output.is_empty());
            assert!(
                !pty.output
                    .windows(ENTER_ALT.len())
                    .any(|window| window == ENTER_ALT)
            );
            pty.assert_restored();
        }
    }

    #[test]
    fn deferred_validation_errors_restore_the_terminal() {
        let file = NamedTempFile::new().unwrap();
        let invalid_offset = tut::MAX_FILE_BYTES.min(2 * 64 * 1024) + 10;
        let mut bytes = vec![b'a'; usize::try_from(invalid_offset).unwrap()];
        bytes.push(0xff);
        std::fs::write(file.path(), bytes).unwrap();

        let mut pty = PtyChild::spawn(file.path()).unwrap();
        let status = pty.wait().unwrap();
        assert_eq!(status.code(), Some(1));
        assert!(
            String::from_utf8_lossy(&pty.stderr_output)
                .contains(&format!("invalid UTF-8 at byte {invalid_offset}"))
        );
        for sequence in [ENTER_ALT, HIDE_CURSOR, SHOW_CURSOR, LEAVE_ALT] {
            assert!(
                pty.output
                    .windows(sequence.len())
                    .any(|window| window == sequence)
            );
        }
        pty.assert_restored();
    }
}
