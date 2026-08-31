use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd},
    time::{Duration, Instant},
};

use super::input::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

const MAX_SEQUENCE_BYTES: usize = 64 * 1024;
const SEQUENCE_TIMEOUT: Duration = Duration::from_secs(2);
const ESCAPE_TIMEOUT: Duration = Duration::from_millis(25);
const READ_CHUNK_BYTES: usize = 8 * 1024;

pub(super) struct TerminalInput {
    input: File,
    screen: File,
    original_mode: Option<libc::termios>,
    buffer: Vec<u8>,
    start: usize,
    ready: Option<Event>,
    incomplete_since: Option<Instant>,
    last_size: (u16, u16),
}

impl TerminalInput {
    pub(super) fn open() -> io::Result<Self> {
        let input = terminal_input()?;
        let screen = screen_terminal()?;
        let last_size = terminal_size(&screen)?;
        Ok(Self {
            input,
            screen,
            original_mode: None,
            buffer: Vec::with_capacity(READ_CHUNK_BYTES),
            start: 0,
            ready: None,
            incomplete_since: None,
            last_size,
        })
    }

    pub(super) fn screen_clone(&self) -> io::Result<File> {
        self.screen.try_clone()
    }

    pub(super) fn size(&self) -> io::Result<(u16, u16)> {
        terminal_size(&self.screen)
    }

    pub(super) fn record_size(&mut self, size: (u16, u16)) {
        self.last_size = size;
    }

    pub(super) fn discard_buffered_events(&mut self) {
        self.ready = None;
        self.buffer.clear();
        self.start = 0;
        self.incomplete_since = None;
    }

    pub(super) fn raw_mode_enabled(&self) -> io::Result<bool> {
        terminal_mode(&self.input).map(|mode| mode_is_raw(&mode))
    }

    pub(super) fn enable_raw_mode(&mut self) -> io::Result<()> {
        if self.original_mode.is_some() {
            return Ok(());
        }

        let original = terminal_mode(&self.input)?;
        let mut raw = original;
        // SAFETY: raw is initialized termios storage owned by this function.
        unsafe { libc::cfmakeraw(&mut raw) };
        // Keep Unix-generated Ctrl-C, Ctrl-Z, and related job-control signals active.
        raw.c_lflag |= libc::ISIG;
        set_terminal_mode(&self.input, &raw)?;
        self.original_mode = Some(original);
        Ok(())
    }

    pub(super) fn disable_raw_mode(&mut self) -> io::Result<()> {
        let Some(original) = self.original_mode.as_ref() else {
            return Ok(());
        };
        set_terminal_mode(&self.input, original)?;
        self.original_mode = None;
        Ok(())
    }

    pub(super) fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        if self.ready.is_some() {
            return Ok(true);
        }

