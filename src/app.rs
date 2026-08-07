use std::{iter::Peekable, ops::Range};

use unicode_segmentation::UnicodeSegmentation;

pub(super) use crate::layout::{ContentWidth, DisplayAtoms, DisplayColumn};
use crate::{
    document::Document,
    error::TutError,
    layout::{
        BodyHeight, DOTTED_CIRCLE, DisplayProjection, GraphemeRange, ProjectedAtom,
        REPLACEMENT_CHARACTER, VisualRowIndex, WrapIndex, ensure_wrap_index, progress_percent,
    },
    line_index::LinePosition,
    search::{IntersectingMatches, MatchIndex, SearchRange},
    source::{SourceOffset, SourceText},
};

pub(super) const MIN_TERMINAL_COLUMNS: u16 = 16;
pub(super) const MIN_TERMINAL_ROWS: u16 = 4;
pub(super) const SEARCH_DRAFT_LIMIT_BYTES: usize = 4096;
const CHROME_ROWS: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Geometry {
    columns: u16,
    rows: u16,
}

impl Geometry {
    pub(super) const fn new(columns: u16, rows: u16) -> Self {
        Self { columns, rows }
    }

    pub(super) const fn is_usable(self) -> bool {
        self.columns >= MIN_TERMINAL_COLUMNS && self.rows >= MIN_TERMINAL_ROWS
    }

    pub(super) const fn content_width(self) -> Option<ContentWidth> {
        if self.is_usable() {
            ContentWidth::new(self.columns)
        } else {
            None
        }
    }

