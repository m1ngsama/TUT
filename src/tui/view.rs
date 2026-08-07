use std::{fmt::Write as _, num::NonZeroU16};

use ratatui::{
    Frame,
    buffer::{Buffer, CellDiffOption},
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    app::{
        ContentWidth, DisplayAtoms, DisplayColumn, Highlight, MIN_TERMINAL_COLUMNS,
        MIN_TERMINAL_ROWS, RenderProjectionKind, RenderRow, RenderSpan, RenderState, SearchStatus,
    },
    error::{TutError, sanitize_text},
};

const TINY_MESSAGE: &str = "terminal too small — resize";
const KEY_HELP: &str = "j/k ↑/↓ move  Space/b page  / search  n/N match  g/G ends  q quit";

pub(super) fn render(frame: &mut Frame<'_>, state: &RenderState<'_>) -> Result<(), TutError> {
    let area = frame.area();
    if area.width < MIN_TERMINAL_COLUMNS || area.height < MIN_TERMINAL_ROWS {
        render_projected_line(
            frame,
            Rect::new(area.x, area.y, area.width, area.height.min(1)),
            TINY_MESSAGE,
        )?;
        return Ok(());
    }

    let header_text = header_text(state, area.width)?;
    let status_text = status_text(state, area.width)?;
    let help_text = ellipsize_end(KEY_HELP, area.width)?;
    let body_height = area.height - 3;
    let header = Rect::new(area.x, area.y, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, body_height);
    let status = Rect::new(area.x, area.y + 1 + body_height, area.width, 1);
    let help = Rect::new(area.x, area.y + 2 + body_height, area.width, 1);

    render_projected_line(frame, header, &header_text)?;
    render_body(frame, body, &state.rows);
    render_projected_line(frame, status, &status_text)?;
    render_projected_line(frame, help, &help_text)?;
    Ok(())
}

fn render_body(frame: &mut Frame<'_>, area: Rect, rows: &[RenderRow]) {
    frame.render_widget(ReaderBody { rows }, area);
}

struct ReaderBody<'a> {
    rows: &'a [RenderRow],
}

impl Widget for ReaderBody<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for (relative_y, row) in self.rows.iter().take(usize::from(area.height)).enumerate() {
            let y =
                area.y + u16::try_from(relative_y).expect("visible row count fits terminal height");
            let mut x = area.x;
            for span in &row.spans {
                let width = u16::try_from(span.cell_width.get())
                    .expect("projected width fits the terminal width");
                if width > area.right().saturating_sub(x) {
                    break;
                }
                x += write_render_span(buffer, x, y, &row.text, span);
            }
        }
    }
}

fn write_render_span(buffer: &mut Buffer, x: u16, y: u16, row: &str, span: &RenderSpan) -> u16 {
    let width =
        u16::try_from(span.cell_width.get()).expect("projected width fits the terminal width");
    let style = style_for(span.highlight);
    let one = NonZeroU16::new(1).expect("one is nonzero");

    if span.projection == RenderProjectionKind::Spaces {
        for offset in 0..width {
            let cell = buffer
                .cell_mut((x + offset, y))
                .expect("span was clipped to the buffer area");
            cell.reset();
            cell.set_symbol(" ")
                .set_style(style)
                .set_diff_option(CellDiffOption::ForcedWidth(one));
        }
        return width;
    }

    let forced_width = NonZeroU16::new(width).expect("projected atoms have nonzero width");
    let cell = buffer
        .cell_mut((x, y))
        .expect("span was clipped to the buffer area");
    cell.reset();
    cell.set_symbol(span.text(row))
        .set_style(style)
        .set_diff_option(CellDiffOption::ForcedWidth(forced_width));
    for offset in 1..width {
        buffer
            .cell_mut((x + offset, y))
            .expect("span was clipped to the buffer area")
            .reset();
    }
    width
}

fn style_for(highlight: Highlight) -> Style {
    match highlight {
        Highlight::None => Style::default(),
        Highlight::Match => Style::default().add_modifier(Modifier::REVERSED),
        Highlight::Current => Style::default()
            .add_modifier(Modifier::REVERSED | Modifier::BOLD | Modifier::UNDERLINED),
    }
}

