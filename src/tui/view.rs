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
        Highlight, MIN_TERMINAL_COLUMNS, MIN_TERMINAL_ROWS, MatchCursor, PendingState,
        RenderProjectionKind, RenderRow, RenderRowsView, RenderSpan, RenderState, SearchStatus,
        ViewActivity, ViewState, ViewportBoundary,
    },
    error::{TutError, sanitize_text},
    layout::{ContentWidth, DisplayAtoms, DisplayColumn},
};

const PENDING_MESSAGE: &str = "Preparing view…";
const HELP_TITLE: &str = "TUT keyboard help";

#[derive(Clone, Copy)]
struct CopyTier {
    min_columns: u16,
    text: &'static str,
}

const READER_FOOTER: &[CopyTier] = &[
    CopyTier {
        min_columns: 80,
        text: "q quit  F1 help  / search  j/k lines  Space/b pages  n/N matches  g/G ends",
    },
    CopyTier {
        min_columns: 40,
        text: "q quit  F1 help  / search  j/k  Space/b",
    },
    CopyTier {
        min_columns: 20,
        text: "q quit  F1 help  /",
    },
    CopyTier {
        min_columns: 16,
        text: "q quit  F1 help",
    },
];
const COMMITTED_SEARCH_FOOTER: &[CopyTier] = &[
    CopyTier {
        min_columns: 80,
        text: "Esc clear search  n/N matches  q quit  F1 help  / new search  j/k lines",
    },
    CopyTier {
        min_columns: 40,
        text: "Esc clear  n/N matches  q quit  F1 help",
    },
    CopyTier {
        min_columns: 20,
        text: "Esc clear q quit n/N",
    },
    CopyTier {
        min_columns: 16,
        text: "Esc clear q quit",
    },
];
const SEARCH_INPUT_FOOTER: &[CopyTier] = &[
    CopyTier {
        min_columns: 80,
        text: "Esc cancel  Enter apply  Backspace delete  F1 help  Ctrl-C interrupt",
    },
    CopyTier {
        min_columns: 40,
        text: "Esc cancel  Enter  Backspace  F1 help",
    },
    CopyTier {
        min_columns: 20,
        text: "Esc cancel  Enter F1",
    },
    CopyTier {
        min_columns: 16,
        text: "Esc cancel Enter",
    },
];
const READER_HELP_FOOTER: &[CopyTier] = &[
    CopyTier {
        min_columns: 80,
        text: "Esc/q/F1 close help  Ctrl-C interrupt",
    },
    CopyTier {
        min_columns: 40,
        text: "Esc/q/F1 close help  Ctrl-C interrupt",
    },
    CopyTier {
        min_columns: 20,
        text: "Esc/q/F1 close help",
    },
    CopyTier {
        min_columns: 16,
        text: "Esc/q/F1 close",
    },
];
const SEARCH_HELP_FOOTER: &[CopyTier] = &[
    CopyTier {
        min_columns: 80,
        text: "Esc/F1 close help  Ctrl-C interrupt",
    },
    CopyTier {
        min_columns: 40,
        text: "Esc/F1 close help  Ctrl-C interrupt",
    },
    CopyTier {
        min_columns: 20,
        text: "Esc/F1 close help",
    },
    CopyTier {
        min_columns: 16,
        text: "Esc/F1 close",
    },
];
const TINY_COPY: &[CopyTier] = &[
    CopyTier {
        min_columns: 80,
        text: "terminal too small  resize  q quit  Ctrl-C interrupt",
    },
    CopyTier {
        min_columns: 40,
        text: "terminal too small  resize  q quit",
    },
    CopyTier {
        min_columns: 14,
        text: "resize  q quit",
    },
    CopyTier {
        min_columns: 8,
        text: "resize q",
    },
    CopyTier {
        min_columns: 6,
        text: "resize",
    },
    CopyTier {
        min_columns: 1,
        text: "q",
    },
];
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
            pick_copy(area.width, TINY_COPY),
        )?;
        return Ok(());
    }

    match state {
        ViewState::Reader(state) => render_reader(frame, state),
        ViewState::Pending(state) => render_pending(frame, state),
        ViewState::Help { q_closes } => render_help(frame, *q_closes),
    }
}