        let call_started = Instant::now();
        let call_deadline = call_started.checked_add(timeout);
        let mut checked_after_deadline = false;
        loop {
            if self.prepare_event()? {
                return Ok(true);
            }

            let size = self.size()?;
            if size != self.last_size {
                self.last_size = size;
                self.ready = Some(Event::Resize(size.0, size.1));
                return Ok(true);
            }

            let now = Instant::now();
            // A zero timeout still means one nonblocking descriptor check. The event loop uses
            // that path while background work is pending so keyboard input can preempt it.
            let call_remaining = remaining_until(call_deadline, now);
            let call_expired = call_remaining.is_none();
            if call_expired && checked_after_deadline {
                return Ok(false);
            }
            checked_after_deadline |= call_expired;
            let call_remaining = call_remaining.unwrap_or(Duration::ZERO);
            let wait = call_remaining.min(self.sequence_wait(now)?);
            if !poll_readable(&self.input, wait)? {
                if self.prepare_event()? {
                    return Ok(true);
                }
                if call_expired || remaining_until(call_deadline, Instant::now()).is_none() {
                    return Ok(false);
                }
                continue;
            }

            self.read_available()?;
        }
    }

    pub(super) fn read(&mut self) -> io::Result<Event> {
        self.ready
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "no terminal event is ready"))
    }

    fn active(&self) -> &[u8] {
        &self.buffer[self.start..]
    }

    fn prepare_event(&mut self) -> io::Result<bool> {
        loop {
            if self.active().is_empty() {
                self.compact();
                self.incomplete_since = None;
                return Ok(false);
            }

            let escape_expired = self
                .incomplete_since
                .is_some_and(|started| started.elapsed() >= ESCAPE_TIMEOUT);
            match parse_one(self.active(), escape_expired)? {
                Parsed::Complete(event, used) => {
                    self.consume(used);
                    self.incomplete_since = None;
                    if event == Event::Ignored {
                        continue;
                    }
                    self.ready = Some(event);
                    return Ok(true);
                }
                Parsed::Incomplete => {
                    let started = *self.incomplete_since.get_or_insert_with(Instant::now);
                    if started.elapsed() >= SEQUENCE_TIMEOUT {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "terminal input sequence did not finish within 2 seconds",
                        ));
                    }
                    return Ok(false);
                }
            }
        }
    }

    fn sequence_wait(&self, now: Instant) -> io::Result<Duration> {
        let Some(started) = self.incomplete_since else {
            return Ok(Duration::MAX);
        };
        let active = self.active();
        let limit = if active == [0x1b] {
            ESCAPE_TIMEOUT
        } else {
            SEQUENCE_TIMEOUT
        };
        let elapsed = now.saturating_duration_since(started);
        if elapsed >= SEQUENCE_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "terminal input sequence did not finish within 2 seconds",
            ));
        }
        Ok(limit.saturating_sub(elapsed))
    }

    fn read_available(&mut self) -> io::Result<()> {
        let active_len = self.active().len();
        if active_len >= MAX_SEQUENCE_BYTES {
            let mut extra = [0_u8; 1];
            return match self.input.read(&mut extra) {
                Ok(0) => Err(input_closed()),
                Ok(_) => Err(sequence_too_large()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(()),
                Err(error) if error.raw_os_error() == Some(libc::EIO) => Err(input_closed()),
                Err(error) => Err(error),
            };
        }

        self.compact();
        let available = MAX_SEQUENCE_BYTES - self.active().len();
        let mut bytes = [0_u8; READ_CHUNK_BYTES];
        let requested = available.min(bytes.len());
        match self.input.read(&mut bytes[..requested]) {
            Ok(0) => Err(input_closed()),
            Ok(count) => {
                self.buffer
                    .try_reserve(count)
                    .map_err(|_| io::Error::other("terminal input buffer allocation failed"))?;
                self.buffer.extend_from_slice(&bytes[..count]);
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(()),
            Err(error) if error.raw_os_error() == Some(libc::EIO) => Err(input_closed()),
            Err(error) => Err(error),
        }
    }

    fn consume(&mut self, count: usize) {
        self.start += count;
        self.compact();
    }

    fn compact(&mut self) {
        if self.start == self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
        } else if self.start >= READ_CHUNK_BYTES && self.start >= self.buffer.len() / 2 {
            self.buffer.drain(..self.start);
            self.start = 0;
        }
    }
}

impl Drop for TerminalInput {
    fn drop(&mut self) {
        if let Some(original) = self.original_mode.take() {
            let _ = set_terminal_mode(&self.input, &original);
        }
    }
}

fn terminal_input() -> io::Result<File> {
    if let Some(stdin) = duplicate_terminal(libc::STDIN_FILENO) {
        return Ok(stdin);
    }
    OpenOptions::new().read(true).write(true).open("/dev/tty")
}

fn screen_terminal() -> io::Result<File> {
    match OpenOptions::new().read(true).write(true).open("/dev/tty") {
        Ok(terminal) => Ok(terminal),
        Err(dev_tty_error) => duplicate_terminal(libc::STDOUT_FILENO).ok_or(dev_tty_error),
    }
}

fn duplicate_terminal(fd: libc::c_int) -> Option<File> {
    // SAFETY: fcntl receives a valid command and does not retain pointers.
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate == -1 {
        return None;
    }
    // SAFETY: duplicate is a fresh owned descriptor returned by fcntl.
    let file = unsafe { File::from_raw_fd(duplicate) };
    // SAFETY: file owns a live descriptor for the duration of the call.
    (unsafe { libc::isatty(file.as_raw_fd()) } == 1).then_some(file)
}

fn terminal_mode(terminal: &File) -> io::Result<libc::termios> {
    // SAFETY: libc::termios contains only C scalar and array fields.
    let mut mode = unsafe { std::mem::zeroed::<libc::termios>() };
    // SAFETY: terminal is open and mode points to writable storage.
    if unsafe { libc::tcgetattr(terminal.as_raw_fd(), &mut mode) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(mode)
    }
}