fn render_projected_line(frame: &mut Frame<'_>, area: Rect, text: &str) -> Result<(), TutError> {
    let Some(content_width) = ContentWidth::new(area.width) else {
        return Ok(());
    };
    if area.height == 0 {
        return Ok(());
    }

    let mut column = DisplayColumn::ZERO;
    let mut x = area.x;
    let mut row = String::new();
    for atom in DisplayAtoms::new(text) {
        let Some(projected) = atom.project(column, content_width) else {
            continue;
        };
        let span = RenderSpan::from_projected(projected, Highlight::None, &mut row)?;
        let width =
            u16::try_from(span.cell_width.get()).expect("projected width fits the terminal width");
        if width > area.right().saturating_sub(x) {
            break;
        }
        x += write_render_span(frame.buffer_mut(), x, area.y, &row, &span);
        column = DisplayColumn::new(column.get() + u32::from(width));
    }
    Ok(())
}

fn header_text(state: &RenderState<'_>, width: u16) -> Result<String, TutError> {
    let filename = sanitize_text(state.filename);
    let path = sanitize_text(state.path);
    let shown_name = ellipsize_end(&filename, (width / 3).max(1))?;
    let used = display_width(&shown_name)?;
    if used >= width {
        return Ok(shown_name);
    }

    let separator = if width - used >= 2 { "  " } else { " " };
    let remaining = width.saturating_sub(used + display_width(separator)?);
    let shown_path = ellipsize_start(&path, remaining)?;
    let mut output = String::new();
    output
        .try_reserve_exact(shown_name.len() + separator.len() + shown_path.len())
        .map_err(|_| TutError::Allocation("header text"))?;
    output.push_str(&shown_name);
    output.push_str(separator);
    output.push_str(&shown_path);
    Ok(output)
}

fn status_text(state: &RenderState<'_>, width: u16) -> Result<String, TutError> {
    let (query, suffix) = match state.status {
        SearchStatus::None => (None, ""),
        SearchStatus::Committed {
            query,
            no_matches: false,
        } => (Some(sanitize_text(query)), ""),
        SearchStatus::Committed {
            query,
            no_matches: true,
        } => (Some(sanitize_text(query)), " — no matches"),
        SearchStatus::Draft {
            draft,
            limit_hit: false,
        } => (Some(sanitize_text(draft)), ""),
        SearchStatus::Draft {
            draft,
            limit_hit: true,
        } => (Some(sanitize_text(draft)), " — query limit: 4096 bytes"),
    };

    let mut prefix = String::new();
    prefix
        .try_reserve_exact(50)
        .map_err(|_| TutError::Allocation("status text"))?;
    write!(
        prefix,
        "{}%  {}/{}",
        state.progress.min(100),
        state.current_line,
        state.total_lines
    )
    .expect("reserved String formatting is infallible");
    if query.is_some() {
        prefix.push_str("  /");
    }
    let fixed_width = display_width(&prefix)?.saturating_add(display_width(suffix)?);
    let query_budget = width.saturating_sub(fixed_width);
    let shown_query = match query.as_deref() {
        Some(query) => ellipsize_end(query, query_budget)?,
        None => String::new(),
    };

    let mut output = String::new();
    output
        .try_reserve_exact(prefix.len() + shown_query.len() + suffix.len())
        .map_err(|_| TutError::Allocation("status text"))?;
    output.push_str(&prefix);
    output.push_str(&shown_query);
    output.push_str(suffix);
    Ok(output)
}

fn ellipsize_end(text: &str, maximum: u16) -> Result<String, TutError> {
    if display_width(text)? <= maximum {
        return fallible_copy(text, "ellipsized text");
    }
    if maximum == 0 {
        return Ok(String::new());
    }
    if maximum == 1 {
        return fallible_copy("…", "ellipsized text");
    }

    let mut end = 0;
    let maximum = u32::from(maximum);
    let mut used = 0_u32;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let width = u32::from(display_width(grapheme)?);
        if used + width + 1 > maximum {
            break;
        }
        end = offset + grapheme.len();
        used += width;
    }
    let mut output = String::new();
    output
        .try_reserve_exact(end + "…".len())
        .map_err(|_| TutError::Allocation("ellipsized text"))?;
    output.push_str(&text[..end]);
    output.push('…');
    Ok(output)
}

