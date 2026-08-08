use std::{iter::Peekable, ops::Range};

use unicode_segmentation::UnicodeSegmentation;

pub(super) use crate::layout::{ContentWidth, DisplayAtoms, DisplayColumn};
use crate::{
    document::{Document, DocumentCache, DocumentReader},
    error::TutError,
    layout::{
        BodyHeight, DOTTED_CIRCLE, DisplayProjection, GraphemeRange, ProjectedAtom,
        REPLACEMENT_CHARACTER, ViewportLayout, ensure_viewport_layout, progress_percent,
    },
    line_index::LinePosition,
    search::{IntersectingMatches, MatchIndex, SearchRange},
    source::SourceOffset,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderProjectionKind {
    Text,
    Spaces,
    Replacement,
    DottedCircle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderSpan {
    text: Range<usize>,
    pub projection: RenderProjectionKind,
    pub cell_width: DisplayColumn,
    pub highlight: Highlight,
}

impl RenderSpan {
    pub(super) fn from_projected(
        atom: ProjectedAtom<'_>,
        highlight: Highlight,
        output: &mut String,
    ) -> Result<Self, TutError> {
        let cell_width = atom.width();
        let start = output.len();
        let projection = match atom.projection() {
            DisplayProjection::Text(text) => {
                append_render_text(output, text)?;
                RenderProjectionKind::Text
            }
            DisplayProjection::Spaces(count) => {
                let text = match count {
                    1 => " ",
                    2 => "  ",
                    3 => "   ",
                    4 => "    ",
                    _ => unreachable!("tab expansion is one through four cells"),
                };
                append_render_text(output, text)?;
                RenderProjectionKind::Spaces
            }
            DisplayProjection::Replacement => {
                append_render_text(output, REPLACEMENT_CHARACTER)?;
                RenderProjectionKind::Replacement
            }
            DisplayProjection::DottedCircle(source) => {
                let additional = DOTTED_CIRCLE
                    .len()
                    .checked_add(source.len())
                    .ok_or(TutError::Allocation("zero-width render atom"))?;
                output
                    .try_reserve_exact(additional)
                    .map_err(|_| TutError::Allocation("zero-width render atom"))?;
                output.push_str(DOTTED_CIRCLE);
                output.push_str(source);
                RenderProjectionKind::DottedCircle
            }
        };

        Ok(Self {
            text: start..output.len(),
            projection,
            cell_width,
            highlight,
        })
    }

    pub(super) fn text<'a>(&self, row: &'a str) -> &'a str {
        row.get(self.text.clone())
            .expect("render spans retain valid row-text boundaries")
    }
}

fn append_render_text(output: &mut String, text: &str) -> Result<(), TutError> {
    output
        .try_reserve_exact(text.len())
        .map_err(|_| TutError::Allocation("visible row text"))?;
    output.push_str(text);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RenderRow {
    pub text: String,
    pub spans: Vec<RenderSpan>,
}

#[derive(Debug)]
pub(super) struct RenderState<'a> {
    pub filename: &'a str,
    pub path: &'a str,
    pub rows: Vec<RenderRow>,
    pub progress: u8,
    pub current_line: Option<u64>,
    pub total_lines: Option<u64>,
    pub status: SearchStatus<'a>,
}

pub(super) struct App {
    document: Document,
    document_cache: DocumentCache,
    layout: Option<ViewportLayout>,
    anchor: SourceOffset,
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
        let anchor = document.source_start();
        Self {
            document,
            document_cache: DocumentCache::default(),
            layout: None,
            anchor,
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

    pub(super) fn viewport(&mut self) -> Result<Option<Viewport>, TutError> {
        let Some(layout) = self.layout.as_ref() else {
            return Ok(None);
        };
        let mut reader = self.document.reader(&mut self.document_cache);
        let (visible_rows, visible_end) = layout.visible_extent(&mut reader, self.anchor)?;
        Ok(Some(Viewport {
            visible_rows,
            first_visible_start: self.anchor,
            visible_end,
        }))
    }

    #[cfg(test)]
    pub(super) fn progress_percent(&mut self) -> Result<u8, TutError> {
        let viewport = self.viewport()?;
        Ok(self.progress_for(viewport))
    }

    pub(super) fn render_state(&mut self) -> Result<RenderState<'_>, TutError> {
        let viewport = self.viewport()?;
        let line = self.line_position_for(viewport)?;
        let progress = self.progress_for(viewport);
        let rows = self.build_render_rows(viewport)?;
        Ok(RenderState {
            filename: self.document.display_name(),
            path: self.document.display_path(),
            rows,
            progress,
            current_line: line.map(LinePosition::current),
            total_lines: line.and_then(LinePosition::total),
            status: self.search_status(),
        })
    }