fn set_terminal_mode(terminal: &File, mode: &libc::termios) -> io::Result<()> {
    // SAFETY: terminal is open and mode points to initialized termios storage.
    if unsafe { libc::tcsetattr(terminal.as_raw_fd(), libc::TCSANOW, mode) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn mode_is_raw(mode: &libc::termios) -> bool {
    let disabled_input = libc::BRKINT | libc::ICRNL | libc::INPCK | libc::ISTRIP | libc::IXON;
    let disabled_local = libc::ECHO | libc::ICANON | libc::IEXTEN;
    mode.c_iflag & disabled_input == 0
        && mode.c_oflag & libc::OPOST == 0
        && mode.c_lflag & disabled_local == 0
        && mode.c_cflag & libc::CSIZE == libc::CS8
        && mode.c_cc[libc::VMIN] == 1
        && mode.c_cc[libc::VTIME] == 0
}

pub(super) fn terminal_size(terminal: &File) -> io::Result<(u16, u16)> {
    // SAFETY: libc::winsize contains only C scalar fields.
    let mut size = unsafe { std::mem::zeroed::<libc::winsize>() };
    // SAFETY: terminal is open and size points to writable winsize storage.
    if unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok((size.ws_col, size.ws_row))
    }
}

#[cfg(not(target_os = "macos"))]
fn poll_readable(input: &File, timeout: Duration) -> io::Result<bool> {
    let mut descriptor = libc::pollfd {
        fd: input.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: descriptor points to one initialized pollfd for the duration of the call.
    let result = unsafe { libc::poll(&mut descriptor, 1, poll_milliseconds(timeout)) };
    if result == -1 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(error)
        };
    }
    if result == 0 {
        return Ok(false);
    }
    if descriptor.revents & libc::POLLNVAL != 0 {
        return Err(input_closed());
    }
    Ok(descriptor.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
}

#[cfg(target_os = "macos")]
fn poll_readable(input: &File, timeout: Duration) -> io::Result<bool> {
    let fd = input.as_raw_fd();
    if fd < 0
        || usize::try_from(fd)
            .ok()
            .is_none_or(|fd| fd >= libc::FD_SETSIZE)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal input descriptor exceeds the select limit",
        ));
    }

    // Darwin poll(2) reports POLLNVAL for valid PTYs, so use select(2) on this platform.
    // SAFETY: fd_set is initialized by FD_ZERO before it is passed to FD_SET or select.
    let mut readable = unsafe { std::mem::zeroed::<libc::fd_set>() };
    // SAFETY: readable points to initialized fd_set storage and fd passed the FD_SETSIZE bound.
    unsafe {
        libc::FD_ZERO(&mut readable);
        libc::FD_SET(fd, &mut readable);
    }
    let mut limit = (timeout != Duration::MAX).then(|| libc::timeval {
        tv_sec: libc::time_t::try_from(timeout.as_secs()).unwrap_or(libc::time_t::MAX),
        tv_usec: libc::suseconds_t::try_from(timeout.subsec_micros())
            .expect("subsecond microseconds fit suseconds_t"),
    });
    // SAFETY: readable and the optional timeout remain valid and exclusive for the call.
    let result = unsafe {
        libc::select(
            fd + 1,
            &mut readable,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            limit
                .as_mut()
                .map_or(std::ptr::null_mut(), std::ptr::from_mut),
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(error)
        };
    }
    if result == 0 {
        return Ok(false);
    }
    // SAFETY: readable was populated by select and fd remains within FD_SETSIZE.
    Ok(unsafe { libc::FD_ISSET(fd, &readable) })
}

#[cfg(not(target_os = "macos"))]
fn poll_milliseconds(timeout: Duration) -> libc::c_int {
    if timeout == Duration::MAX {
        return -1;
    }
    let milliseconds =
        timeout.as_millis() + u128::from(!timeout.subsec_nanos().is_multiple_of(1_000_000));
    libc::c_int::try_from(milliseconds).unwrap_or(libc::c_int::MAX)
}

fn remaining_until(deadline: Option<Instant>, now: Instant) -> Option<Duration> {
    match deadline {
        Some(deadline) if now >= deadline => None,
        Some(deadline) => Some(deadline.duration_since(now)),
        None => Some(Duration::MAX),
    }
}

fn input_closed() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "terminal input closed")
}

