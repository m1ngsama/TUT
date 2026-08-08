use std::{fs, process::Command};

use tempfile::{NamedTempFile, tempdir};

fn tut() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tut"))
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
}

#[test]
fn invalid_invocations_use_gnu_diagnostics_and_exit_two() {
    for (arguments, message) in [
        (Vec::<&str>::new(), "missing file operand"),
        (vec!["a", "b"], "extra operand 'b'"),
        (vec!["--unknown"], "unrecognized option '--unknown'"),
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
        b"tut: interactive reading requires terminal stdin and stdout\n"
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
        b"tut: interactive reading requires terminal stdin and stdout\n"
    );

    let file = NamedTempFile::new().unwrap();
    fs::write(file.path(), "valid UTF-8").unwrap();
    let valid = tut().arg(file.path()).output().unwrap();
    assert_eq!(valid.status.code(), Some(1));
    assert!(valid.stdout.is_empty());
    assert_eq!(valid.stderr, missing.stderr);
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
        fs::File,
        io::{self, Read, Write},
        os::unix::process::CommandExt,
        path::Path,
        process::{Child, ChildStderr, ExitStatus, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use rustix::{
        fs::{self, Mode, OFlags},
        process::{Pid, Signal, kill_process, setsid},
        pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
        termios::{self, SpecialCodeIndex, Termios, Winsize},
    };
    use tempfile::{NamedTempFile, tempdir};

    use super::tut;

    const ENTER_ALT: &[u8] = b"\x1b[?1049h";
    const LEAVE_ALT: &[u8] = b"\x1b[?1049l";
    const HIDE_CURSOR: &[u8] = b"\x1b[?25l";
    const SHOW_CURSOR: &[u8] = b"\x1b[?25h";
    const TIMEOUT: Duration = Duration::from_secs(5);

    struct PtyChild {
        child: Child,
        master: File,
        probe: File,
        stderr: Option<ChildStderr>,
        initial: Termios,
        output: Vec<u8>,
        stderr_output: Vec<u8>,
        reaped: bool,
    }

    impl PtyChild {
        fn spawn(path: &Path) -> io::Result<Self> {
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
            let stdin = File::from(slave_fd);
            let stdout = stdin.try_clone()?;

            let flags = fs::fcntl_getfl(&master_fd)?;
            fs::fcntl_setfl(&master_fd, flags | OFlags::NONBLOCK)?;
            let master = File::from(master_fd);
            let probe = master.try_clone()?;
            let controlling_name = slave_name;

            let mut command = tut();
            command
                .arg(path)
                .stdin(Stdio::from(stdin))
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::piped());
            // The child setup uses only async-signal-safe descriptor and session operations.
            unsafe {
                command.pre_exec(move || {
                    setsid().map_err(io::Error::from)?;
                    let controlling =
                        fs::open(controlling_name.as_c_str(), OFlags::RDWR, Mode::empty())
                            .map_err(io::Error::from)?;
                    drop(controlling);
                    Ok(())
                });
            }
            let mut child = command.spawn()?;
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
            })
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
            let deadline = Instant::now() + TIMEOUT;
            loop {
                self.pump()?;
                if self
                    .output
                    .windows(bytes.len())
                    .any(|window| window == bytes)
                {
                    return Ok(());
                }
                if self.child.try_wait()?.is_some() {
                    self.reaped = true;
                    self.read_stderr()?;
                    return Err(io::Error::other(
                        "child exited before expected terminal output",
                    ));
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

        fn signal(&self, signal: Signal) -> io::Result<()> {
            let pid = Pid::from_raw(
                i32::try_from(self.child.id())
                    .map_err(|_| io::Error::other("child pid overflow"))?,
            )
            .ok_or_else(|| io::Error::other("invalid child pid"))?;
            kill_process(pid, signal).map_err(io::Error::from)
        }

        fn assert_restored(&self) {
            let after = termios::tcgetattr(&self.probe).unwrap();
            assert_eq!(after.input_modes, self.initial.input_modes);
            assert_eq!(after.output_modes, self.initial.output_modes);
            assert_eq!(after.control_modes, self.initial.control_modes);
            assert_eq!(after.local_modes, self.initial.local_modes);
            assert_eq!(after.input_speed(), self.initial.input_speed());
            assert_eq!(after.output_speed(), self.initial.output_speed());
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
                assert_eq!(
                    after.special_codes[index],
                    self.initial.special_codes[index]
                );
            }
            #[cfg(target_os = "linux")]
            assert_eq!(after.line_discipline, self.initial.line_discipline);
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

    #[test]
    fn normal_quit_and_keyboard_interrupt_restore_the_terminal() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "line one\nline two\n").unwrap();
        for input in [b"q".as_slice(), b"\x03".as_slice()] {
            let mut pty = PtyChild::spawn(file.path()).unwrap();
            pty.wait_for(HIDE_CURSOR).unwrap();
            pty.master.write_all(input).unwrap();
            let status = pty.wait().unwrap();
            assert_eq!(status.code(), Some(0));
            assert!(pty.stderr_output.is_empty());
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

    #[test]
    fn external_signals_restore_before_reporting_the_signal() {
        let file = NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "text").unwrap();
        for (signal, code, diagnostic) in [
            (Signal::HUP, 129, b"tut: interrupted by SIGHUP\n".as_slice()),
            (Signal::INT, 130, b"tut: interrupted by SIGINT\n".as_slice()),
            (
                Signal::TERM,
                143,
                b"tut: interrupted by SIGTERM\n".as_slice(),
            ),
        ] {
            let mut pty = PtyChild::spawn(file.path()).unwrap();
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
        }
    }

    #[test]
    fn file_errors_happen_before_terminal_mutation() {
        let directory = tempdir().unwrap();
        let invalid = directory.path().join("invalid.txt");
        std::fs::write(&invalid, b"ok\xffbad").unwrap();
        let oversize = directory.path().join("oversize.txt");
        File::create(&oversize)
            .unwrap()
            .set_len((tut::MAX_FILE_BYTES + 1) as u64)
            .unwrap();
        let missing = directory.path().join("missing.txt");

        for path in [&missing, &invalid, &oversize] {
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
}
