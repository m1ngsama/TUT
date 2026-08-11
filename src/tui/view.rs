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
        Highlight, MIN_TERMINAL_COLUMNS, MIN_TERMINAL_ROWS, MatchCursor, RenderProjectionKind,
        RenderRow, RenderRowsView, RenderSpan, RenderState, SearchStatus, ViewState,
    },
    error::{TutError, sanitize_text},
    layout::{ContentWidth, DisplayAtoms, DisplayColumn},
};

const TINY_MESSAGE: &str = "terminal too small — resize";
const READER_FOOTER: &str =
    "F1 help  q quit  j/k lines  Space/b pages  / search  n/N matches  g/G ends";
const SEARCH_FOOTER: &str = "F1 help  Enter apply  Esc cancel  Backspace delete  Ctrl-C interrupt";
const HELP_TITLE: &str = "TUT keyboard help";
const HELP_FOOTER: &str = "F1 / Esc close help";
const READER_HELP_FOOTER: &str = "F1 / Esc / q close help";
const COMPACT_HELP: &[&str] = &[
    "j/k or Up/Down        move by line",
    "/ search   n/N next/previous match",
    "Space/b or PgDn/PgUp  move by page",
    "g/G or Home/End       document ends",
    "Ctrl-D/Ctrl-U         move by half page",
    "Esc                   cancel or clear search",
    "q quits reader        Ctrl-C interrupts",
];
const FULL_HELP: &[&str] = &[
    "Navigation",
    "  j / Down             next line",
    "  k / Up               previous line",
    "  Space / PageDown     next page",
    "  b / PageUp           previous page",
    "  Ctrl-F / Ctrl-B      next / previous page",
    "  Ctrl-D / Ctrl-U      half page down / up",
    "  g / Home             document start",
    "  G / End              document end",
    "Search",
    "  /                    enter search",
    "  Enter                apply search",
    "  Esc                  cancel or clear search",
    "  n / N                next / previous match",
    "General",
    "  F1                   open / close help",
    "  q                    quit from reader mode",
    "  Ctrl-C               interrupt",
];

pub(super) fn render(frame: &mut Frame<'_>, state: &ViewState<'_>) -> Result<(), TutError> {
    let area = frame.area();
    if area.width < MIN_TERMINAL_COLUMNS || area.height < MIN_TERMINAL_ROWS {
        render_projected_line(
            frame,
            Rect::new(area.x, area.y, area.width, area.height.min(1)),
            TINY_MESSAGE,
        )?;
        return Ok(());
    }

    match state {
        ViewState::Reader(state) => render_reader(frame, state),
        ViewState::Help { q_closes } => render_help(frame, *q_closes),
    }
}

fn render_reader(frame: &mut Frame<'_>, state: &RenderState<'_>) -> Result<(), TutError> {
    let area = frame.area();
    let header_text = header_text(state, area.width)?;
    let status_text = status_text(state, area.width)?;
    let footer = match state.status {
        SearchStatus::Draft { .. } => SEARCH_FOOTER,
        SearchStatus::None | SearchStatus::Committed { .. } => READER_FOOTER,
    };
    let help_text = ellipsize_end(footer, area.width)?;
    let body_height = area.height - 3;
    let header = Rect::new(area.x, area.y, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, body_height);
    let status = Rect::new(area.x, area.y + 1 + body_height, area.width, 1);
    let help = Rect::new(area.x, area.y + 2 + body_height, area.width, 1);

    render_projected_line(frame, header, &header_text)?;
    render_body(frame, body, state.rows);
    render_projected_line(frame, status, &status_text)?;
    render_projected_line(frame, help, &help_text)?;
    Ok(())
}

fn render_help(frame: &mut Frame<'_>, q_closes: bool) -> Result<(), TutError> {
    let area = frame.area();
    let title = Rect::new(area.x, area.y, area.width, 1);
    let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let body_height = area.height - 2;
    let lines = if usize::from(body_height) >= FULL_HELP.len() {
        FULL_HELP
    } else {
        COMPACT_HELP
    };

    render_projected_line(frame, title, HELP_TITLE)?;
    for (relative_y, line) in lines.iter().take(usize::from(body_height)).enumerate() {
        let y = area.y + 1 + u16::try_from(relative_y).expect("help rows fit the terminal height");
        render_projected_line(frame, Rect::new(area.x, y, area.width, 1), line)?;
    }
    let footer_text = ellipsize_end(
        if q_closes {
            READER_HELP_FOOTER
        } else {
            HELP_FOOTER
        },
        area.width,
    )?;
    render_projected_line(frame, footer, &footer_text)
}