fn sequence_too_large() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "terminal input sequence exceeded 65536 bytes",
    )
}

enum Parsed {
    Complete(Event, usize),
    Incomplete,
}

fn parse_one(bytes: &[u8], escape_expired: bool) -> io::Result<Parsed> {
    let Some(&first) = bytes.first() else {
        return Ok(Parsed::Incomplete);
    };
    if first == 0x1b {
        return parse_escape(bytes, escape_expired);
    }
    parse_plain_key(bytes)
}

fn parse_plain_key(bytes: &[u8]) -> io::Result<Parsed> {
    let byte = bytes[0];
    let event = match byte {
        b'\r' | b'\n' => key_event(KeyCode::Enter, KeyModifiers::NONE),
        0x7f => key_event(KeyCode::Backspace, KeyModifiers::NONE),
        0x01..=0x1a => {
            let character = char::from(b'a' + byte - 1);
            key_event(KeyCode::Char(character), KeyModifiers::CONTROL)
        }
        0x00 | 0x1c..=0x1f => Event::Ignored,
        0x20..=0x7e => {
            let character = char::from(byte);
            let modifiers = if character.is_ascii_uppercase()
                || matches!(
                    character,
                    '!' | '@'
                        | '#'
                        | '$'
                        | '%'
                        | '^'
                        | '&'
                        | '*'
                        | '('
                        | ')'
                        | '_'
                        | '+'
                        | '{'
                        | '}'
                        | '|'
                        | ':'
                        | '"'
                        | '<'
                        | '>'
                        | '?'
                ) {
                KeyModifiers::SHIFT
            } else {
                KeyModifiers::NONE
            };
            key_event(KeyCode::Char(character), modifiers)
        }
        _ => {
            let width = utf8_width(byte).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid UTF-8 in terminal input",
                )
            })?;
            if bytes.len() < width {
                return Ok(Parsed::Incomplete);
            }
            let text = std::str::from_utf8(&bytes[..width]).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid UTF-8 in terminal input",
                )
            })?;
            let character = text
                .chars()
                .next()
                .expect("a complete UTF-8 scalar contains one character");
            return Ok(Parsed::Complete(
                if character.is_control() {
                    Event::Ignored
                } else {
                    key_event(KeyCode::Char(character), KeyModifiers::NONE)
                },
                width,
            ));
        }
    };
    Ok(Parsed::Complete(event, 1))
}

fn parse_escape(bytes: &[u8], escape_expired: bool) -> io::Result<Parsed> {
    if bytes.len() == 1 {
        return if escape_expired {
            Ok(Parsed::Complete(
                key_event(KeyCode::Esc, KeyModifiers::NONE),
                1,
            ))
        } else {
            Ok(Parsed::Incomplete)
        };
    }

    if bytes[1] == 0x1b {
        return Ok(Parsed::Complete(
            key_event(KeyCode::Esc, KeyModifiers::NONE),
            1,
        ));
    }

    match bytes[1] {
        b'[' => parse_csi(bytes),
        b'O' => parse_ss3(bytes),
        b']' => parse_control_string(bytes, true),
        b'P' | b'_' | b'^' => parse_control_string(bytes, false),
        _ => match parse_plain_key(&bytes[1..])? {
            Parsed::Complete(Event::Key(mut key), used) => {
                key.modifiers = key.modifiers.union(KeyModifiers::ALT);
                Ok(Parsed::Complete(Event::Key(key), used + 1))
            }
            Parsed::Complete(_, used) => Ok(Parsed::Complete(Event::Ignored, used + 1)),
            Parsed::Incomplete => Ok(Parsed::Incomplete),
        },
    }
}

fn parse_ss3(bytes: &[u8]) -> io::Result<Parsed> {
    if bytes.len() < 3 {
        return Ok(Parsed::Incomplete);
    }
    let code = match bytes[2] {
        b'A' => Some(KeyCode::Up),
        b'B' => Some(KeyCode::Down),
        b'H' => Some(KeyCode::Home),
        b'F' => Some(KeyCode::End),
        b'P' => Some(KeyCode::F(1)),
        _ => None,
    };
    Ok(Parsed::Complete(
        code.map_or(Event::Ignored, |code| key_event(code, KeyModifiers::NONE)),
        3,
    ))
}