    fn line_position_for(
        &mut self,
        viewport: Option<Viewport>,
    ) -> Result<Option<LinePosition>, TutError> {
        let offset = viewport.map_or(self.document.source_start(), |viewport| {
            viewport.first_visible_start
        });
        let mut reader = self.document.reader(&mut self.document_cache);
        Ok(reader.line_position(offset)?)
    }

    pub(super) const fn has_background_work(&self) -> bool {
        !self.document.line_index_complete()
    }

    pub(super) fn advance_background(&mut self) -> Result<bool, TutError> {
        let covered = self.document.line_index_covers(self.anchor);
        let complete = self.document.line_index_complete();
        let advanced = self.document.advance_line_index(&mut self.document_cache)?;
        if !advanced {
            return Ok(false);
        }
        Ok((!covered && self.document.line_index_covers(self.anchor))
            || (!complete && self.document.line_index_complete()))
    }

    fn progress_for(&self, viewport: Option<Viewport>) -> u8 {
        match viewport {
            Some(viewport) => progress_percent(
                self.document.source_start(),
                self.document.source_end(),
                viewport.visible_end,
            ),
            None if self.document.source_start() == self.document.source_end() => 100,
            None => 0,
        }
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
            Action::LineDown if reading => self.move_rows(true, 1)?,
            Action::LineUp if reading => self.move_rows(false, 1)?,
            Action::PageDown if reading => self.move_rows(true, self.page_amount())?,
            Action::PageUp if reading => self.move_rows(false, self.page_amount())?,
            Action::HalfPageDown if reading => self.move_rows(true, self.half_page_amount())?,
            Action::HalfPageUp if reading => self.move_rows(false, self.half_page_amount())?,
            Action::DocumentStart if reading => self.document_start(),
            Action::DocumentEnd if reading => self.document_end()?,
            Action::BeginSearch if reading => self.begin_search(),
            Action::SearchInsert(character) if editing => self.insert_search(character),
            Action::SearchBackspace if editing => self.backspace_search(),
            Action::SearchCommit if editing => self.commit_search()?,
            Action::SearchCancel if editing => self.cancel_search(),
            Action::NextMatch if reading => self.select_match(true)?,
            Action::PreviousMatch if reading => self.select_match(false)?,
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
        let old_anchor = self.anchor;
        self.geometry = geometry;

        if !geometry.is_usable() {
            return Ok(geometry_changed);
        }

        let mut reader = self.document.reader(&mut self.document_cache);
        let rebuilt = ensure_viewport_layout(
            &mut self.layout,
            &reader,
            geometry.content_width(),
            geometry.body_height(),
        );
        let layout = self.layout.as_ref().expect("usable geometry has a layout");
        self.anchor = layout.resolve_top(&mut reader, self.anchor, self.follow_end)?;

        Ok(geometry_changed || rebuilt || old_anchor != self.anchor)
    }

    fn move_rows(&mut self, downward: bool, amount: usize) -> Result<bool, TutError> {
        let layout = self.layout.as_ref().expect("usable geometry");
        let old_anchor = self.anchor;
        let old_follow_end = self.follow_end;
        let mut reader = self.document.reader(&mut self.document_cache);
        let target = layout.move_row_start(&mut reader, self.anchor, downward, amount)?;
        let reached_end =
            downward && target != old_anchor && layout.is_last_viewport(&mut reader, target)?;

        self.anchor = target;
        self.follow_end = downward && (old_follow_end || reached_end);

        Ok(old_anchor != self.anchor || old_follow_end != self.follow_end)
    }