    pub(super) const fn body_height(self) -> Option<BodyHeight> {
        if self.is_usable() {
            BodyHeight::new(self.rows - CHROME_ROWS)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mode {
    Reading,
    SearchInput { draft: String, limit_hit: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Resize(Geometry),
    LineDown,
    LineUp,
    PageDown,
    PageUp,
    HalfPageDown,
    HalfPageUp,
    DocumentStart,
    DocumentEnd,
    BeginSearch,
    SearchInsert(char),
    SearchBackspace,
    SearchCommit,
    SearchCancel,
    NextMatch,
    PreviousMatch,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Unchanged,
    Changed,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Viewport {
    pub top: VisualRowIndex,
    pub visible_rows: usize,
    pub first_visible_start: SourceOffset,
    pub visible_end: SourceOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SearchStatus<'a> {
    None,
    Committed { query: &'a str, no_matches: bool },
    Draft { draft: &'a str, limit_hit: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Highlight {
    None,
    Match,
    Current,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RenderText<'a> {
    Borrowed(&'a str),
    OwnedZeroWidth(String),
}

impl RenderText<'_> {
    pub(super) fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(text) => text,
            Self::OwnedZeroWidth(text) => text,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderProjectionKind {
    Text,
    Spaces,
    Replacement,
    OwnedZeroWidth,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderSpan<'a> {
    pub text: RenderText<'a>,
    pub projection: RenderProjectionKind,
    pub cell_width: DisplayColumn,
    pub highlight: Highlight,
}

impl<'a> RenderSpan<'a> {
    pub(super) fn from_projected(
        atom: ProjectedAtom<'a>,
        highlight: Highlight,
    ) -> Result<Self, TutError> {
        let cell_width = atom.width();
        let (projection, text) = match atom.projection() {
            DisplayProjection::Text(text) => {
                (RenderProjectionKind::Text, RenderText::Borrowed(text))
            }
            DisplayProjection::Spaces(count) => {
                let text = match count {
                    1 => " ",
                    2 => "  ",
                    3 => "   ",
                    4 => "    ",
                    _ => unreachable!("tab expansion is one through four cells"),
                };
                (RenderProjectionKind::Spaces, RenderText::Borrowed(text))
            }
            DisplayProjection::Replacement => (
                RenderProjectionKind::Replacement,
                RenderText::Borrowed(REPLACEMENT_CHARACTER),
            ),
            DisplayProjection::DottedCircle(source) => {
                let capacity = DOTTED_CIRCLE
                    .len()
                    .checked_add(source.len())
                    .ok_or(TutError::Allocation("zero-width render atom"))?;
                let mut visible = String::new();
                visible
                    .try_reserve_exact(capacity)
                    .map_err(|_| TutError::Allocation("zero-width render atom"))?;
                visible.push_str(DOTTED_CIRCLE);
                visible.push_str(source);
                (
                    RenderProjectionKind::OwnedZeroWidth,
                    RenderText::OwnedZeroWidth(visible),
                )
            }
        };

        Ok(Self {
            text,
            projection,
            cell_width,
            highlight,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderRow<'a> {
    pub spans: Vec<RenderSpan<'a>>,
}

#[derive(Debug)]
pub(super) struct RenderState<'a> {
    pub filename: &'a str,
    pub path: &'a str,
    pub rows: Vec<RenderRow<'a>>,
    pub progress: u8,
    pub current_line: u64,
    pub total_lines: u64,
    pub status: SearchStatus<'a>,
}

pub(super) struct App {
    document: Document,
    wrap_index: Option<WrapIndex>,
    anchor: SourceOffset,
    top: VisualRowIndex,
    follow_end: bool,
    geometry: Geometry,
    mode: Mode,
    committed_query: String,
    match_index: Option<MatchIndex>,
    current_match: Option<SearchRange>,
}

#[cfg(test)]
pub(super) fn app_from_text(path: &std::path::Path, text: String) -> App {
    App::new(Document::from_text(path, text))
}

impl App {
    pub(super) fn new(document: Document) -> Self {
        let anchor = document.source().start();
        Self {
            document,
            wrap_index: None,
            anchor,
            top: VisualRowIndex::ZERO,
            follow_end: false,
            geometry: Geometry::new(0, 0),
            mode: Mode::Reading,
            committed_query: String::new(),
            match_index: None,
            current_match: None,
        }
    }

    pub(super) const fn terminal_too_small(&self) -> bool {
        !self.geometry.is_usable()
    }

    pub(super) fn mode(&self) -> &Mode {
        &self.mode
    }

    pub(super) fn search_status(&self) -> SearchStatus<'_> {
        match &self.mode {
            Mode::SearchInput { draft, limit_hit } => SearchStatus::Draft {
                draft,
                limit_hit: *limit_hit,
            },
            Mode::Reading if self.committed_query.is_empty() => SearchStatus::None,
            Mode::Reading => SearchStatus::Committed {
                query: &self.committed_query,
                no_matches: self.current_match.is_none(),
            },
        }
    }

    pub(super) fn viewport(&self) -> Option<Viewport> {
        let body_height = self.geometry.body_height()?;
        let index = self.wrap_index.as_ref()?;
        let remaining = index.row_count().saturating_sub(self.top.as_usize());
        let visible_rows = remaining.min(usize::from(body_height.get()));
        Some(Viewport {
            top: self.top,
            visible_rows,
            first_visible_start: index.row_start(self.top)?,
            visible_end: index.visible_end(self.top, body_height),
        })
    }

    pub(super) fn progress_percent(&self) -> u8 {
        let source = self.document.source();
        match self.viewport() {
            Some(viewport) => progress_percent(source, viewport.visible_end),
            None if source.start() == source.end() => 100,
            None => 0,
        }
    }

    pub(super) fn render_state(&self) -> Result<RenderState<'_>, TutError> {
        let line = self.line_position();
        Ok(RenderState {
            filename: self.document.display_name(),
            path: self.document.display_path(),
            rows: self.build_render_rows()?,
            progress: self.progress_percent(),
            current_line: line.current(),
            total_lines: line.total(),
            status: self.search_status(),
        })
    }

    fn line_position(&self) -> LinePosition {
        let offset = self
            .viewport()
            .map_or(self.document.source().start(), |viewport| {
                viewport.first_visible_start
            });
        self.document
            .line_position(offset)
            .expect("viewport anchors are valid document boundaries")
    }

    pub(super) fn update(&mut self, action: Action) -> Result<Outcome, TutError> {
        if self.terminal_too_small() && !matches!(action, Action::Resize(_) | Action::Quit) {
            return Ok(Outcome::Unchanged);
        }
        if action == Action::Quit {
            return Ok(Outcome::Quit);
        }

        let reading = matches!(self.mode, Mode::Reading);
        let editing = matches!(self.mode, Mode::SearchInput { .. });
        let changed = match action {
            Action::Resize(geometry) => self.resize(geometry)?,
            Action::LineDown if reading => self.move_rows(true, 1),
            Action::LineUp if reading => self.move_rows(false, 1),
            Action::PageDown if reading => self.move_rows(true, self.page_amount()),
            Action::PageUp if reading => self.move_rows(false, self.page_amount()),
            Action::HalfPageDown if reading => self.move_rows(true, self.half_page_amount()),
            Action::HalfPageUp if reading => self.move_rows(false, self.half_page_amount()),
            Action::DocumentStart if reading => self.document_start(),
            Action::DocumentEnd if reading => self.document_end(),
            Action::BeginSearch if reading => self.begin_search(),
            Action::SearchInsert(character) if editing => self.insert_search(character),
            Action::SearchBackspace if editing => self.backspace_search(),
            Action::SearchCommit if editing => self.commit_search()?,
            Action::SearchCancel if editing => self.cancel_search(),
            Action::NextMatch if reading => self.select_match(true),
            Action::PreviousMatch if reading => self.select_match(false),
            _ => false,
        };

        Ok(if changed {
            Outcome::Changed
        } else {
            Outcome::Unchanged
        })
    }

    fn page_amount(&self) -> usize {
        usize::from(
            self.geometry
                .body_height()
                .expect("usable geometry")
                .get()
                .saturating_sub(1)
                .max(1),
        )
    }

    fn half_page_amount(&self) -> usize {
        usize::from((self.geometry.body_height().expect("usable geometry").get() / 2).max(1))
    }

    fn resize(&mut self, geometry: Geometry) -> Result<bool, TutError> {
        let geometry_changed = self.geometry != geometry;
        let old_top = self.top;
        self.geometry = geometry;

        if !geometry.is_usable() {
            return Ok(geometry_changed);
        }

        let rebuilt = ensure_wrap_index(
            &mut self.wrap_index,
            self.document.source(),
            geometry.content_width(),
        )?;
        let index = self
            .wrap_index
            .as_ref()
            .expect("usable geometry has an index");
        self.top = index.resolve_top(
            self.anchor,
            self.follow_end,
            geometry.body_height().expect("usable geometry"),
        );

        Ok(geometry_changed || rebuilt || old_top != self.top)
    }

    fn move_rows(&mut self, downward: bool, amount: usize) -> bool {
        let body_height = self.geometry.body_height().expect("usable geometry");
        let index = self.wrap_index.as_ref().expect("usable geometry");
        let max_top = index.max_top(body_height);
        let old_top = self.top;
        let old_anchor = self.anchor;
        let old_follow_end = self.follow_end;
        let amount = u32::try_from(amount).unwrap_or(u32::MAX);
        let target = if downward {
            VisualRowIndex::new(old_top.get().saturating_add(amount).min(max_top.get()))
        } else {
            VisualRowIndex::new(old_top.get().saturating_sub(amount))
        };

        self.anchor = index.row_start(target).expect("clamped row exists");
        self.top = target;
        if downward {
            if target != old_top && target == max_top {
                self.follow_end = true;
            }
        } else {
            self.follow_end = false;
        }

        old_top != self.top || old_anchor != self.anchor || old_follow_end != self.follow_end
    }

    fn document_start(&mut self) -> bool {
        let source_start = self.document.source().start();
        let changed =
            self.top != VisualRowIndex::ZERO || self.anchor != source_start || self.follow_end;
        self.top = VisualRowIndex::ZERO;
        self.anchor = source_start;
        self.follow_end = false;
        changed
    }

    fn document_end(&mut self) -> bool {
        let body_height = self.geometry.body_height().expect("usable geometry");
        let index = self.wrap_index.as_ref().expect("usable geometry");
        let top = index.max_top(body_height);
        let anchor = index.row_start(top).expect("max top exists");
        let changed = self.top != top || self.anchor != anchor || !self.follow_end;
        self.top = top;
        self.anchor = anchor;
        self.follow_end = true;
        changed
    }

    fn begin_search(&mut self) -> bool {
        self.mode = Mode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        };
        true
    }

    fn insert_search(&mut self, character: char) -> bool {
        let Mode::SearchInput { draft, limit_hit } = &mut self.mode else {
            return false;
        };
        if draft.len() + character.len_utf8() > SEARCH_DRAFT_LIMIT_BYTES {
            let changed = !*limit_hit;
            *limit_hit = true;
            return changed;
        }
        draft.push(character);
        *limit_hit = false;
        true
    }

    fn backspace_search(&mut self) -> bool {
        let Mode::SearchInput { draft, limit_hit } = &mut self.mode else {
            return false;
        };
        let old_limit = *limit_hit;
        *limit_hit = false;
        let Some((start, _)) = draft.grapheme_indices(true).next_back() else {
            return old_limit;
        };
        draft.truncate(start);
        true
    }

    fn cancel_search(&mut self) -> bool {
        self.mode = Mode::Reading;
        true
    }

    fn commit_search(&mut self) -> Result<bool, TutError> {
        let Mode::SearchInput { draft, .. } = std::mem::replace(&mut self.mode, Mode::Reading)
        else {
            return Ok(false);
        };

        if draft.is_empty() {
            self.committed_query.clear();
            self.match_index = None;
            self.current_match = None;
            return Ok(true);
        }

        let first_visible = self
            .viewport()
            .map_or(self.document.source().start(), |viewport| {
                viewport.first_visible_start
            });
        let index = MatchIndex::build(self.document.source(), &draft)?
            .expect("nonempty query creates an index");
        let selected = index.first_intersecting_or_wrap(first_visible);
        self.committed_query = draft;
        self.match_index = Some(index);
        self.current_match = selected;
        if let Some(selected) = selected {
            self.jump_to_match(selected);
        }
        Ok(true)
    }

    fn select_match(&mut self, forward: bool) -> bool {
        let (Some(index), Some(current)) = (self.match_index.as_ref(), self.current_match) else {
            return false;
        };
        let selected = if forward {
            index.next_after(current)
        } else {
            index.previous_before(current)
        };
        let Some(selected) = selected else {
            return false;
        };
        let changed = self.current_match != Some(selected);
        self.current_match = Some(selected);
        self.jump_to_match(selected) || changed
    }

    fn jump_to_match(&mut self, selected: SearchRange) -> bool {
        let body_height = self.geometry.body_height().expect("usable geometry");
        let index = self.wrap_index.as_ref().expect("usable geometry");
        let match_row = index.row_at_or_before(selected.start());
        let intended = VisualRowIndex::new(
            match_row
                .get()
                .saturating_sub(u32::from(body_height.get() / 2)),
        );
        let actual = intended.min(index.max_top(body_height));
        let anchor = index.row_start(intended).expect("intended row exists");
        let changed = self.top != actual || self.anchor != anchor || self.follow_end;
        self.top = actual;
        self.anchor = anchor;
        self.follow_end = false;
        changed
    }

    fn build_render_rows(&self) -> Result<Vec<RenderRow<'_>>, TutError> {
        let Some(viewport) = self.viewport() else {
            return Ok(Vec::new());
        };
        let wrap = self.wrap_index.as_ref().expect("viewport has an index");
        let visible = viewport.first_visible_start..viewport.visible_end;
        let mut matches =
            MatchCursor::for_viewport(self.match_index.as_ref(), self.current_match, visible);
        let mut rows = Vec::new();
        rows.try_reserve_exact(viewport.visible_rows)
            .map_err(|_| TutError::Allocation("visible rows"))?;

        for row_number in viewport.top.as_usize()..viewport.top.as_usize() + viewport.visible_rows {
            let spans = build_render_row(
                wrap,
                self.document.source(),
                VisualRowIndex::new(u32::try_from(row_number).expect("source bounds row indices")),
                &mut matches,
            )?;
            rows.push(RenderRow { spans });
        }
        Ok(rows)
    }
}

fn build_render_row<'a>(
    wrap: &WrapIndex,
    source: SourceText<'a>,
    row: VisualRowIndex,
    matches: &mut MatchCursor<'_>,
) -> Result<Vec<RenderSpan<'a>>, TutError> {
    let mut spans = Vec::new();
    for atom in wrap
        .projected_row(source, row)
        .expect("indexed row matches immutable document")
    {
        let highlight = matches.role_for(atom.source());
        if spans.len() == spans.capacity() {
            spans
                .try_reserve(1)
                .map_err(|_| TutError::Allocation("visible row spans"))?;
        }
        spans.push(RenderSpan::from_projected(atom, highlight)?);
    }
    Ok(spans)
}

struct MatchCursor<'a> {
    pending: Option<Peekable<IntersectingMatches<'a>>>,
    spanning: Option<SearchRange>,
    current: Option<SearchRange>,
}

impl<'a> MatchCursor<'a> {
    fn for_viewport(
        match_index: Option<&'a MatchIndex>,
        current: Option<SearchRange>,
        visible: Range<SourceOffset>,
    ) -> Self {
        Self {
            pending: match_index.map(|index| index.intersecting(visible).peekable()),
            spanning: None,
            current,
        }
    }

    fn role_for(&mut self, atom: GraphemeRange) -> Highlight {
        let mut role = Highlight::None;
        if let Some(active) = self.spanning.take() {
            if intersects(active, atom) {
                role = promote(role, active, self.current);
            }
            if active.end() > atom.end() {
                self.spanning = Some(active);
                return role;
            }
        }

        let Some(pending) = &mut self.pending else {
            return role;
        };
        while pending
            .peek()
            .is_some_and(|range| range.start() < atom.end())
        {
            let range = pending.next().expect("peeked match exists");
            if intersects(range, atom) {
                role = promote(role, range, self.current);
            }
            if range.end() > atom.end() {
                self.spanning = Some(range);
                break;
            }
        }
        role
    }
}

fn intersects(search: SearchRange, atom: GraphemeRange) -> bool {
    search.start() < atom.end() && atom.start() < search.end()
}

fn promote(current_role: Highlight, range: SearchRange, current: Option<SearchRange>) -> Highlight {
    if current == Some(range) {
        Highlight::Current
    } else if current_role == Highlight::Current {
        current_role
    } else {
        Highlight::Match
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn reader(text: &str, columns: u16, rows: u16) -> App {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), text.to_owned());
        app.update(Action::Resize(Geometry::new(columns, rows)))
            .unwrap();
        app
    }

    fn commit(app: &mut App, query: &str) {
        app.update(Action::BeginSearch).unwrap();
        for character in query.chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }
        app.update(Action::SearchCommit).unwrap();
    }

    #[test]
    fn navigation_clamps_and_preserves_end_following_across_reflow() {
        let mut app = reader("0123456789abcdef", 16, 4);
        assert_eq!(app.viewport().unwrap().top, VisualRowIndex::ZERO);
        app.update(Action::DocumentEnd).unwrap();
        assert!(app.follow_end);
        assert_eq!(app.progress_percent(), 100);
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        assert!(app.follow_end);
        assert_eq!(app.viewport().unwrap().top, VisualRowIndex::ZERO);
        app.update(Action::LineUp).unwrap();
        assert!(!app.follow_end);
    }

    #[test]
    fn tiny_geometry_freezes_state_except_for_quit_and_resize() {
        let mut app = reader("line", 10, 3);
        assert!(app.terminal_too_small());
        assert_eq!(app.update(Action::BeginSearch).unwrap(), Outcome::Unchanged);
        assert!(matches!(app.mode(), Mode::Reading));
        assert_eq!(app.update(Action::Quit).unwrap(), Outcome::Quit);
    }

    #[test]
    fn search_editing_is_transactional_and_grapheme_aware() {
        let mut app = reader("alpha beta alpha", 16, 5);
        commit(&mut app, "alpha");
        let first = app.current_match.unwrap();
        app.update(Action::NextMatch).unwrap();
        assert_ne!(app.current_match, Some(first));
        app.update(Action::PreviousMatch).unwrap();
        assert_eq!(app.current_match, Some(first));

        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('e')).unwrap();
        app.update(Action::SearchInsert('\u{301}')).unwrap();
        app.update(Action::SearchBackspace).unwrap();
        assert!(matches!(
            app.search_status(),
            SearchStatus::Draft { draft: "", .. }
        ));
        app.update(Action::SearchCancel).unwrap();
        assert_eq!(app.committed_query, "alpha");
    }