fn parse_control_string(bytes: &[u8], bell_terminated: bool) -> io::Result<Parsed> {
    let tail = &bytes[2..];
    let bell = bell_terminated.then(|| tail.iter().position(|byte| *byte == 0x07));
    let string_terminator = tail.windows(2).position(|window| window == b"\x1b\\");
    let used = match (bell.flatten(), string_terminator) {
        (Some(bell), Some(st)) => (bell + 1).min(st + 2),
        (Some(bell), None) => bell + 1,
        (None, Some(st)) => st + 2,
        (None, None) => return Ok(Parsed::Incomplete),
    };
    Ok(Parsed::Complete(Event::Ignored, 2 + used))
}

fn parse_csi(bytes: &[u8]) -> io::Result<Parsed> {
    if bytes.len() < 3 {
        return Ok(Parsed::Incomplete);
    }
    if bytes.starts_with(b"\x1b[M") {
        return if bytes.len() < 6 {
            Ok(Parsed::Incomplete)
        } else {
            Ok(Parsed::Complete(Event::Ignored, 6))
        };
    }
    // The Linux virtual console uses ESC [[ A through ESC [[ E for F1 through F5. Handle that
    // legacy form before the general CSI final-byte scan treats the second '[' as the terminator.
    if bytes.starts_with(b"\x1b[[") {
        if bytes.len() < 4 {
            return Ok(Parsed::Incomplete);
        }
        let event = if bytes[3] == b'A' {
            key_event(KeyCode::F(1), KeyModifiers::NONE)
        } else {
            Event::Ignored
        };
        return Ok(Parsed::Complete(event, 4));
    }

    let Some(final_index) = bytes[2..]
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
        .map(|index| index + 2)
    else {
        return Ok(Parsed::Incomplete);
    };
    let sequence = &bytes[..=final_index];
    let final_byte = bytes[final_index];
    let body = &sequence[2..final_index];

    if final_byte == b'~' && body == b"200" {
        let Some(end) = bytes[final_index + 1..]
            .windows(6)
            .position(|window| window == b"\x1b[201~")
        else {
            return Ok(Parsed::Incomplete);
        };
        return Ok(Parsed::Complete(Event::Ignored, final_index + 1 + end + 6));
    }

    let event = match final_byte {
        b'A' => modified_csi_key(KeyCode::Up, body),
        b'B' => modified_csi_key(KeyCode::Down, body),
        b'H' => modified_csi_key(KeyCode::Home, body),
        b'F' => modified_csi_key(KeyCode::End, body),
        b'P' => modified_csi_key(KeyCode::F(1), body),
        b'~' => parse_tilde_key(body),
        b'u' => parse_csi_u(body),
        _ => None,
    }
    .unwrap_or(Event::Ignored);
    Ok(Parsed::Complete(event, final_index + 1))
}

fn modified_csi_key(code: KeyCode, body: &[u8]) -> Option<Event> {
    let text = std::str::from_utf8(body).ok()?;
    let (modifiers, kind, state) = match text.rsplit_once(';') {
        Some((_, field)) => parse_modifier_field(field)?,
        None => (KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE),
    };
    Some(Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        state,
    }))
}

fn parse_tilde_key(body: &[u8]) -> Option<Event> {
    let text = std::str::from_utf8(body).ok()?;
    let mut fields = text.split(';');
    let code = match fields.next()?.parse::<u8>().ok()? {
        1 | 7 => KeyCode::Home,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        11 => KeyCode::F(1),
        _ => return None,
    };
    let (modifiers, kind, state) = match fields.next() {
        Some(field) => parse_modifier_field(field)?,
        None => (KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE),
    };
    Some(Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        state,
    }))
}