fn render_reader(frame: &mut Frame<'_>, state: &RenderState<'_>) -> Result<(), TutError> {
    let area = frame.area();
    let header_text = header_text(state.filename, state.path, area.width)?;
    let status_text = status_text(state, area.width)?;
    let help_text = footer_for(state.status, area.width);
    let body_height = area.height - 3;
    let header = Rect::new(area.x, area.y, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, body_height);
    let status = Rect::new(area.x, area.y + 1 + body_height, area.width, 1);
    let help = Rect::new(area.x, area.y + 2 + body_height, area.width, 1);

    render_projected_line(frame, header, &header_text)?;
    render_body(frame, body, state.rows);
    render_projected_line(frame, status, &status_text)?;
    render_projected_line(frame, help, help_text)?;
    Ok(())
}

fn render_pending(frame: &mut Frame<'_>, state: &PendingState<'_>) -> Result<(), TutError> {
    let area = frame.area();
    let header_text = header_text(state.filename, state.path, area.width)?;
    let status_text = pending_status_text(state.status, area.width)?;
    let footer_text = footer_for(state.status, area.width);
    let body_height = area.height - 3;
    let header = Rect::new(area.x, area.y, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, body_height);
    let status = Rect::new(area.x, area.y + 1 + body_height, area.width, 1);
    let footer = Rect::new(area.x, area.y + 2 + body_height, area.width, 1);

    render_projected_line(frame, header, &header_text)?;
    render_centered_line(frame, body, PENDING_MESSAGE)?;
    render_projected_line(frame, status, &status_text)?;
    render_projected_line(frame, footer, footer_text)
}

fn pick_copy(columns: u16, tiers: &[CopyTier]) -> &'static str {
    tiers
        .iter()
        .find(|tier| columns >= tier.min_columns)
        .map_or("", |tier| tier.text)
}

fn footer_for(status: SearchStatus<'_>, columns: u16) -> &'static str {
    match status {
        SearchStatus::None => pick_copy(columns, READER_FOOTER),
        SearchStatus::Committed { .. } => pick_copy(columns, COMMITTED_SEARCH_FOOTER),
        SearchStatus::Draft { .. } => pick_copy(columns, SEARCH_INPUT_FOOTER),
    }
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
    let footer_text = pick_copy(
        area.width,
        if q_closes {
            READER_HELP_FOOTER
        } else {
            SEARCH_HELP_FOOTER
        },
    );
    render_projected_line(frame, footer, footer_text)
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

fn render_centered_line(frame: &mut Frame<'_>, area: Rect, text: &str) -> Result<(), TutError> {
    if area.width == 0 || area.height == 0 {
        return Ok(());
    }
    let shown = ellipsize_end(text, area.width)?;
    let used = display_width(&shown);
    let x = area.x + area.width.saturating_sub(used) / 2;
    let y = area.y + area.height.saturating_sub(1) / 2;
    render_projected_line(
        frame,
        Rect::new(x, y, area.right().saturating_sub(x), 1),
        &shown,
    )
}

fn header_text(filename: &str, path: &str, width: u16) -> Result<String, TutError> {
    let filename = sanitize_text(filename);
    let path = sanitize_text(path);
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
    let mut prefix = String::new();
    prefix
        .try_reserve_exact(50)
        .map_err(|_| TutError::Allocation("status text"))?;
    match state.boundary {
        Some(ViewportBoundary::Top) => prefix.push_str("TOP"),
        Some(ViewportBoundary::End) => prefix.push_str("END"),
        Some(ViewportBoundary::All) => prefix.push_str("ALL"),
        None => write!(prefix, "{}%", state.progress.min(100))
            .expect("reserved String formatting is infallible"),
    }
    match (state.current_line, state.total_lines) {
        (Some(current), Some(total)) => write!(prefix, "  {current}/{total}"),
        (Some(current), None) => write!(prefix, "  {current}/?"),
        (None, Some(total)) => write!(prefix, "  ?/{total}"),
        (None, None) => write!(prefix, "  ?/?"),
    }
    .expect("reserved String formatting is infallible");
    compose_status(prefix, state.status, state.activity, width)
}

fn pending_status_text(status: SearchStatus<'_>, width: u16) -> Result<String, TutError> {
    compose_status(String::new(), status, None, width)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusNotice {
    Searching,
    NoMatches,
    QueryLimit,
    PreparingView,
    GoingToEnd,
}

impl StatusNotice {
    const fn text(self) -> &'static str {
        match self {
            Self::Searching => "searching",
            Self::NoMatches => "no matches",
            Self::QueryLimit => "query limit: 4096 bytes",
            Self::PreparingView => "preparing view",
            Self::GoingToEnd => "going to end",
        }
    }

    const fn compact_text(self) -> &'static str {
        match self {
            Self::QueryLimit => "query limit",
            _ => self.text(),
        }
    }

    const fn suffix(self) -> &'static str {
        match self {
            Self::Searching => " — searching",
            Self::NoMatches => " — no matches",
            Self::QueryLimit => " — query limit: 4096 bytes",
            Self::PreparingView => " — preparing view",
            Self::GoingToEnd => " — going to end",
        }
    }
}