fn ellipsize_start(text: &str, maximum: u16) -> Result<String, TutError> {
    if display_width(text)? <= maximum {
        return fallible_copy(text, "ellipsized text");
    }
    if maximum == 0 {
        return Ok(String::new());
    }
    if maximum == 1 {
        return fallible_copy("…", "ellipsized text");
    }

    let mut start = text.len();
    let maximum = u32::from(maximum);
    let mut used = 1_u32;
    for (offset, grapheme) in text.grapheme_indices(true).rev() {
        let width = u32::from(display_width(grapheme)?);
        if used + width > maximum {
            break;
        }
        start = offset;
        used += width;
    }
    let mut output = String::new();
    output
        .try_reserve_exact("…".len() + text.len() - start)
        .map_err(|_| TutError::Allocation("ellipsized text"))?;
    output.push('…');
    output.push_str(&text[start..]);
    Ok(output)
}

fn fallible_copy(text: &str, context: &'static str) -> Result<String, TutError> {
    let mut output = String::new();
    output
        .try_reserve_exact(text.len())
        .map_err(|_| TutError::Allocation(context))?;
    output.push_str(text);
    Ok(output)
}

fn display_width(text: &str) -> Result<u16, TutError> {
    let content_width = ContentWidth::new(u16::MAX).expect("u16::MAX is nonzero");
    let mut column = DisplayColumn::ZERO;
    for atom in DisplayAtoms::new(text) {
        let Some(projected) = atom.project(column, content_width) else {
            continue;
        };
        column = DisplayColumn::new(column.get().saturating_add(projected.width().get()));
    }
    Ok(u16::try_from(column.get()).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{Action, Geometry, app_from_text};

    fn draw(app: &mut crate::app::App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = app.render_state().unwrap();
        terminal
            .draw(|frame| render(frame, &state).unwrap())
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_text(buffer: &Buffer, row: u16) -> String {
        (0..buffer.area.width)
            .map(|column| buffer.cell((column, row)).unwrap().symbol())
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn frame_renders_fixed_regions_and_empty_progress() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), String::new());
        app.update(Action::Resize(Geometry::new(40, 5))).unwrap();
        let buffer = draw(&mut app, 40, 5);
        assert!(row_text(&buffer, 0).starts_with("book.txt"));
        assert_eq!(row_text(&buffer, 3), "100%  1/1");
        assert!(row_text(&buffer, 4).contains("move"));
    }

    #[test]
    fn body_uses_highlight_and_forced_width_metadata() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "ｶﾞ cat".to_owned());
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        app.update(Action::BeginSearch).unwrap();
        for character in "cat".chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }
        app.update(Action::SearchCommit).unwrap();
        let buffer = draw(&mut app, 20, 4);
        let first = buffer.cell((0, 1)).unwrap();
        assert_eq!(first.symbol(), "ｶﾞ");
        assert_eq!(
            first.diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap())
        );
        let current = buffer.cell((2, 1)).unwrap();
        assert!(current.modifier.contains(Modifier::REVERSED));
        assert!(current.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn long_queries_preserve_fixed_status_indicators() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        app.update(Action::Resize(Geometry::new(48, 4))).unwrap();
        app.update(Action::BeginSearch).unwrap();
        for character in "q".repeat(128).chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }
        app.update(Action::SearchCommit).unwrap();
        assert!(row_text(&draw(&mut app, 48, 4), 2).ends_with("— no matches"));
    }

    #[test]
    fn tiny_frames_render_only_the_resize_message() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        let buffer = draw(&mut app, 12, 3);
        assert_eq!(row_text(&buffer, 0), "terminal too");
        assert_eq!(row_text(&buffer, 1), "");
    }

    #[test]
    fn ellipsis_preserves_grapheme_boundaries() {
        assert_eq!(ellipsize_end("aébc", 3).unwrap(), "aé…");
        assert_eq!(ellipsize_start("aébc", 3).unwrap(), "…bc");

        let long = "a".repeat(usize::from(u16::MAX) + 1);
        assert_eq!(
            display_width(&ellipsize_end(&long, u16::MAX).unwrap()).unwrap(),
            u16::MAX
        );
        assert_eq!(
            display_width(&ellipsize_start(&long, u16::MAX).unwrap()).unwrap(),
            u16::MAX
        );
    }
}