fn parse_csi_u(body: &[u8]) -> Option<Event> {
    let text = std::str::from_utf8(body).ok()?;
    let mut fields = text.split(';');
    let mut codepoints = fields.next()?.split(':');
    let primary = codepoints.next()?.parse::<u32>().ok()?;
    let alternate = codepoints
        .next()
        .and_then(|codepoint| codepoint.parse::<u32>().ok())
        .and_then(char::from_u32);
    let (mut modifiers, kind, state) = match fields.next() {
        Some(field) => parse_modifier_field(field)?,
        None => (KeyModifiers::NONE, KeyEventKind::Press, KeyEventState::NONE),
    };

    let code = match primary {
        57344 => KeyCode::Esc,
        57345 => KeyCode::Enter,
        57347 => KeyCode::Backspace,
        57352 => KeyCode::Up,
        57353 => KeyCode::Down,
        57354 => KeyCode::PageUp,
        57355 => KeyCode::PageDown,
        57356 => KeyCode::Home,
        57357 => KeyCode::End,
        57358 => KeyCode::CapsLock,
        57359 => KeyCode::ScrollLock,
        57360 => KeyCode::NumLock,
        57364 => KeyCode::F(1),
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        127 => KeyCode::Backspace,
        57441..=57454 => {
            let modifier = match primary {
                57441 | 57447 => KeyModifiers::SHIFT,
                57442 | 57448 => KeyModifiers::CONTROL,
                57443 | 57449 => KeyModifiers::ALT,
                57444 | 57450 => KeyModifiers::SUPER,
                57445 | 57451 => KeyModifiers::HYPER,
                57446 | 57452 => KeyModifiers::META,
                _ => KeyModifiers::NONE,
            };
            modifiers = modifiers.union(modifier);
            KeyCode::Modifier
        }
        0..=31 | 127..=159 | 57344..=57454 => return Some(Event::Ignored),
        _ => {
            let mut character = char::from_u32(primary)?;
            if character.is_control() {
                return Some(Event::Ignored);
            }
            if modifiers.contains(KeyModifiers::SHIFT)
                && let Some(shifted) = alternate
            {
                character = shifted;
                modifiers.set(KeyModifiers::SHIFT, false);
            }
            KeyCode::Char(character)
        }
    };
    Some(Event::Key(KeyEvent {
        code,
        modifiers,
        kind,
        state,
    }))
}

fn parse_modifier_field(field: &str) -> Option<(KeyModifiers, KeyEventKind, KeyEventState)> {
    let mut parts = field.split(':');
    let encoded = parts.next()?.parse::<u16>().ok()?;
    let mask = encoded.checked_sub(1)?;
    let kind = match parts.next() {
        None | Some("1") => KeyEventKind::Press,
        Some("2") => KeyEventKind::Repeat,
        Some("3") => KeyEventKind::Release,
        Some(_) => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    let mut bits = 0;
    for (kitty, modifier) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if mask & kitty != 0 {
            bits |= modifier.bits();
        }
    }
    let state = if mask & 64 != 0 {
        KeyEventState::CAPS_LOCK
    } else {
        KeyEventState::NONE
    };
    Some((KeyModifiers::from_bits_truncate(bits), kind, state))
}

fn key_event(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    })
}