fn render_body(frame: &mut Frame<'_>, area: Rect, rows: RenderRowsView<'_>) {
    frame.render_widget(ReaderBody { rows }, area);
}

struct ReaderBody<'a> {
    rows: RenderRowsView<'a>,
}

impl Widget for ReaderBody<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        let mut highlights = self.rows.highlight_cursor();
        for (relative_y, row) in self.rows.iter().take(usize::from(area.height)).enumerate() {
            let y =
                area.y + u16::try_from(relative_y).expect("visible row count fits terminal height");
            write_render_row(buffer, area, y, row, &mut highlights);
        }
    }
}

fn write_render_row(
    buffer: &mut Buffer,
    area: Rect,
    y: u16,
    row: RenderRow<'_>,
    highlights: &mut MatchCursor<'_>,
) {
    let mut x = area.x;
    let mut pending: Option<(RenderSpan, Highlight)> = None;
    for span in row.spans {
        let highlight = highlights.role_for(span.source());
        if let Some((current, current_highlight)) = pending.as_mut()
            && *current_highlight == highlight
            && current.merge(span)
        {
            continue;
        }
        if let Some((current, current_highlight)) = pending.take()
            && !write_render_run(buffer, area, &mut x, y, row, &current, current_highlight)
        {
            return;
        }
        pending = Some((span.clone(), highlight));
    }
    if let Some((span, highlight)) = pending {
        write_render_run(buffer, area, &mut x, y, row, &span, highlight);
    }
}

fn write_render_run(
    buffer: &mut Buffer,
    area: Rect,
    x: &mut u16,
    y: u16,
    row: RenderRow<'_>,
    span: &RenderSpan,
    highlight: Highlight,
) -> bool {
    let width =
        u16::try_from(span.cell_width.get()).expect("projected width fits the terminal width");
    if width > area.right().saturating_sub(*x) {
        return false;
    }
    *x += write_render_span(buffer, *x, y, row.span_text(span), span, highlight);
    true
}

fn write_render_span(
    buffer: &mut Buffer,
    x: u16,
    y: u16,
    text: &str,
    span: &RenderSpan,
    highlight: Highlight,
) -> u16 {
    let width =
        u16::try_from(span.cell_width.get()).expect("projected width fits the terminal width");
    let style = style_for(highlight);
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
    cell.set_symbol(text)
        .set_style(style)
        .set_diff_option(CellDiffOption::ForcedWidth(forced_width));
    for offset in 1..width {
        let cell = buffer
            .cell_mut((x + offset, y))
            .expect("span was clipped to the buffer area");
        cell.reset();
        cell.set_diff_option(CellDiffOption::Skip);
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
        let span = RenderSpan::from_projected(projected, &mut row)?;
        let width =
            u16::try_from(span.cell_width.get()).expect("projected width fits the terminal width");
        if width > area.right().saturating_sub(x) {
            break;
        }
        x += write_render_span(
            frame.buffer_mut(),
            x,
            area.y,
            span.standalone_text(&row),
            &span,
            Highlight::None,
        );
        column = DisplayColumn::new(column.get() + u32::from(width));
    }
    Ok(())
}

