use std::{
    fs::File,
    io::{self, BufWriter, Stdout, Write},
    mem,
    os::fd::AsRawFd,
};

use ratatui::{
    backend::{Backend, ClearType, WindowSize},
    buffer::{Cell, CellWidth},
    layout::{Position, Size},
    style::{Color, Modifier},
};

const CURSOR_HIDE: &[u8] = b"\x1b[?25l";
const CURSOR_SHOW: &[u8] = b"\x1b[?25h";
const STYLE_RESET: &[u8] = b"\x1b[0m";

/// A small ANSI backend for the fixed, Unix-only terminal surface used by TUT.
#[derive(Debug)]
pub(super) struct AnsiBackend<W: Write = BufWriter<Stdout>> {
    writer: W,
    tty: File,
    cursor: Position,
}

impl AnsiBackend<BufWriter<Stdout>> {
    /// Creates a buffered stdout backend whose dimensions come from `tty`.
    pub(super) fn new(tty: &File) -> io::Result<Self> {
        Ok(Self {
            writer: BufWriter::new(io::stdout()),
            tty: tty.try_clone()?,
            cursor: Position::ORIGIN,
        })
    }
}

impl<W: Write> AnsiBackend<W> {
    /// Writes a session-level escape sequence immediately.
    pub(super) fn write_session(&mut self, sequence: &[u8]) -> io::Result<()> {
        self.writer.write_all(sequence)?;
        self.writer.flush()
    }

    fn move_cursor(&mut self, position: Position) -> io::Result<()> {
        write!(
            self.writer,
            "\x1b[{};{}H",
            position.y.saturating_add(1),
            position.x.saturating_add(1)
        )?;
        self.cursor = position;
        Ok(())
    }
}