const fn utf8_width(first: u8) -> Option<usize> {
    match first {
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use rustix::{
        fs::{self, Mode, OFlags},
        pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
        termios::{self, Winsize},
    };

    use super::*;

    fn test_terminal_input() -> (File, TerminalInput) {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let name = ptsname(&master, Vec::new()).unwrap();
        let slave = fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )
        .unwrap();
        termios::tcsetwinsize(
            &slave,
            Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )
        .unwrap();
        let input = File::from(slave);
        let screen = input.try_clone().unwrap();
        (
            File::from(master),
            TerminalInput {
                input,
                screen,
                original_mode: None,
                buffer: Vec::with_capacity(READ_CHUNK_BYTES),
                start: 0,
                ready: None,
                incomplete_since: None,
                last_size: (80, 24),
            },
        )
    }

    fn parsed(bytes: &[u8]) -> Event {
        match parse_one(bytes, true).unwrap() {
            Parsed::Complete(event, used) => {
                assert_eq!(used, bytes.len());
                event
            }
            Parsed::Incomplete => panic!("expected a complete event"),
        }
    }

    #[test]
    fn parses_plain_control_and_navigation_keys() {
        assert_eq!(
            parsed(b"G"),
            key_event(KeyCode::Char('G'), KeyModifiers::SHIFT)
        );
        assert_eq!(
            parsed(b"\x15"),
            key_event(KeyCode::Char('u'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            parsed(b"\x1b[A"),
            key_event(KeyCode::Up, KeyModifiers::NONE)
        );
        assert!(matches!(
            parse_one(b"\x1b[[", false),
            Ok(Parsed::Incomplete)
        ));
        assert_eq!(
            parsed(b"\x1b[[A"),
            key_event(KeyCode::F(1), KeyModifiers::NONE)
        );
        assert_eq!(parsed(b"\x1b[[B"), Event::Ignored);
        for control in [0x00, 0x1c, 0x1d, 0x1e, 0x1f] {
            assert_eq!(parsed(&[control]), Event::Ignored);
        }
    }

    #[test]
    fn escape_and_utf8_boundaries_are_incremental_and_nonfatal() {
        assert!(matches!(parse_one(b"\x1b", false), Ok(Parsed::Incomplete)));
        assert_eq!(parsed(b"\x1b"), key_event(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(parse_one(&[0xc3], false), Ok(Parsed::Incomplete)));
        assert_eq!(
            parsed("é".as_bytes()),
            key_event(KeyCode::Char('é'), KeyModifiers::NONE)
        );
        assert_eq!(
            parsed("\u{1b}é".as_bytes()),
            key_event(KeyCode::Char('é'), KeyModifiers::ALT)
        );
        assert!(parse_one(&[0xc3, b'x'], false).is_err());
        assert!(matches!(
            parse_one(b"\x1b\x1b", false),
            Ok(Parsed::Complete(
                Event::Key(KeyEvent {
                    code: KeyCode::Esc,
                    ..
                }),
                1
            ))
        ));
    }

    #[test]
    fn parses_the_supported_kitty_keyboard_fields() {
        assert_eq!(
            parsed(b"\x1b[103:71;2:2u"),
            Event::Key(KeyEvent {
                code: KeyCode::Char('G'),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Repeat,
                state: KeyEventState::NONE,
            })
        );
        assert_eq!(
            parsed(b"\x1b[103;66u"),
            Event::Key(KeyEvent {
                code: KeyCode::Char('g'),
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: KeyEventState::CAPS_LOCK,
            })
        );
        assert_eq!(
            parsed(b"\x1b[57352;1:2u"),
            Event::Key(KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Repeat,
                state: KeyEventState::NONE,
            })
        );
        assert_eq!(parsed(b"\x1b[57361u"), Event::Ignored);
        assert_eq!(parsed(b"\x1b[9u"), Event::Ignored);
        assert_eq!(parsed(b"\x1b[113;:u"), Event::Ignored);
    }

    #[test]
    fn legacy_csi_navigation_preserves_event_metadata() {
        assert_eq!(
            parsed(b"\x1b[1;1:3A"),
            Event::Key(KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Release,
                state: KeyEventState::NONE,
            })
        );
        assert_eq!(
            parsed(b"\x1b[1;66:2P"),
            Event::Key(KeyEvent {
                code: KeyCode::F(1),
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Repeat,
                state: KeyEventState::CAPS_LOCK,
            })
        );
        assert_eq!(parsed(b"\x1b[1;1:4A"), Event::Ignored);
    }

    #[test]
    fn zero_timeout_checks_ready_input_and_suspend_reset_discards_it() {
        let (mut master, mut terminal) = test_terminal_input();
        terminal.enable_raw_mode().unwrap();

        master.write_all(b"q").unwrap();
        assert!(terminal.poll(Duration::ZERO).unwrap());
        terminal.discard_buffered_events();
        assert_eq!(
            terminal.read().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        master.write_all(b"q").unwrap();
        assert!(terminal.poll(Duration::ZERO).unwrap());
        assert_eq!(
            terminal.read().unwrap(),
            key_event(KeyCode::Char('q'), KeyModifiers::NONE)
        );
        terminal.disable_raw_mode().unwrap();
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn poll_timeout_rounds_partial_milliseconds_up() {
        assert_eq!(poll_milliseconds(Duration::ZERO), 0);
        assert_eq!(poll_milliseconds(Duration::from_nanos(1)), 1);
        assert_eq!(poll_milliseconds(Duration::from_millis(1)), 1);
        assert_eq!(poll_milliseconds(Duration::from_micros(1_001)), 2);
        assert_eq!(poll_milliseconds(Duration::MAX), -1);
    }

    #[test]
    fn consumes_mouse_reports_and_complete_private_replies() {
        assert_eq!(parsed(b"\x1b[M\x20\x20\x20"), Event::Ignored);
        assert_eq!(parsed(b"\x1b[<0;0;0M"), Event::Ignored);
        assert_eq!(parsed(b"\x1b[?997;1n"), Event::Ignored);
    }

    #[test]
    fn bracketed_paste_is_one_bounded_ignored_sequence() {
        assert!(matches!(
            parse_one(b"\x1b[200~draft", false),
            Ok(Parsed::Incomplete)
        ));
        assert_eq!(parsed(b"\x1b[200~draft\x1b[201~"), Event::Ignored);
    }
}