fn header_text(state: &RenderState<'_>, width: u16) -> Result<String, TutError> {
    let filename = sanitize_text(state.filename);
    let path = sanitize_text(state.path);
    if filename == path {
        return ellipsize_end(&filename, width);
    }
    let shown_name = ellipsize_end(&filename, (width / 3).max(1))?;
    let used = display_width(&shown_name);
    if used >= width {
        return Ok(shown_name);
    }

    let separator = if width - used >= 2 { "  " } else { " " };
    let remaining = width.saturating_sub(used + display_width(separator));
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
            searching: true,
            ..
        } => (Some(sanitize_text(query)), " — searching"),
        SearchStatus::Committed {
            query,
            no_matches: true,
            searching: false,
        } => (Some(sanitize_text(query)), " — no matches"),
        SearchStatus::Committed {
            query,
            no_matches: false,
            searching: false,
        } => (Some(sanitize_text(query)), ""),
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
    match (state.current_line, state.total_lines) {
        (Some(current), Some(total)) => {
            write!(prefix, "{}%  {current}/{total}", state.progress.min(100))
        }
        (Some(current), None) => write!(prefix, "{}%  {current}/?", state.progress.min(100)),
        (None, Some(total)) => write!(prefix, "{}%  ?/{total}", state.progress.min(100)),
        (None, None) => write!(prefix, "{}%  ?/?", state.progress.min(100)),
    }
    .expect("reserved String formatting is infallible");
    if query.is_some() {
        prefix.push_str("  /");
    }
    let fixed_width = display_width(&prefix).saturating_add(display_width(suffix));
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
    if display_width(text) <= maximum {
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
        let width = u32::from(display_width(grapheme));
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
    if display_width(text) <= maximum {
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
        let width = u32::from(display_width(grapheme));
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

fn display_width(text: &str) -> u16 {
    let content_width = ContentWidth::new(u16::MAX).expect("u16::MAX is nonzero");
    let mut column = DisplayColumn::ZERO;
    for atom in DisplayAtoms::new(text) {
        let Some(projected) = atom.project(column, content_width) else {
            continue;
        };
        column = DisplayColumn::new(column.get().saturating_add(projected.width().get()));
    }
    u16::try_from(column.get()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{Action, Geometry, app_from_text};

    fn prepare_frame(app: &mut crate::app::App) {
        for _ in 0..1024 {
            if app.frame_ready() {
                return;
            }
            app.advance_background().unwrap();
        }
        panic!("render work exceeded the test step limit");
    }

    fn draw_into(terminal: &mut Terminal<TestBackend>, app: &mut crate::app::App) {
        prepare_frame(app);
        let state = app.view_state().unwrap();
        terminal
            .draw(|frame| render(frame, &state).unwrap())
            .unwrap();
    }

    fn draw(app: &mut crate::app::App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        draw_into(&mut terminal, app);
        terminal.backend().buffer().clone()
    }

    fn body_buffer(app: &mut crate::app::App, width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        prepare_frame(app);
        let state = app.render_state().unwrap();
        ReaderBody { rows: state.rows }.render(area, &mut buffer);
        buffer
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
        assert!(row_text(&buffer, 4).starts_with("F1 help"));
    }

    #[test]
    fn footer_tracks_search_input_context() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        app.update(Action::Resize(Geometry::new(48, 4))).unwrap();
        app.update(Action::BeginSearch).unwrap();

        let buffer = draw(&mut app, 48, 4);
        assert!(row_text(&buffer, 3).starts_with("F1 help  Enter apply  Esc cancel"));

        app.update(Action::ShowHelp).unwrap();
        let buffer = draw(&mut app, 48, 4);
        assert_eq!(row_text(&buffer, 3), "F1 / Esc close help");
    }

    #[test]
    fn help_renders_full_and_compact_keyboard_guides() {
        let mut full = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        full.update(Action::Resize(Geometry::new(80, 24))).unwrap();
        full.update(Action::ShowHelp).unwrap();
        let full = draw(&mut full, 80, 24);
        assert_eq!(row_text(&full, 0), "TUT keyboard help");
        assert_eq!(row_text(&full, 1), "Navigation");
        assert_eq!(row_text(&full, 10), "Search");
        assert_eq!(row_text(&full, 15), "General");
        assert_eq!(row_text(&full, 23), "F1 / Esc / q close help");

        let mut compact = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        compact
            .update(Action::Resize(Geometry::new(16, 4)))
            .unwrap();
        compact.update(Action::ShowHelp).unwrap();
        let compact = draw(&mut compact, 16, 4);
        assert!(row_text(&compact, 0).starts_with("TUT keyboard"));
        assert!(row_text(&compact, 1).starts_with("j/k or Up/Down"));
        assert!(row_text(&compact, 2).starts_with("/ search"));
        assert!(row_text(&compact, 3).starts_with("F1 / Esc"));
    }

    #[test]
    fn dismissing_help_restores_reader_content_and_clears_overlay_rows() {
        let backend = TestBackend::new(40, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "reader body".to_owned());
        app.update(Action::Resize(Geometry::new(40, 8))).unwrap();
        draw_into(&mut terminal, &mut app);

        app.update(Action::ShowHelp).unwrap();
        draw_into(&mut terminal, &mut app);
        assert!(row_text(terminal.backend().buffer(), 2).starts_with("/ search"));

        app.update(Action::DismissHelp).unwrap();
        draw_into(&mut terminal, &mut app);
        assert_eq!(
            terminal.backend().buffer().cell((0, 1)).unwrap().symbol(),
            "reader body"
        );
        assert_eq!(row_text(terminal.backend().buffer(), 2), "");
        assert!(row_text(terminal.backend().buffer(), 7).starts_with("F1 help"));
    }

    #[test]
    fn body_uses_highlight_and_forced_width_metadata() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "ｶﾞ cat".to_owned());
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        prepare_frame(&mut app);
        app.update(Action::BeginSearch).unwrap();
        for character in "cat".chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }
        app.update(Action::SearchCommit).unwrap();
        app.advance_background().unwrap();
        let buffer = draw(&mut app, 20, 4);
        let first = buffer.cell((0, 1)).unwrap();
        assert_eq!(first.symbol(), "ｶﾞ ");
        assert_eq!(
            first.diff_option,
            CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap())
        );
        let current = buffer.cell((2, 1)).unwrap();
        assert!(current.modifier.contains(Modifier::REVERSED));
        assert!(current.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn compacted_spans_clear_cells_when_the_next_frame_is_shorter() {
        let mut long = app_from_text(Path::new("/tmp/book.txt"), "abcdef".to_owned());
        long.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        let previous = body_buffer(&mut long, 20, 1);

        let mut short = app_from_text(Path::new("/tmp/book.txt"), "a".to_owned());
        short.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        let next = body_buffer(&mut short, 20, 1);
        let body_updates: Vec<_> = previous
            .diff_iter(&next)
            .filter_map(|(x, y, _)| (y == 0).then_some(x))
            .collect();

        assert_eq!(body_updates, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn consecutive_terminal_frames_clear_shortened_body_cells() {
        let backend = TestBackend::new(20, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "abcdef\nx".to_owned());
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();

        draw_into(&mut terminal, &mut app);
        assert_eq!(row_text(terminal.backend().buffer(), 1), "abcdef");

        app.update(Action::LineDown).unwrap();
        for _ in 0..16 {
            if !app.has_background_work() {
                break;
            }
            app.advance_background().unwrap();
        }
        assert!(!app.has_background_work());
        draw_into(&mut terminal, &mut app);

        assert_eq!(row_text(terminal.backend().buffer(), 1), "x");
        for column in 1..6 {
            let cell = terminal.backend().buffer().cell((column, 1)).unwrap();
            assert_eq!(cell.symbol(), " ");
            assert_eq!(cell.modifier, Modifier::empty());
            assert_eq!(cell.diff_option, CellDiffOption::None);
        }
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
        assert!(row_text(&draw(&mut app, 48, 4), 2).ends_with("— searching"));
        app.advance_background().unwrap();
        assert!(row_text(&draw(&mut app, 48, 4), 2).ends_with("— no matches"));
    }

    #[test]
    fn incomplete_line_coordinates_are_explicit() {
        let state = RenderState {
            filename: "book.txt",
            path: "/tmp/book.txt",
            rows: RenderRowsView::empty(),
            progress: 12,
            current_line: Some(7),
            total_lines: None,
            status: SearchStatus::None,
        };
        assert_eq!(status_text(&state, 40).unwrap(), "12%  7/?");

        let state = RenderState {
            current_line: None,
            ..state
        };
        assert_eq!(status_text(&state, 40).unwrap(), "12%  ?/?");
    }

    #[test]
    fn identical_source_names_are_not_repeated_in_the_header() {
        let state = RenderState {
            filename: "standard input",
            path: "standard input",
            rows: RenderRowsView::empty(),
            progress: 100,
            current_line: Some(1),
            total_lines: Some(1),
            status: SearchStatus::None,
        };
        assert_eq!(header_text(&state, 40).unwrap(), "standard input");
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
            display_width(&ellipsize_end(&long, u16::MAX).unwrap()),
            u16::MAX
        );
        assert_eq!(
            display_width(&ellipsize_start(&long, u16::MAX).unwrap()),
            u16::MAX
        );
    }
}