    fn document_start(&mut self) -> bool {
        let source_start = self.document.source_start();
        let changed = self.anchor != source_start || self.follow_end;
        self.anchor = source_start;
        self.follow_end = false;
        changed
    }

    fn document_end(&mut self) -> Result<bool, TutError> {
        let layout = self.layout.as_ref().expect("usable geometry");
        let mut reader = self.document.reader(&mut self.document_cache);
        let anchor = layout.last_viewport_start(&mut reader)?;
        let changed = self.anchor != anchor || !self.follow_end;
        self.anchor = anchor;
        self.follow_end = true;
        Ok(changed)
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
            .viewport()?
            .map_or(self.document.source_start(), |viewport| {
                viewport.first_visible_start
            });
        let mut reader = self.document.reader(&mut self.document_cache);
        let index =
            MatchIndex::build(&mut reader, &draft)?.expect("nonempty query creates an index");
        let selected = index.first_intersecting_or_wrap(first_visible);
        self.committed_query = draft;
        self.match_index = Some(index);
        self.current_match = selected;
        if let Some(selected) = selected {
            let _ = self.jump_to_match(selected)?;
        }
        Ok(true)
    }

    fn select_match(&mut self, forward: bool) -> Result<bool, TutError> {
        let (Some(index), Some(current)) = (self.match_index.as_ref(), self.current_match) else {
            return Ok(false);
        };
        let selected = if forward {
            index.next_after(current)
        } else {
            index.previous_before(current)
        };
        let Some(selected) = selected else {
            return Ok(false);
        };
        let changed = self.current_match != Some(selected);
        self.current_match = Some(selected);
        Ok(self.jump_to_match(selected)? || changed)
    }

    fn jump_to_match(&mut self, selected: SearchRange) -> Result<bool, TutError> {
        let body_height = self.geometry.body_height().expect("usable geometry");
        let layout = self.layout.as_ref().expect("usable geometry");
        let mut reader = self.document.reader(&mut self.document_cache);
        let match_row = layout.row_start_at_or_before(&mut reader, selected.start())?;
        let anchor = layout.move_row_start(
            &mut reader,
            match_row,
            false,
            usize::from(body_height.get() / 2),
        )?;
        let changed = self.anchor != anchor || self.follow_end;
        self.anchor = anchor;
        self.follow_end = false;
        Ok(changed)
    }

    fn build_render_rows(
        &mut self,
        viewport: Option<Viewport>,
    ) -> Result<Vec<RenderRow>, TutError> {
        let Some(viewport) = viewport else {
            return Ok(Vec::new());
        };
        let layout = self.layout.as_ref().expect("viewport has a layout");
        let visible = viewport.first_visible_start..viewport.visible_end;
        let mut matches =
            MatchCursor::for_viewport(self.match_index.as_ref(), self.current_match, visible);
        let mut rows = Vec::new();
        rows.try_reserve_exact(viewport.visible_rows)
            .map_err(|_| TutError::Allocation("visible rows"))?;

        let mut start = viewport.first_visible_start;
        let mut reader = self.document.reader(&mut self.document_cache);
        for row_number in 0..viewport.visible_rows {
            let (row, next) = build_render_row(layout, &mut reader, start, &mut matches)?;
            rows.push(row);
            if row_number + 1 < viewport.visible_rows {
                start = next.expect("non-final visible rows have successors");
            }
        }
        Ok(rows)
    }
}