fn compose_status(
    mut prefix: String,
    status: SearchStatus<'_>,
    activity: Option<ViewActivity>,
    width: u16,
) -> Result<String, TutError> {
    let (query, search_notice, preserve_query_tail) = match status {
        SearchStatus::None => (None, None, false),
        SearchStatus::Committed {
            query,
            searching: true,
            ..
        } => (
            Some(sanitize_text(query)),
            Some(StatusNotice::Searching),
            false,
        ),
        SearchStatus::Committed {
            query,
            no_matches: true,
            searching: false,
        } => (
            Some(sanitize_text(query)),
            Some(StatusNotice::NoMatches),
            false,
        ),
        SearchStatus::Committed {
            query,
            no_matches: false,
            searching: false,
        } => (Some(sanitize_text(query)), None, false),
        SearchStatus::Draft {
            draft,
            limit_hit: false,
        } => (Some(sanitize_text(draft)), None, true),
        SearchStatus::Draft {
            draft,
            limit_hit: true,
        } => (
            Some(sanitize_text(draft)),
            Some(StatusNotice::QueryLimit),
            true,
        ),
    };
    let notice = search_notice.or_else(|| {
        activity.map(|activity| match activity {
            ViewActivity::PreparingView => StatusNotice::PreparingView,
            ViewActivity::GoingToEnd => StatusNotice::GoingToEnd,
        })
    });

    if query.is_some() {
        prefix.push_str(if prefix.is_empty() { "/" } else { "  /" });
    }
    let suffix = notice.map_or("", StatusNotice::suffix);
    let fixed_width = display_width(&prefix).saturating_add(display_width(suffix));
    if fixed_width > width {
        if let Some(notice) = notice {
            let notice = if display_width(notice.text()) <= width {
                notice.text()
            } else {
                notice.compact_text()
            };
            return ellipsize_end(notice, width);
        }
        return ellipsize_end(&prefix, width);
    }
    let query_budget = width.saturating_sub(fixed_width);
    let shown_query = match query.as_deref() {
        Some(query) if preserve_query_tail => ellipsize_start(query, query_budget)?,
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
        draw_available_into(terminal, app);
    }

    fn draw_available_into(terminal: &mut Terminal<TestBackend>, app: &mut crate::app::App) {
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

    fn draw_available(app: &mut crate::app::App, width: u16, height: u16) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        draw_available_into(&mut terminal, app);
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

    fn assert_copy_tiers(tiers: &[CopyTier]) {
        assert!(
            tiers
                .windows(2)
                .all(|pair| pair[0].min_columns > pair[1].min_columns)
        );
        for (index, tier) in tiers.iter().enumerate() {
            assert_eq!(pick_copy(tier.min_columns, tiers), tier.text);
            assert_eq!(
                pick_copy(tier.min_columns.saturating_add(1), tiers),
                tier.text
            );
            let narrower = tier.min_columns.saturating_sub(1);
            let expected = tiers.get(index + 1).map_or("", |next| next.text);
            assert_eq!(pick_copy(narrower, tiers), expected);
            assert!(tier.text.is_ascii());
            assert!(!tier.text.contains('…'));
            assert!(display_width(tier.text) <= tier.min_columns);
        }
    }

    #[test]
    fn responsive_copy_tiers_are_complete_and_fit_their_breakpoints() {
        for tiers in [
            READER_FOOTER,
            COMMITTED_SEARCH_FOOTER,
            SEARCH_INPUT_FOOTER,
            READER_HELP_FOOTER,
            SEARCH_HELP_FOOTER,
            TINY_COPY,
        ] {
            assert_copy_tiers(tiers);
        }

        let committed = SearchStatus::Committed {
            query: "needle",
            no_matches: false,
            searching: false,
        };
        let draft = SearchStatus::Draft {
            draft: "needle",
            limit_hit: false,
        };
        assert_eq!(footer_for(SearchStatus::None, 16), "q quit  F1 help");
        assert_eq!(footer_for(SearchStatus::None, 20), "q quit  F1 help  /");
        assert_eq!(
            footer_for(SearchStatus::None, 40),
            "q quit  F1 help  / search  j/k  Space/b"
        );
        assert_eq!(
            footer_for(SearchStatus::None, 80),
            "q quit  F1 help  / search  j/k lines  Space/b pages  n/N matches  g/G ends"
        );
        assert_eq!(footer_for(committed, 16), "Esc clear q quit");
        assert_eq!(footer_for(committed, 20), "Esc clear q quit n/N");
        assert_eq!(
            footer_for(committed, 40),
            "Esc clear  n/N matches  q quit  F1 help"
        );
        assert_eq!(footer_for(draft, 16), "Esc cancel Enter");
        assert_eq!(footer_for(draft, 20), "Esc cancel  Enter F1");
        assert_eq!(
            footer_for(draft, 40),
            "Esc cancel  Enter  Backspace  F1 help"
        );
        assert_eq!(pick_copy(16, READER_HELP_FOOTER), "Esc/q/F1 close");
        assert_eq!(pick_copy(16, SEARCH_HELP_FOOTER), "Esc/F1 close");
        assert_eq!(pick_copy(0, TINY_COPY), "");
        assert_eq!(pick_copy(6, TINY_COPY), "resize");
        assert_eq!(pick_copy(12, TINY_COPY), "resize q");
        assert_eq!(pick_copy(16, TINY_COPY), "resize  q quit");
    }

    #[test]
    fn frame_renders_fixed_regions_and_empty_progress() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), String::new());
        app.update(Action::Resize(Geometry::new(40, 5))).unwrap();
        let buffer = draw(&mut app, 40, 5);
        assert!(row_text(&buffer, 0).starts_with("book.txt"));
        assert_eq!(row_text(&buffer, 3), "ALL  1/1");
        assert_eq!(
            row_text(&buffer, 4),
            "q quit  F1 help  / search  j/k  Space/b"
        );
    }

    #[test]
    fn footer_tracks_search_input_context() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        app.update(Action::Resize(Geometry::new(48, 4))).unwrap();
        app.update(Action::BeginSearch).unwrap();

        let buffer = draw(&mut app, 48, 4);
        assert_eq!(
            row_text(&buffer, 3),
            "Esc cancel  Enter  Backspace  F1 help"
        );

        app.update(Action::ShowHelp).unwrap();
        let buffer = draw(&mut app, 48, 4);
        assert_eq!(row_text(&buffer, 3), "Esc/F1 close help  Ctrl-C interrupt");
    }

    #[test]
    fn responsive_footer_clears_hints_across_mode_and_width_changes() {
        let backend = TestBackend::new(80, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        app.update(Action::Resize(Geometry::new(80, 4))).unwrap();
        draw_into(&mut terminal, &mut app);
        assert_eq!(
            row_text(terminal.backend().buffer(), 3),
            "q quit  F1 help  / search  j/k lines  Space/b pages  n/N matches  g/G ends"
        );

        terminal.backend_mut().resize(40, 4);
        app.update(Action::Resize(Geometry::new(40, 4))).unwrap();
        draw_into(&mut terminal, &mut app);
        assert_eq!(
            row_text(terminal.backend().buffer(), 3),
            "q quit  F1 help  / search  j/k  Space/b"
        );

        app.update(Action::BeginSearch).unwrap();
        draw_into(&mut terminal, &mut app);
        assert_eq!(
            row_text(terminal.backend().buffer(), 3),
            "Esc cancel  Enter  Backspace  F1 help"
        );
        for column in 37..40 {
            assert_eq!(
                terminal
                    .backend()
                    .buffer()
                    .cell((column, 3))
                    .unwrap()
                    .symbol(),
                " "
            );
        }

        terminal.backend_mut().resize(16, 4);
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        draw_into(&mut terminal, &mut app);
        assert_eq!(row_text(terminal.backend().buffer(), 3), "Esc cancel Enter");
    }

    #[test]
    fn pending_shell_is_immediate_static_and_responsive() {
        let mut normal = app_from_text(Path::new("/tmp/資料e\u{301}/book.txt"), "x".repeat(4096));
        normal
            .update(Action::Resize(Geometry::new(80, 24)))
            .unwrap();
        assert!(!normal.frame_ready());
        let normal = draw_available(&mut normal, 80, 24);
        assert!(row_text(&normal, 0).starts_with("book.txt"));
        assert_eq!(row_text(&normal, 11).trim(), "Preparing view…");
        assert_eq!(row_text(&normal, 22), "");
        assert_eq!(
            row_text(&normal, 23),
            "q quit  F1 help  / search  j/k lines  Space/b pages  n/N matches  g/G ends"
        );

        let mut minimum = app_from_text(Path::new("/tmp/book.txt"), "x".repeat(4096));
        minimum
            .update(Action::Resize(Geometry::new(16, 4)))
            .unwrap();
        assert!(!minimum.frame_ready());
        let minimum = draw_available(&mut minimum, 16, 4);
        assert_eq!(row_text(&minimum, 1), "Preparing view…");
        assert_eq!(row_text(&minimum, 2), "");
        assert_eq!(row_text(&minimum, 3), "q quit  F1 help");
    }

    #[test]
    fn pending_shell_preserves_unicode_search_input_without_document_rows() {
        let mut app = app_from_text(Path::new("/tmp/資料e\u{301}.txt"), "x".repeat(4096));
        app.update(Action::Resize(Geometry::new(40, 4))).unwrap();
        app.update(Action::BeginSearch).unwrap();
        for character in "prefixe\u{301}終".chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }

        let buffer = draw_available(&mut app, 40, 4);
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "資");
        assert_eq!(buffer.cell((2, 0)).unwrap().symbol(), "料");
        assert_eq!(buffer.cell((4, 0)).unwrap().symbol(), "é");
        assert_eq!(row_text(&buffer, 1).trim(), "Preparing view…");
        assert_eq!(row_text(&buffer, 2), "/prefixé終");
        assert_eq!(
            row_text(&buffer, 3),
            "Esc cancel  Enter  Backspace  F1 help"
        );
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
        assert_eq!(row_text(&full, 23), "Esc/q/F1 close help  Ctrl-C interrupt");

        let mut compact = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        compact
            .update(Action::Resize(Geometry::new(16, 4)))
            .unwrap();
        compact.update(Action::ShowHelp).unwrap();
        let compact = draw(&mut compact, 16, 4);
        assert!(row_text(&compact, 0).starts_with("TUT keyboard"));
        assert!(row_text(&compact, 1).starts_with("j/k or Up/Down"));
        assert!(row_text(&compact, 2).starts_with("/ search"));
        assert_eq!(row_text(&compact, 3), "Esc/q/F1 close");
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
        assert_eq!(
            row_text(terminal.backend().buffer(), 7),
            "q quit  F1 help  / search  j/k  Space/b"
        );
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
    fn long_search_drafts_keep_the_latest_text_visible() {
        let state = RenderState {
            filename: "book.txt",
            path: "/tmp/book.txt",
            rows: RenderRowsView::empty(),
            progress: 12,
            current_line: Some(7),
            total_lines: Some(9),
            status: SearchStatus::Draft {
                draft: "abcdefghij",
                limit_hit: false,
            },
            activity: None,
            boundary: None,
        };

        assert_eq!(status_text(&state, 18).unwrap(), "12%  7/9  /…efghij");
    }

    #[test]
    fn search_draft_clipping_preserves_unicode_graphemes_and_limit_notice() {
        let state = RenderState {
            filename: "book.txt",
            path: "/tmp/book.txt",
            rows: RenderRowsView::empty(),
            progress: 12,
            current_line: Some(7),
            total_lines: Some(9),
            status: SearchStatus::Draft {
                draft: "prefixe\u{301}終",
                limit_hit: false,
            },
            activity: None,
            boundary: None,
        };
        assert_eq!(status_text(&state, 15).unwrap(), "12%  7/9  /…e\u{301}終");

        let state = RenderState {
            status: SearchStatus::Draft {
                draft: "abcdefghijklmnopqrstuvwxyz",
                limit_hit: true,
            },
            ..state
        };
        let status = status_text(&state, 48).unwrap();
        assert!(status.contains("/…qrstuvwxyz"), "{status:?}");
        assert!(status.ends_with(" — query limit: 4096 bytes"));
    }

    #[test]
    fn committed_searches_keep_the_query_prefix_visible() {
        let state = RenderState {
            filename: "book.txt",
            path: "/tmp/book.txt",
            rows: RenderRowsView::empty(),
            progress: 12,
            current_line: Some(7),
            total_lines: Some(9),
            status: SearchStatus::Committed {
                query: "abcdefghij",
                no_matches: false,
                searching: false,
            },
            activity: None,
            boundary: None,
        };

        assert_eq!(status_text(&state, 18).unwrap(), "12%  7/9  /abcdef…");
    }

    #[test]
    fn viewport_boundaries_replace_only_the_progress_slot() {
        let mut state = RenderState {
            filename: "book.txt",
            path: "/tmp/book.txt",
            rows: RenderRowsView::empty(),
            progress: 12,
            current_line: Some(7),
            total_lines: Some(9),
            status: SearchStatus::None,
            activity: None,
            boundary: None,
        };

        assert_eq!(status_text(&state, 40).unwrap(), "12%  7/9");
        for (boundary, expected) in [
            (ViewportBoundary::Top, "TOP  7/9"),
            (ViewportBoundary::End, "END  7/9"),
            (ViewportBoundary::All, "ALL  7/9"),
        ] {
            state.boundary = Some(boundary);
            assert_eq!(status_text(&state, 40).unwrap(), expected);
        }
    }

    #[test]
    fn status_notices_preempt_generic_activity_and_survive_narrow_frames() {
        let mut state = RenderState {
            filename: "book.txt",
            path: "/tmp/book.txt",
            rows: RenderRowsView::empty(),
            progress: 12,
            current_line: Some(7),
            total_lines: Some(9),
            status: SearchStatus::None,
            activity: Some(ViewActivity::PreparingView),
            boundary: Some(ViewportBoundary::Top),
        };

        assert_eq!(
            status_text(&state, 40).unwrap(),
            "TOP  7/9 — preparing view"
        );
        assert_eq!(status_text(&state, 16).unwrap(), "preparing view");

        state.activity = Some(ViewActivity::GoingToEnd);
        assert_eq!(status_text(&state, 16).unwrap(), "going to end");

        state.status = SearchStatus::Committed {
            query: "e\u{301}終needle",
            no_matches: false,
            searching: true,
        };
        state.activity = Some(ViewActivity::PreparingView);
        assert_eq!(status_text(&state, 16).unwrap(), "searching");

        state.status = SearchStatus::Committed {
            query: "needle",
            no_matches: true,
            searching: false,
        };
        assert_eq!(status_text(&state, 16).unwrap(), "no matches");

        let limit_draft = "x".repeat(4096);
        state.status = SearchStatus::Draft {
            draft: &limit_draft,
            limit_hit: true,
        };
        assert_eq!(status_text(&state, 16).unwrap(), "query limit");
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
            activity: None,
            boundary: None,
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
            activity: None,
            boundary: None,
        };
        assert_eq!(
            header_text(state.filename, state.path, 40).unwrap(),
            "standard input"
        );
    }

    #[test]
    fn tiny_frames_prioritize_recovery_and_exit_hints() {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), "body".to_owned());
        let buffer = draw(&mut app, 12, 3);
        assert_eq!(row_text(&buffer, 0), "resize q");
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