    #[test]
    fn search_draft_enforces_the_utf8_byte_limit() {
        let mut app = reader("body", 16, 4);
        app.update(Action::BeginSearch).unwrap();
        for _ in 0..SEARCH_DRAFT_LIMIT_BYTES {
            app.update(Action::SearchInsert('q')).unwrap();
        }
        app.update(Action::SearchInsert('x')).unwrap();
        assert!(matches!(
            app.search_status(),
            SearchStatus::Draft {
                draft,
                limit_hit: true
            } if draft.len() == SEARCH_DRAFT_LIMIT_BYTES
        ));
    }

    #[test]
    fn render_state_borrows_text_and_marks_all_matches() {
        let mut app = reader("cat cat", 16, 4);
        commit(&mut app, "cat");
        let state = app.render_state().unwrap();
        assert_eq!(state.rows[0].spans[0].highlight, Highlight::Current);
        assert_eq!(state.rows[0].spans[4].highlight, Highlight::Match);
        assert!(matches!(
            state.rows[0].spans[0].text,
            RenderText::Borrowed("c")
        ));
    }

    #[test]
    fn bom_and_raw_line_endings_keep_absolute_coordinates_end_to_end() {
        let mut app = reader("\u{feff}a\r\ncat\rend", 16, 6);
        let index = app.wrap_index.as_ref().unwrap();

        assert_eq!(
            app.viewport().unwrap().first_visible_start,
            SourceOffset::new(3)
        );
        assert_eq!(
            index.row_start(VisualRowIndex::new(1)),
            Some(SourceOffset::new(6))
        );
        assert_eq!(
            index.row_start(VisualRowIndex::new(2)),
            Some(SourceOffset::new(10))
        );
        let state = app.render_state().unwrap();
        assert_eq!((state.current_line, state.total_lines), (1, 3));

        commit(&mut app, "cat");
        assert_eq!(app.current_match.unwrap().start(), SourceOffset::new(6));
        assert_eq!(
            app.render_state().unwrap().rows[1].spans[0].highlight,
            Highlight::Current
        );

        app.update(Action::DocumentEnd).unwrap();
        app.update(Action::DocumentStart).unwrap();
        assert_eq!(app.anchor, SourceOffset::new(3));
    }

    #[test]
    fn zero_width_source_has_one_owned_visible_cell() {
        let app = reader("\u{200b}", 16, 4);
        let state = app.render_state().unwrap();
        let span = &state.rows[0].spans[0];
        assert_eq!(span.text.as_str(), "◌\u{200b}");
        assert_eq!(span.projection, RenderProjectionKind::OwnedZeroWidth);
        assert_eq!(span.cell_width, DisplayColumn::new(1));
    }
}