impl<W: Write> Backend for AnsiBackend<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut current_style = None;
        let mut next_position = None;

        for (x, y, cell) in content {
            let position = Position::new(x, y);
            if next_position != Some(position) {
                self.move_cursor(position)?;
            }

            let style = CellStyle::from(cell);
            if current_style != Some(style) {
                write_style(&mut self.writer, style)?;
                current_style = Some(style);
            }

            self.writer.write_all(cell.symbol().as_bytes())?;
            let position_after_cell = Position::new(x.saturating_add(cell.cell_width()), y);
            self.cursor = position_after_cell;
            next_position = Some(position_after_cell);
        }

        if current_style.is_some() {
            self.writer.write_all(STYLE_RESET)?;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.write_session(CURSOR_HIDE)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.write_session(CURSOR_SHOW)
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.cursor)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.move_cursor(position.into())?;
        self.writer.flush()
    }

    fn clear(&mut self) -> io::Result<()> {
        self.clear_region(ClearType::All)
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        let sequence = match clear_type {
            ClearType::All => b"\x1b[2J".as_slice(),
            ClearType::AfterCursor => b"\x1b[0J".as_slice(),
            ClearType::BeforeCursor => b"\x1b[1J".as_slice(),
            ClearType::CurrentLine => b"\x1b[2K".as_slice(),
            ClearType::UntilNewLine => b"\x1b[0K".as_slice(),
        };
        self.write_session(sequence)
    }

    fn size(&self) -> io::Result<Size> {
        Ok(query_window_size(&self.tty)?.columns_rows)
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        query_window_size(&self.tty)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellStyle {
    foreground: Color,
    background: Color,
    modifiers: Modifier,
}

impl From<&Cell> for CellStyle {
    fn from(cell: &Cell) -> Self {
        Self {
            foreground: cell.fg,
            background: cell.bg,
            modifiers: cell.modifier,
        }
    }
}

fn write_style<W: Write>(writer: &mut W, style: CellStyle) -> io::Result<()> {
    writer.write_all(b"\x1b[0")?;

    for (modifier, code) in [
        (Modifier::BOLD, 1),
        (Modifier::DIM, 2),
        (Modifier::ITALIC, 3),
        (Modifier::UNDERLINED, 4),
        (Modifier::SLOW_BLINK, 5),
        (Modifier::RAPID_BLINK, 6),
        (Modifier::REVERSED, 7),
        (Modifier::HIDDEN, 8),
        (Modifier::CROSSED_OUT, 9),
    ] {
        if style.modifiers.contains(modifier) {
            write!(writer, ";{code}")?;
        }
    }

    write_color(writer, style.foreground, ColorLayer::Foreground)?;
    write_color(writer, style.background, ColorLayer::Background)?;
    writer.write_all(b"m")
}

#[derive(Debug, Clone, Copy)]
enum ColorLayer {
    Foreground,
    Background,
}

fn write_color<W: Write>(writer: &mut W, color: Color, layer: ColorLayer) -> io::Result<()> {
    let (normal_offset, bright_offset, reset, indexed, rgb) = match layer {
        ColorLayer::Foreground => (30, 90, 39, 38, 38),
        ColorLayer::Background => (40, 100, 49, 48, 48),
    };

    match color {
        Color::Reset => write!(writer, ";{reset}"),
        Color::Black => write!(writer, ";{normal_offset}"),
        Color::Red => write!(writer, ";{}", normal_offset + 1),
        Color::Green => write!(writer, ";{}", normal_offset + 2),
        Color::Yellow => write!(writer, ";{}", normal_offset + 3),
        Color::Blue => write!(writer, ";{}", normal_offset + 4),
        Color::Magenta => write!(writer, ";{}", normal_offset + 5),
        Color::Cyan => write!(writer, ";{}", normal_offset + 6),
        Color::Gray => write!(writer, ";{}", normal_offset + 7),
        Color::DarkGray => write!(writer, ";{bright_offset}"),
        Color::LightRed => write!(writer, ";{}", bright_offset + 1),
        Color::LightGreen => write!(writer, ";{}", bright_offset + 2),
        Color::LightYellow => write!(writer, ";{}", bright_offset + 3),
        Color::LightBlue => write!(writer, ";{}", bright_offset + 4),
        Color::LightMagenta => write!(writer, ";{}", bright_offset + 5),
        Color::LightCyan => write!(writer, ";{}", bright_offset + 6),
        Color::White => write!(writer, ";{}", bright_offset + 7),
        Color::Indexed(value) => write!(writer, ";{indexed};5;{value}"),
        Color::Rgb(red, green, blue) => write!(writer, ";{rgb};2;{red};{green};{blue}"),
    }
}

fn query_window_size(tty: &File) -> io::Result<WindowSize> {
    // SAFETY: `libc::winsize` contains only integer fields, so all-zero is valid.
    let mut size = unsafe { mem::zeroed::<libc::winsize>() };
    // SAFETY: `tty` owns a valid descriptor and `size` is writable for the duration of the call.
    if unsafe { libc::ioctl(tty.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } == -1 {
        return Err(io::Error::last_os_error());
    }

    Ok(WindowSize {
        columns_rows: Size::new(size.ws_col, size.ws_row),
        pixels: Size::new(size.ws_xpixel, size.ws_ypixel),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> AnsiBackend<Vec<u8>> {
        AnsiBackend {
            writer: Vec::new(),
            tty: File::open("/dev/null").expect("open /dev/null"),
            cursor: Position::ORIGIN,
        }
    }

    #[test]
    fn style_sequence_covers_all_modifiers_and_named_colors() {
        let mut output = Vec::new();
        let style = CellStyle {
            foreground: Color::LightCyan,
            background: Color::Blue,
            modifiers: Modifier::all(),
        };

        write_style(&mut output, style).expect("encode style");

        assert_eq!(output, b"\x1b[0;1;2;3;4;5;6;7;8;9;96;44m");
    }

    #[test]
    fn style_sequence_supports_indexed_and_true_colors() {
        let mut indexed = Vec::new();
        write_style(
            &mut indexed,
            CellStyle {
                foreground: Color::Indexed(123),
                background: Color::Reset,
                modifiers: Modifier::empty(),
            },
        )
        .expect("encode indexed style");
        assert_eq!(indexed, b"\x1b[0;38;5;123;49m");

        let mut rgb = Vec::new();
        write_style(
            &mut rgb,
            CellStyle {
                foreground: Color::Reset,
                background: Color::Rgb(1, 2, 3),
                modifiers: Modifier::empty(),
            },
        )
        .expect("encode RGB style");
        assert_eq!(rgb, b"\x1b[0;39;48;2;1;2;3m");
    }

    #[test]
    fn draw_repositions_disjoint_cells_and_tracks_the_cursor() {
        let mut backend = backend();
        let mut first = Cell::new("A");
        first.set_fg(Color::Red);
        let second = Cell::new("界");
        let third = Cell::new("Z");

        backend
            .draw([(2, 3, &first), (3, 3, &second), (8, 4, &third)].into_iter())
            .expect("draw cells");

        assert_eq!(backend.get_cursor_position().unwrap(), Position::new(9, 4));
        assert_eq!(
            backend.writer,
            b"\x1b[4;3H\x1b[0;31;49mA\x1b[0;39;49m\
              \xE7\x95\x8C\x1b[5;9HZ\x1b[0m"
        );
    }

    #[test]
    fn cursor_clear_and_session_sequences_flush_to_the_writer() {
        let mut backend = backend();

        backend.hide_cursor().unwrap();
        backend.show_cursor().unwrap();
        backend.set_cursor_position(Position::new(7, 11)).unwrap();
        backend.clear_region(ClearType::BeforeCursor).unwrap();
        backend.clear_region(ClearType::CurrentLine).unwrap();
        backend.clear_region(ClearType::UntilNewLine).unwrap();
        backend.clear_region(ClearType::AfterCursor).unwrap();
        backend.clear().unwrap();
        backend.write_session(b"session").unwrap();

        assert_eq!(backend.get_cursor_position().unwrap(), Position::new(7, 11));
        assert_eq!(
            backend.writer,
            b"\x1b[?25l\x1b[?25h\x1b[12;8H\x1b[1J\x1b[2K\x1b[0K\x1b[0J\
              \x1b[2Jsession"
        );
    }

    #[test]
    fn failed_terminal_size_query_preserves_the_os_error() {
        let mut backend = backend();

        assert!(backend.size().is_err());
        assert!(backend.window_size().is_err());
    }
}