fn build_render_row(
    layout: &ViewportLayout,
    reader: &mut DocumentReader<'_>,
    start: SourceOffset,
    matches: &mut MatchCursor<'_>,
) -> Result<(RenderRow, Option<SourceOffset>), TutError> {
    let mut text = String::new();
    let mut spans = Vec::new();
    let next = layout.visit_projected_row(reader, start, |atom| {
        let highlight = matches.role_for(atom.source());
        if spans.len() == spans.capacity() {
            spans
                .try_reserve(1)
                .map_err(|_| TutError::Allocation("visible row spans"))?;
        }
        let span = RenderSpan::from_projected(atom, highlight, &mut text)?;
        spans.push(span);
        Ok(())
    })?;
    Ok((RenderRow { text, spans }, next))
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
    use std::{fs, path::Path};

    use tempfile::tempdir;

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
        assert_eq!(
            app.viewport().unwrap().unwrap().first_visible_start,
            SourceOffset::ZERO
        );
        app.update(Action::DocumentEnd).unwrap();
        assert!(app.follow_end);
        assert_eq!(app.progress_percent().unwrap(), 100);
        app.update(Action::LineDown).unwrap();
        assert!(app.follow_end);
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        assert!(app.follow_end);
        assert_eq!(
            app.viewport().unwrap().unwrap().first_visible_start,
            SourceOffset::ZERO
        );
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
    fn render_state_owns_rows_and_marks_all_matches() {
        let mut app = reader("cat cat", 16, 4);
        commit(&mut app, "cat");
        let state = app.render_state().unwrap();
        assert_eq!(state.rows[0].spans[0].highlight, Highlight::Current);
        assert_eq!(state.rows[0].spans[4].highlight, Highlight::Match);
        assert_eq!(state.rows[0].spans[0].text(&state.rows[0].text), "c");
    }

    #[test]
    fn bom_and_raw_line_endings_keep_absolute_coordinates_end_to_end() {
        let mut app = reader("\u{feff}a\r\ncat\rend", 16, 6);

        assert_eq!(
            app.viewport().unwrap().unwrap().first_visible_start,
            SourceOffset::new(3)
        );
        {
            let layout = app.layout.as_ref().unwrap();
            let mut reader = app.document.reader(&mut app.document_cache);
            assert_eq!(
                layout
                    .next_row_start(&mut reader, SourceOffset::new(3))
                    .unwrap(),
                Some(SourceOffset::new(6))
            );
            assert_eq!(
                layout
                    .next_row_start(&mut reader, SourceOffset::new(6))
                    .unwrap(),
                Some(SourceOffset::new(10))
            );
        }
        let state = app.render_state().unwrap();
        assert_eq!((state.current_line, state.total_lines), (Some(1), Some(3)));

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
    fn file_backed_documents_render_and_search_without_a_contiguous_source() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("book.txt");
        fs::write(&path, "\u{feff}alpha\nneedle\nend").unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 6))).unwrap();

        commit(&mut app, "needle");
        assert_eq!(app.current_match.unwrap().start(), SourceOffset::new(9));
        let state = app.render_state().unwrap();
        assert!(state.rows.iter().any(|row| {
            row.spans
                .iter()
                .any(|span| span.highlight == Highlight::Current)
        }));
    }

    #[test]
    fn file_line_index_advances_in_bounded_background_steps() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("large-book.txt");
        let mut text = "line\n".repeat(30_000);
        text.push_str("end");
        fs::write(&path, text).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 6))).unwrap();

        let initial = app.render_state().unwrap();
        assert_eq!((initial.current_line, initial.total_lines), (Some(1), None));
        assert!(app.has_background_work());

        let mut advances = 0;
        let mut redraws = 0;
        while app.has_background_work() {
            redraws += usize::from(app.advance_background().unwrap());
            advances += 1;
        }

        assert_eq!(advances, 3);
        assert_eq!(redraws, 1);
        let complete = app.render_state().unwrap();
        assert_eq!(
            (complete.current_line, complete.total_lines),
            (Some(1), Some(30_001))
        );
    }

    #[test]
    fn zero_width_source_has_one_owned_visible_cell() {
        let mut app = reader("\u{200b}", 16, 4);
        let state = app.render_state().unwrap();
        let row = &state.rows[0];
        let span = &row.spans[0];
        assert_eq!(span.text(&row.text), "◌\u{200b}");
        assert_eq!(span.projection, RenderProjectionKind::DottedCircle);
        assert_eq!(span.cell_width, DisplayColumn::new(1));
    }

    #[test]
    fn search_near_document_end_clamps_to_the_last_full_viewport() {
        let mut app = reader("a\nb\nc\nd\nneedle", 16, 6);
        commit(&mut app, "needle");

        assert_eq!(app.anchor, {
            let layout = app.layout.as_ref().expect("usable layout");
            let mut reader = app.document.reader(&mut app.document_cache);
            layout.last_viewport_start(&mut reader).unwrap()
        });
        assert_eq!(app.progress_percent().unwrap(), 100);
    }
}
