use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    document::{Document, DocumentCache},
    error::TutError,
    layout::{
        BodyHeight, ContentWidth, DOTTED_CIRCLE, DisplayColumn, DisplayProjection, GraphemeRange,
        ProjectedAtom, ProjectedRowSink, REPLACEMENT_CHARACTER, ViewportLayout,
        ensure_viewport_layout, progress_percent,
    },
    line_index::LinePosition,
    locator::{LocatedViewport, RowDelta, RowNeighborhood, ViewportLocator},
    search::{
        MAX_SEARCH_QUERY_BYTES, MatchBlock, SearchHighlights, SearchIndex, SearchNavigation,
        SearchRange,
    },
    source::SourceOffset,
};

#[cfg(test)]
use crate::document::SOURCE_WINDOW_BYTES;

pub(super) const MIN_TERMINAL_COLUMNS: u16 = 16;
pub(super) const MIN_TERMINAL_ROWS: u16 = 4;
pub(super) const SEARCH_DRAFT_LIMIT_BYTES: usize = MAX_SEARCH_QUERY_BYTES;
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
    Committed {
        query: &'a str,
        no_matches: bool,
        searching: bool,
    },
    Draft {
        draft: &'a str,
        limit_hit: bool,
    },
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
    source: GraphemeRange,
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
                    .try_reserve(additional)
                    .map_err(|_| TutError::Allocation("zero-width render atom"))?;
                output.push_str(DOTTED_CIRCLE);
                output.push_str(source);
                RenderProjectionKind::DottedCircle
            }
        };

        Ok(Self {
            text: start..output.len(),
            source: atom.source(),
            projection,
            cell_width,
            highlight,
        })
    }

    pub(super) fn text<'a>(&self, row: &'a str) -> &'a str {
        row.get(self.text.clone())
            .expect("render spans retain valid row-text boundaries")
    }

    fn merge(&mut self, next: &Self) -> bool {
        if self.projection != next.projection
            || self.highlight != next.highlight
            || self.text.end != next.text.start
            || self.source.end() != next.source.start()
        {
            return false;
        }
        let Some(width) = self.cell_width.get().checked_add(next.cell_width.get()) else {
            return false;
        };
        self.text.end = next.text.end;
        self.source = GraphemeRange::new(self.source.start(), next.source.end())
            .expect("adjacent render spans retain a nonempty source range");
        self.cell_width = DisplayColumn::new(width);
        true
    }
}

fn append_render_text(output: &mut String, text: &str) -> Result<(), TutError> {
    output
        .try_reserve(text.len())
        .map_err(|_| TutError::Allocation("visible row text"))?;
    output.push_str(text);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderRowRange {
    text: Range<usize>,
    spans: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct RenderRows {
    text: String,
    spans: Vec<RenderSpan>,
    rows: Vec<RenderRowRange>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RenderRow<'a> {
    pub text: &'a str,
    pub spans: &'a [RenderSpan],
}

impl RenderRows {
    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = RenderRow<'_>> + '_ {
        self.rows.iter().map(|row| RenderRow {
            text: self
                .text
                .get(row.text.clone())
                .expect("render rows retain valid text boundaries"),
            spans: self
                .spans
                .get(row.spans.clone())
                .expect("render rows retain valid span boundaries"),
        })
    }

    #[cfg(test)]
    pub(super) fn get(&self, index: usize) -> Option<RenderRow<'_>> {
        let row = self.rows.get(index)?;
        Some(RenderRow {
            text: self.text.get(row.text.clone())?,
            spans: self.spans.get(row.spans.clone())?,
        })
    }

    pub(super) const fn len(&self) -> usize {
        self.rows.len()
    }

    fn try_clone(&self) -> Result<Self, TutError> {
        let mut text = String::new();
        text.try_reserve_exact(self.text.len())
            .map_err(|_| TutError::Allocation("cached visible row text"))?;
        text.push_str(&self.text);

        let mut spans = Vec::new();
        spans
            .try_reserve_exact(self.spans.len())
            .map_err(|_| TutError::Allocation("cached visible row spans"))?;
        spans.extend_from_slice(&self.spans);

        let mut rows = Vec::new();
        rows.try_reserve_exact(self.rows.len())
            .map_err(|_| TutError::Allocation("cached visible rows"))?;
        rows.extend_from_slice(&self.rows);
        Ok(Self { text, spans, rows })
    }

    fn apply_highlights(&mut self, ranges: &[SearchRange], current: Option<SearchRange>) {
        let mut matches = MatchCursor::new(ranges, current);
        let mut write = 0;
        for row in &mut self.rows {
            let source = row.spans.clone();
            let row_start = write;
            for read in source {
                let mut span = self.spans[read].clone();
                span.highlight = matches.role_for(span.source);
                if write > row_start && self.spans[write - 1].merge(&span) {
                    continue;
                }
                self.spans[write] = span;
                write += 1;
            }
            row.spans = row_start..write;
        }
        self.spans.truncate(write);
    }
}

#[derive(Debug)]
pub(super) struct RenderState<'a> {
    pub filename: &'a str,
    pub path: &'a str,
    pub rows: RenderRows,
    pub progress: u8,
    pub current_line: Option<u64>,
    pub total_lines: Option<u64>,
    pub status: SearchStatus<'a>,
}

struct RenderViewportCache {
    geometry: Geometry,
    anchor: SourceOffset,
    viewport: Viewport,
    rows: RenderRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowEndPolicy {
    Never,
    AtEnd,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewportRequest {
    Reflow {
        target: SourceOffset,
    },
    Move {
        target: SourceOffset,
        delta: RowDelta,
        follow_end: FollowEndPolicy,
    },
    Search {
        target: SourceOffset,
    },
    End,
}

impl ViewportRequest {
    const fn target(self, source_end: SourceOffset) -> SourceOffset {
        match self {
            Self::Reflow { target } | Self::Move { target, .. } | Self::Search { target } => target,
            Self::End => source_end,
        }
    }

    fn locator_parameters(
        self,
        source_end: SourceOffset,
        height: BodyHeight,
    ) -> (SourceOffset, RowDelta) {
        let target = self.target(source_end);
        let delta = match self {
            Self::Move { delta, .. } => delta,
            Self::Search { .. } => RowDelta::Backward(usize::from(height.get() / 2)),
            Self::Reflow { .. } | Self::End => RowDelta::Forward(0),
        };
        (target, delta)
    }

    const fn follows_end(self, at_end: bool) -> bool {
        match self {
            Self::End => true,
            Self::Move { follow_end, .. } => match follow_end {
                FollowEndPolicy::Never => false,
                FollowEndPolicy::AtEnd => at_end,
                FollowEndPolicy::Always => true,
            },
            Self::Reflow { .. } | Self::Search { .. } => false,
        }
    }

    const fn is_move(self) -> bool {
        matches!(self, Self::Move { .. })
    }
}

pub(super) struct App {
    document: Document,
    document_cache: DocumentCache,
    search_cache: DocumentCache,
    layout: Option<ViewportLayout>,
    anchor: SourceOffset,
    anchor_is_row_start: bool,
    follow_end: bool,
    geometry: Geometry,
    mode: Mode,
    committed_query: String,
    search_index: Option<SearchIndex>,
    current_match: Option<SearchRange>,
    navigation: Option<SearchNavigation>,
    match_block: Option<MatchBlock>,
    pending_navigation: i64,
    highlights: Option<SearchHighlights>,
    viewport_request: Option<ViewportRequest>,
    locator: Option<ViewportLocator>,
    row_neighborhood: RowNeighborhood,
    render_cache: Option<RenderViewportCache>,
    queued_rows: i64,
    search_jump_pending: bool,
    search_turn: bool,
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
            search_cache: DocumentCache::default(),
            layout: None,
            anchor,
            anchor_is_row_start: true,
            follow_end: false,
            geometry: Geometry::new(0, 0),
            mode: Mode::Reading,
            committed_query: String::new(),
            search_index: None,
            current_match: None,
            navigation: None,
            match_block: None,
            pending_navigation: 0,
            highlights: None,
            viewport_request: None,
            locator: None,
            row_neighborhood: RowNeighborhood::default(),
            render_cache: None,
            queued_rows: 0,
            search_jump_pending: false,
            search_turn: true,
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
                no_matches: self
                    .search_index
                    .as_ref()
                    .is_some_and(|index| index.is_complete() && !index.has_matches()),
                searching: self
                    .search_index
                    .as_ref()
                    .is_some_and(|index| !index.is_complete())
                    || self.navigation.is_some()
                    || self.pending_navigation != 0
                    || matches!(self.viewport_request, Some(ViewportRequest::Search { .. })),
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
        let (viewport, mut rows) = self.build_render_viewport()?;
        self.prepare_search_highlights(viewport, &rows)?;
        let line = self.line_position_for(viewport)?;
        let progress = self.progress_for(viewport);
        if let Some(viewport) = viewport {
            let visible = viewport.first_visible_start..viewport.visible_end;
            let ranges = self
                .highlights
                .as_ref()
                .filter(|highlights| highlights.covers(&visible))
                .map_or(&[][..], SearchHighlights::ranges);
            rows.apply_highlights(ranges, self.current_match);
        }
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

    fn prepare_search_highlights(
        &mut self,
        viewport: Option<Viewport>,
        rows: &RenderRows,
    ) -> Result<(), TutError> {
        let Some(viewport) = viewport else {
            self.highlights = None;
            return Ok(());
        };
        let visible = viewport.first_visible_start..viewport.visible_end;
        if !matches!(self.mode, Mode::Reading) {
            self.highlights = None;
            return Ok(());
        }
        if self
            .highlights
            .as_ref()
            .is_some_and(|highlights| highlights.covers(&visible))
        {
            return Ok(());
        }
        self.highlights = None;
        let Some(index) = self
            .search_index
            .as_ref()
            .filter(|index| index.is_complete() && index.has_matches())
        else {
            return Ok(());
        };
        let targets = rows.spans.iter().map(|span| {
            SearchRange::new(span.source.start(), span.source.end())
                .expect("render spans retain nonempty source ranges")
        });
        self.highlights = index.display_highlights(visible, targets)?;
        Ok(())
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

    pub(super) fn has_background_work(&self) -> bool {
        !self.document.line_index_complete()
            || (matches!(self.mode, Mode::Reading)
                && self.geometry.is_usable()
                && self.viewport_request.is_some())
            || (matches!(self.mode, Mode::Reading)
                && self.geometry.is_usable()
                && (self
                    .search_index
                    .as_ref()
                    .is_some_and(|index| !index.is_complete())
                    || self.navigation.is_some()
                    || self.pending_navigation != 0
                    || self
                        .highlights
                        .as_ref()
                        .is_some_and(|highlights| !highlights.is_complete())))
    }

    pub(super) fn advance_background(&mut self) -> Result<bool, TutError> {
        let viewport_pending = matches!(self.mode, Mode::Reading)
            && self.geometry.is_usable()
            && self.viewport_request.is_some();
        if viewport_pending {
            let request = self
                .viewport_request
                .expect("viewport work retains its request");
            let index_ready = match request {
                ViewportRequest::End => self.document.line_index_complete(),
                _ => self
                    .document
                    .line_index_covers(request.target(self.document.source_end())),
            };
            if !index_ready {
                return self.advance_line_index();
            }
            return self.advance_viewport_locator();
        }
        let search_pending = matches!(self.mode, Mode::Reading)
            && self.geometry.is_usable()
            && (self
                .search_index
                .as_ref()
                .is_some_and(|index| !index.is_complete())
                || self.navigation.is_some()
                || self.pending_navigation != 0
                || self
                    .highlights
                    .as_ref()
                    .is_some_and(|highlights| !highlights.is_complete()));
        let line_pending = !self.document.line_index_complete();
        if search_pending && (!line_pending || self.search_turn) {
            self.search_turn = false;
            return self.advance_search();
        }
        if line_pending {
            self.search_turn = true;
            return self.advance_line_index();
        }
        Ok(false)
    }

    fn advance_viewport_locator(&mut self) -> Result<bool, TutError> {
        let request = self
            .viewport_request
            .expect("viewport work retains its request");
        if self.locator.is_none() {
            let height = self.geometry.body_height().expect("usable geometry");
            let (target, delta) = request.locator_parameters(self.document.source_end(), height);
            let row_key = self
                .layout
                .as_ref()
                .expect("usable geometry has a layout")
                .row_cache_key();
            if request.is_move()
                && self.anchor_is_row_start
                && let Some(located) = self.row_neighborhood.locate_move(
                    row_key,
                    self.document.source_start(),
                    target,
                    delta,
                    height,
                )
            {
                self.document.validate()?;
                return Ok(self.finish_viewport_request(request, located));
            }
            self.locator = Some(if request.is_move() && self.anchor_is_row_start {
                ViewportLocator::from_row_start(target, delta, height)?
            } else {
                ViewportLocator::new(target, delta, height)?
            });
        }

        let layout = self.layout.as_ref().expect("usable geometry has a layout");
        let mut reader = self.document.reader(&mut self.document_cache);
        let located = self
            .locator
            .as_mut()
            .expect("viewport locator was initialized")
            .advance(layout, &mut reader, &mut self.row_neighborhood)?;
        let Some(located) = located else {
            return Ok(false);
        };

        Ok(self.finish_viewport_request(request, located))
    }

    fn finish_viewport_request(
        &mut self,
        request: ViewportRequest,
        located: LocatedViewport,
    ) -> bool {
        let old_anchor = self.anchor;
        let old_follow_end = self.follow_end;
        self.viewport_request = None;
        self.locator = None;
        self.anchor = located.anchor;
        self.anchor_is_row_start = true;
        self.follow_end = request.follows_end(located.at_end);
        if request.is_move() {
            self.start_queued_move();
        }
        old_anchor != self.anchor || old_follow_end != self.follow_end
    }

    fn advance_line_index(&mut self) -> Result<bool, TutError> {
        let covered = self.document.line_index_covers(self.anchor);
        let complete = self.document.line_index_complete();
        let advanced = self.document.advance_line_index(&mut self.document_cache)?;
        if !advanced {
            return Ok(false);
        }
        Ok((!covered && self.document.line_index_covers(self.anchor))
            || (!complete && self.document.line_index_complete()))
    }

    fn advance_search(&mut self) -> Result<bool, TutError> {
        let Some(index) = self.search_index.as_mut() else {
            return Ok(false);
        };
        let initial_scan = !index.is_complete();
        let selection_should_jump;
        let advance = if initial_scan {
            selection_should_jump = self.search_jump_pending;
            let mut reader = self.document.reader(&mut self.search_cache);
            index.advance(&mut reader, &self.committed_query)?
        } else {
            if self.navigation.is_none() {
                if self.pending_navigation == 0 {
                    let Some(highlights) = self.highlights.as_mut() else {
                        return Ok(false);
                    };
                    let mut reader = self.document.reader(&mut self.search_cache);
                    return highlights.advance(&mut reader, &self.committed_query);
                }
                let Some(current) = self.current_match else {
                    let changed = self.pending_navigation != 0;
                    self.pending_navigation = 0;
                    self.search_jump_pending = false;
                    return Ok(changed);
                };
                let forward = self.pending_navigation > 0;
                self.pending_navigation -= if forward { 1 } else { -1 };
                self.navigation =
                    index.navigation_with_block(current, forward, self.match_block.take());
            }
            let Some(navigation) = self.navigation.as_mut() else {
                let changed = self.pending_navigation != 0;
                self.pending_navigation = 0;
                return Ok(changed);
            };
            selection_should_jump = true;
            let mut reader = self.document.reader(&mut self.search_cache);
            navigation.advance(&mut reader, &self.committed_query)?
        };
        let mut changed = advance.completed();
        if initial_scan && (advance.selected().is_some() || advance.completed()) {
            self.search_jump_pending = false;
        }
        if advance.completed() && self.navigation.is_some() {
            self.match_block = self
                .navigation
                .as_mut()
                .and_then(SearchNavigation::take_block);
            self.navigation = None;
        }
        if let Some(selected) = advance.selected() {
            changed |= self.current_match != Some(selected);
            self.current_match = Some(selected);
            if selection_should_jump && self.viewport_request.is_none() && !self.follow_end {
                changed |= self.schedule_search_jump(selected);
            }
        }
        Ok(changed)
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
            Action::Resize(geometry) => self.resize(geometry),
            Action::LineDown if reading => self.move_rows(true, 1),
            Action::LineUp if reading => self.move_rows(false, 1),
            Action::PageDown if reading => self.move_rows(true, self.page_amount()),
            Action::PageUp if reading => self.move_rows(false, self.page_amount()),
            Action::HalfPageDown if reading => self.move_rows(true, self.half_page_amount()),
            Action::HalfPageUp if reading => self.move_rows(false, self.half_page_amount()),
            Action::DocumentStart if reading => self.document_start(),
            Action::DocumentEnd if reading => self.document_end(),
            Action::BeginSearch if reading => self.begin_search(),
            Action::SearchInsert(character) if editing => self.insert_search(character)?,
            Action::SearchBackspace if editing => self.backspace_search(),
            Action::SearchCommit if editing => self.commit_search()?,
            Action::SearchCancel if editing => self.cancel_search(),
            Action::SearchCancel if reading => self.cancel_committed_search(),
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

    fn resize(&mut self, geometry: Geometry) -> bool {
        let geometry_changed = self.geometry != geometry;
        if !geometry_changed {
            return false;
        }
        self.highlights = None;
        if self.geometry.content_width() != geometry.content_width() {
            self.row_neighborhood.clear();
            self.anchor_is_row_start = self.anchor == self.document.source_start();
        }
        self.geometry = geometry;
        self.locator = None;

        if !geometry.is_usable() {
            if self.follow_end && self.viewport_request.is_none() {
                self.viewport_request = Some(ViewportRequest::End);
                self.follow_end = false;
            }
            return true;
        }

        let reader = self.document.reader(&mut self.document_cache);
        let rebuilt = ensure_viewport_layout(
            &mut self.layout,
            &reader,
            geometry.content_width(),
            geometry.body_height(),
        );
        if self.viewport_request.is_some() {
            return true;
        }
        if self.follow_end {
            self.follow_end = false;
            self.viewport_request = Some(ViewportRequest::End);
        } else if rebuilt && self.anchor != self.document.source_start() {
            self.viewport_request = Some(ViewportRequest::Reflow {
                target: self.anchor,
            });
        }
        true
    }

    fn move_rows(&mut self, downward: bool, amount: usize) -> bool {
        let canceled_search = self.cancel_search_navigation();
        let canceled_jump = std::mem::take(&mut self.search_jump_pending);
        if downward && self.follow_end && self.viewport_request.is_none() {
            return canceled_search || canceled_jump;
        }

        let amount = i64::try_from(amount).expect("viewport row counts fit in i64");
        let delta = if downward { amount } else { -amount };
        if self.viewport_request.is_some_and(ViewportRequest::is_move) {
            self.queued_rows = self.queued_rows.saturating_add(delta);
            return true;
        }

        let canceled_viewport = self.cancel_viewport_request();
        self.queued_rows = delta;
        let scheduled = self.start_queued_move();
        canceled_search || canceled_jump || canceled_viewport || scheduled
    }

    fn document_start(&mut self) -> bool {
        let source_start = self.document.source_start();
        let changed = self.cancel_viewport_request()
            || self.cancel_search_navigation()
            || std::mem::take(&mut self.search_jump_pending)
            || self.anchor != source_start
            || self.follow_end;
        self.anchor = source_start;
        self.anchor_is_row_start = true;
        self.follow_end = false;
        changed
    }

    fn document_end(&mut self) -> bool {
        let canceled_search = self.cancel_search_navigation();
        let canceled_jump = std::mem::take(&mut self.search_jump_pending);
        if self.viewport_request == Some(ViewportRequest::End) {
            return canceled_search || canceled_jump;
        }
        if self.follow_end && self.viewport_request.is_none() {
            return canceled_search || canceled_jump;
        }
        self.cancel_viewport_request();
        self.viewport_request = Some(ViewportRequest::End);
        self.follow_end = false;
        true
    }

    fn cancel_viewport_request(&mut self) -> bool {
        let changed = self.viewport_request.is_some() || self.queued_rows != 0;
        self.viewport_request = None;
        self.locator = None;
        self.queued_rows = 0;
        changed
    }

    fn start_queued_move(&mut self) -> bool {
        if self.viewport_request.is_some() || self.queued_rows == 0 || !self.geometry.is_usable() {
            return false;
        }
        let limit = i64::from(self.geometry.body_height().expect("usable geometry").get());
        let step = self.queued_rows.clamp(-limit, limit);
        self.queued_rows -= step;
        let downward = step > 0;
        let amount =
            usize::try_from(step.unsigned_abs()).expect("viewport row counts fit in usize");
        let follow_end = if downward && self.follow_end {
            FollowEndPolicy::Always
        } else if downward {
            FollowEndPolicy::AtEnd
        } else {
            FollowEndPolicy::Never
        };
        self.viewport_request = Some(ViewportRequest::Move {
            target: self.anchor,
            delta: if downward {
                RowDelta::Forward(amount)
            } else {
                RowDelta::Backward(amount)
            },
            follow_end,
        });
        self.follow_end = false;
        true
    }

    fn cancel_search_navigation(&mut self) -> bool {
        let changed = self.navigation.is_some() || self.pending_navigation != 0;
        if let Some(mut navigation) = self.navigation.take()
            && let Some(block) = navigation.take_block()
        {
            self.match_block = Some(block);
        }
        self.pending_navigation = 0;
        changed
    }

    fn begin_search(&mut self) -> bool {
        if !matches!(self.viewport_request, Some(ViewportRequest::Reflow { .. })) {
            self.cancel_viewport_request();
        }
        self.cancel_search_navigation();
        self.search_jump_pending = false;
        self.mode = Mode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        };
        true
    }

    fn insert_search(&mut self, character: char) -> Result<bool, TutError> {
        let Mode::SearchInput { draft, limit_hit } = &mut self.mode else {
            return Ok(false);
        };
        if draft.len() + character.len_utf8() > SEARCH_DRAFT_LIMIT_BYTES {
            let changed = !*limit_hit;
            *limit_hit = true;
            return Ok(changed);
        }
        draft
            .try_reserve(character.len_utf8())
            .map_err(|_| TutError::Allocation("search query"))?;
        draft.push(character);
        *limit_hit = false;
        Ok(true)
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

    fn cancel_committed_search(&mut self) -> bool {
        if self.committed_query.is_empty() {
            return false;
        }
        if matches!(self.viewport_request, Some(ViewportRequest::Search { .. })) {
            self.viewport_request = None;
            self.locator = None;
        }
        self.committed_query.clear();
        self.search_index = None;
        self.current_match = None;
        self.navigation = None;
        self.match_block = None;
        self.pending_navigation = 0;
        self.highlights = None;
        self.search_jump_pending = false;
        true
    }

    fn commit_search(&mut self) -> Result<bool, TutError> {
        let Mode::SearchInput { draft, .. } = std::mem::replace(&mut self.mode, Mode::Reading)
        else {
            return Ok(false);
        };

        if draft.is_empty() {
            self.committed_query.clear();
            self.search_index = None;
            self.current_match = None;
            self.navigation = None;
            self.match_block = None;
            self.pending_navigation = 0;
            self.highlights = None;
            self.search_jump_pending = false;
            return Ok(true);
        }

        let first_visible = self
            .viewport()?
            .map_or(self.document.source_start(), |viewport| {
                viewport.first_visible_start
            });
        let reader = self.document.reader(&mut self.search_cache);
        let index = SearchIndex::new(&reader, &draft, first_visible)?
            .expect("nonempty queries create search indexes");
        let search_jump_pending = !index.is_complete();
        self.committed_query = draft;
        self.search_index = Some(index);
        self.current_match = None;
        self.navigation = None;
        self.match_block = None;
        self.pending_navigation = 0;
        self.highlights = None;
        self.follow_end = false;
        self.search_jump_pending = search_jump_pending;
        self.search_turn = true;
        Ok(true)
    }

    fn select_match(&mut self, forward: bool) -> bool {
        let Some(index) = self.search_index.as_ref() else {
            return false;
        };
        if index.is_complete() && self.current_match.is_none() {
            return false;
        }
        self.cancel_viewport_request();
        self.follow_end = false;
        if self.current_match.is_none() {
            self.search_jump_pending = true;
        }
        self.pending_navigation = if forward {
            self.pending_navigation.saturating_add(1)
        } else {
            self.pending_navigation.saturating_sub(1)
        };
        true
    }

    fn schedule_search_jump(&mut self, selected: SearchRange) -> bool {
        let request = ViewportRequest::Search {
            target: selected.start(),
        };
        let changed = self.viewport_request != Some(request) || self.follow_end;
        self.viewport_request = Some(request);
        self.locator = None;
        self.queued_rows = 0;
        self.follow_end = false;
        changed
    }

    fn build_render_viewport(&mut self) -> Result<(Option<Viewport>, RenderRows), TutError> {
        let Some(row_capacity) = self.geometry.body_height().map(BodyHeight::get) else {
            self.render_cache = None;
            return Ok((None, RenderRows::default()));
        };
        if let Some(cached) = self
            .render_cache
            .as_ref()
            .filter(|cached| cached.geometry == self.geometry && cached.anchor == self.anchor)
        {
            self.document.validate()?;
            return Ok((Some(cached.viewport), cached.rows.try_clone()?));
        }
        self.render_cache = None;
        let layout = self.layout.as_ref().expect("viewport has a layout");
        let mut rows = RenderRowsBuilder::new(usize::from(row_capacity))?;
        let mut reader = self.document.reader(&mut self.document_cache);
        let (visible_rows, visible_end) =
            layout.project_visible_rows(&mut reader, self.anchor, &mut rows)?;
        let rows = rows.finish();
        debug_assert_eq!(rows.len(), visible_rows);
        let viewport = Viewport {
            visible_rows,
            first_visible_start: self.anchor,
            visible_end,
        };
        self.render_cache = Some(RenderViewportCache {
            geometry: self.geometry,
            anchor: self.anchor,
            viewport,
            rows: rows.try_clone()?,
        });
        Ok((Some(viewport), rows))
    }
}

#[derive(Debug, Clone, Copy)]
struct RenderRowsCheckpoint {
    text: usize,
    spans: usize,
}

struct RenderRowsBuilder {
    text: String,
    spans: Vec<RenderSpan>,
    rows: Vec<RenderRowRange>,
    row_start: RenderRowsCheckpoint,
}

impl RenderRowsBuilder {
    fn new(row_capacity: usize) -> Result<Self, TutError> {
        let mut rows = Vec::new();
        rows.try_reserve_exact(row_capacity)
            .map_err(|_| TutError::Allocation("visible rows"))?;
        Ok(Self {
            text: String::new(),
            spans: Vec::new(),
            rows,
            row_start: RenderRowsCheckpoint { text: 0, spans: 0 },
        })
    }

    fn finish(self) -> RenderRows {
        debug_assert_eq!(self.row_start.text, self.text.len());
        debug_assert_eq!(self.row_start.spans, self.spans.len());
        RenderRows {
            text: self.text,
            spans: self.spans,
            rows: self.rows,
        }
    }
}

impl ProjectedRowSink for RenderRowsBuilder {
    type Checkpoint = RenderRowsCheckpoint;

    fn checkpoint(&self) -> Self::Checkpoint {
        RenderRowsCheckpoint {
            text: self.text.len(),
            spans: self.spans.len(),
        }
    }

    fn push(&mut self, atom: ProjectedAtom<'_>) -> Result<(), TutError> {
        if self.spans.len() == self.spans.capacity() {
            self.spans
                .try_reserve(1)
                .map_err(|_| TutError::Allocation("visible row spans"))?;
        }
        let span = RenderSpan::from_projected(atom, Highlight::None, &mut self.text)?;
        self.spans.push(span);
        Ok(())
    }

    fn finish_row(&mut self, through: Self::Checkpoint, carry_tail: bool) -> Result<(), TutError> {
        debug_assert!(through.text >= self.row_start.text && through.text <= self.text.len());
        debug_assert!(through.spans >= self.row_start.spans && through.spans <= self.spans.len());
        if !carry_tail {
            self.text.truncate(through.text);
            self.spans.truncate(through.spans);
        }
        for span in &mut self.spans[self.row_start.spans..through.spans] {
            span.text.start -= self.row_start.text;
            span.text.end -= self.row_start.text;
        }
        if self.rows.len() == self.rows.capacity() {
            self.rows
                .try_reserve(1)
                .map_err(|_| TutError::Allocation("visible rows"))?;
        }
        self.rows.push(RenderRowRange {
            text: self.row_start.text..through.text,
            spans: self.row_start.spans..through.spans,
        });
        self.row_start = through;
        Ok(())
    }
}

struct MatchCursor<'a> {
    ranges: &'a [SearchRange],
    next: usize,
    spanning: Option<SearchRange>,
    current: Option<SearchRange>,
}

impl<'a> MatchCursor<'a> {
    fn new(ranges: &'a [SearchRange], current: Option<SearchRange>) -> Self {
        Self {
            ranges,
            next: 0,
            spanning: None,
            current,
        }
    }

    fn role_for(&mut self, atom: GraphemeRange) -> Highlight {
        let mut role = self.current.map_or(Highlight::None, |current| {
            if intersects(current, atom) {
                Highlight::Current
            } else {
                Highlight::None
            }
        });
        if let Some(active) = self.spanning.take() {
            if intersects(active, atom) {
                role = promote(role, active, self.current);
            }
            if active.end() > atom.end() {
                self.spanning = Some(active);
                return role;
            }
        }

        while let Some(&range) = self.ranges.get(self.next) {
            if range.start() >= atom.end() {
                break;
            }
            self.next += 1;
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

    const BACKGROUND_STEP_LIMIT: usize = 100_000;

    fn reader(text: &str, columns: u16, rows: u16) -> App {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), text.to_owned());
        app.update(Action::Resize(Geometry::new(columns, rows)))
            .unwrap();
        app
    }

    fn submit(app: &mut App, query: &str) {
        app.update(Action::BeginSearch).unwrap();
        for character in query.chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }
        app.update(Action::SearchCommit).unwrap();
    }

    fn commit(app: &mut App, query: &str) {
        submit(app, query);
        settle(app);
    }

    fn settle(app: &mut App) {
        settle_count(app);
    }

    fn settle_count(app: &mut App) -> (usize, usize) {
        let mut changes = 0;
        for steps in 0..BACKGROUND_STEP_LIMIT {
            if !app.has_background_work() {
                return (steps, changes);
            }
            changes += usize::from(app.advance_background().unwrap());
        }
        panic!("background work exceeded the test step limit");
    }

    fn advance_until(app: &mut App, ready: impl Fn(&App) -> bool) {
        for _ in 0..BACKGROUND_STEP_LIMIT {
            if ready(app) {
                return;
            }
            app.advance_background().unwrap();
        }
        panic!("background condition exceeded the test step limit");
    }

    fn locate(app: &mut App, target: SourceOffset, delta: RowDelta) -> LocatedViewport {
        let height = app.geometry.body_height().unwrap();
        let mut locator = ViewportLocator::new(target, delta, height).unwrap();
        let mut neighborhood = RowNeighborhood::default();
        for _ in 0..BACKGROUND_STEP_LIMIT {
            let layout = app.layout.as_ref().unwrap();
            let mut reader = app.document.reader(&mut app.document_cache);
            if let Some(located) = locator
                .advance(layout, &mut reader, &mut neighborhood)
                .unwrap()
            {
                return located;
            }
        }
        panic!("viewport locator exceeded the test step limit");
    }

    #[test]
    fn navigation_clamps_and_preserves_end_following_across_reflow() {
        let mut app = reader("0123456789abcdef", 16, 4);
        assert_eq!(
            app.viewport().unwrap().unwrap().first_visible_start,
            SourceOffset::ZERO
        );
        assert_eq!(app.update(Action::DocumentEnd).unwrap(), Outcome::Changed);
        assert!(!app.follow_end);
        settle(&mut app);
        assert!(app.follow_end);
        assert_eq!(app.progress_percent().unwrap(), 100);
        app.update(Action::LineDown).unwrap();
        assert!(app.follow_end);
        assert_eq!(
            app.update(Action::Resize(Geometry::new(20, 4))).unwrap(),
            Outcome::Changed
        );
        assert!(!app.follow_end);
        settle(&mut app);
        assert!(app.follow_end);
        assert_eq!(
            app.viewport().unwrap().unwrap().first_visible_start,
            SourceOffset::ZERO
        );
        assert_eq!(
            app.update(Action::Resize(Geometry::new(20, 4))).unwrap(),
            Outcome::Unchanged
        );
        assert!(app.follow_end);
        assert_eq!(
            app.update(Action::Resize(Geometry::new(10, 3))).unwrap(),
            Outcome::Changed
        );
        assert!(!app.follow_end);
        assert_eq!(app.viewport_request, Some(ViewportRequest::End));
        assert!(!app.has_background_work());
        assert_eq!(
            app.update(Action::Resize(Geometry::new(10, 3))).unwrap(),
            Outcome::Unchanged
        );
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        settle(&mut app);
        assert!(app.follow_end);
        app.update(Action::LineUp).unwrap();
        assert!(!app.follow_end);
    }

    #[test]
    fn document_end_scans_long_lines_in_bounded_background_steps() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2 + 17);
        let mut app = reader(&text, 16, 4);
        let expected = SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2 + 16);

        assert_eq!(app.update(Action::DocumentEnd).unwrap(), Outcome::Changed);
        assert_eq!(app.anchor, SourceOffset::ZERO);
        assert!(app.has_background_work());
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.anchor, SourceOffset::ZERO);
        assert!(app.locator.is_some());

        settle(&mut app);
        assert_eq!(app.anchor, expected);
        assert!(app.follow_end);
        assert!(app.locator.is_none());
    }

    #[test]
    fn document_end_matches_layout_across_line_endings_and_empty_rows() {
        for text in [
            "",
            "a",
            "a\n",
            "a\r\nb\rc\n",
            "0123456789abcdefx\nshort\n",
            "word word word word\n\n終",
        ] {
            for rows in [4, 6, 8] {
                let mut app = reader(text, 16, rows);
                let expected = {
                    let layout = app.layout.as_ref().unwrap();
                    let mut reader = app.document.reader(&mut app.document_cache);
                    layout.last_viewport_start(&mut reader).unwrap()
                };

                app.update(Action::DocumentEnd).unwrap();
                settle(&mut app);
                assert_eq!(app.anchor, expected, "text={text:?}, rows={rows}");
                assert!(app.follow_end);
            }
        }
    }

    #[test]
    fn viewport_locator_matches_synchronous_layout_for_every_character_offset() {
        let text = "alpha beta gamma delta\r\n\n終わり\tline\r0123456789abcdefghi";
        let mut app = reader(text, 16, 7);
        let offsets: Vec<_> = text
            .char_indices()
            .map(|(offset, _)| SourceOffset::from_usize(offset))
            .chain(std::iter::once(SourceOffset::from_usize(text.len())))
            .collect();

        for target in offsets {
            for (downward, delta) in [
                (false, RowDelta::Backward(0)),
                (false, RowDelta::Backward(1)),
                (false, RowDelta::Backward(5)),
                (true, RowDelta::Forward(1)),
                (true, RowDelta::Forward(5)),
            ] {
                let (expected, at_end) = {
                    let layout = app.layout.as_ref().unwrap();
                    let mut reader = app.document.reader(&mut app.document_cache);
                    let amount = match delta {
                        RowDelta::Backward(amount) | RowDelta::Forward(amount) => amount,
                    };
                    let expected = layout
                        .move_row_start(&mut reader, target, downward, amount)
                        .unwrap();
                    let at_end = layout.is_last_viewport(&mut reader, expected).unwrap();
                    (expected, at_end)
                };
                assert_eq!(
                    locate(&mut app, target, delta),
                    LocatedViewport {
                        anchor: expected,
                        at_end,
                    },
                    "target={}, delta={delta:?}",
                    target.get()
                );
            }
        }
    }

    #[test]
    fn deep_reflow_and_search_remain_background_while_cached_moves_are_immediate() {
        let match_start = SOURCE_WINDOW_BYTES * 2;
        let mut text = "x".repeat(match_start);
        text.push_str("needle");
        text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES * 2));
        let mut app = reader(&text, 16, 7);
        app.anchor = SourceOffset::from_usize(match_start);

        app.update(Action::Resize(Geometry::new(20, 7))).unwrap();
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Reflow { .. })
        ));
        assert_eq!(app.anchor, SourceOffset::from_usize(match_start));
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.anchor, SourceOffset::from_usize(match_start));
        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::from_usize(131_060));

        app.update(Action::LineUp).unwrap();
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Move {
                delta: RowDelta::Backward(1),
                ..
            })
        ));
        let before_move = app.anchor;
        assert!(app.advance_background().unwrap());
        assert_eq!(app.anchor, SourceOffset::new(before_move.get() - 20));
        assert!(app.viewport_request.is_none());

        submit(&mut app, "needle");
        advance_until(&mut app, |app| app.current_match.is_some());
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Search { .. })
        ));
        let before_search = app.anchor;
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.anchor, before_search);
        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::from_usize(131_020));
    }

    #[test]
    fn forward_moves_from_known_rows_do_not_rescan_long_line_prefixes() {
        let width = 16usize;
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), width as u16, 7);
        let anchor = SourceOffset::from_usize(width * 2_000);
        app.anchor = anchor;
        app.document_cache = DocumentCache::default();
        app.document_cache.reset_metrics();

        app.update(Action::LineDown).unwrap();
        settle(&mut app);

        assert_eq!(app.anchor, anchor.checked_add(width).unwrap());
        let height = usize::from(app.geometry.body_height().unwrap().get());
        assert!(app.document_cache.metrics().grapheme_emissions() <= (height + 1) * (width + 1));
    }

    #[test]
    fn recent_row_edges_make_repeated_long_line_navigation_constant_work() {
        let width = 16usize;
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), width as u16, 7);
        let initial = SourceOffset::from_usize(width * 2_000);
        app.anchor = initial;

        for _ in 0..100 {
            app.update(Action::LineDown).unwrap();
            settle(&mut app);
        }
        assert_eq!(app.anchor, initial.checked_add(width * 100).unwrap());

        app.document_cache.reset_metrics();
        for _ in 0..100 {
            app.update(Action::LineUp).unwrap();
            settle(&mut app);
        }

        assert_eq!(app.anchor, initial);
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), 0);
    }

    #[test]
    fn cached_row_navigation_matches_the_layout_across_unicode_and_endings() {
        let text = "alpha\t界 e\u{301} beta\r\nblank\rwide🙂word\n".repeat(40);
        let mut app = reader(&text, 16, 7);

        for downward in std::iter::repeat_n(true, 60).chain(std::iter::repeat_n(false, 60)) {
            let expected = {
                let layout = app.layout.as_ref().unwrap();
                let mut reader = app.document.reader(&mut app.document_cache);
                layout
                    .move_row_start(&mut reader, app.anchor, downward, 1)
                    .unwrap()
            };
            app.update(if downward {
                Action::LineDown
            } else {
                Action::LineUp
            })
            .unwrap();
            settle(&mut app);
            assert_eq!(app.anchor, expected);
        }
    }

    #[test]
    fn locator_prefixes_long_previous_lines_in_bounded_steps() {
        let line_bytes = SOURCE_WINDOW_BYTES * 2;
        let mut text = "x".repeat(line_bytes);
        text.push_str("\nneedle\nend");
        let mut app = reader(&text, 16, 7);
        let target = SourceOffset::from_usize(line_bytes + 1);

        app.schedule_search_jump(
            SearchRange::new(target, target.checked_add("needle".len()).unwrap()).unwrap(),
        );
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.anchor, SourceOffset::ZERO);
        assert!(app.locator.is_some());

        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::from_usize(line_bytes - 32));
    }

    #[test]
    fn queued_row_actions_preserve_input_order() {
        let mut app = reader("0\n1\n2\n3\n4\n5\n6\n7\n8\n9", 16, 6);

        app.update(Action::LineDown).unwrap();
        app.update(Action::LineDown).unwrap();
        app.update(Action::LineDown).unwrap();
        assert_eq!(app.queued_rows, 2);
        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(6));

        app.update(Action::LineDown).unwrap();
        app.update(Action::LineUp).unwrap();
        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(6));

        app.update(Action::PageDown).unwrap();
        assert!(app.viewport_request.is_some());
        app.update(Action::DocumentStart).unwrap();
        assert!(app.viewport_request.is_none());
        assert_eq!(app.queued_rows, 0);
        assert_eq!(app.anchor, SourceOffset::ZERO);
    }

    #[test]
    fn moves_during_pending_width_reflow_relocate_the_old_anchor() {
        let mut app = reader(&"x".repeat(1024), 20, 7);
        app.anchor = SourceOffset::new(40);

        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        assert!(!app.anchor_is_row_start);
        app.update(Action::LineDown).unwrap();
        settle(&mut app);

        assert_eq!(app.anchor, SourceOffset::new(48));
        assert!(app.anchor_is_row_start);
    }

    #[test]
    fn resizing_a_pending_move_relocates_the_old_width_anchor() {
        let mut app = reader(&"x".repeat(1024), 20, 7);
        app.anchor = SourceOffset::new(40);

        app.update(Action::LineDown).unwrap();
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        assert!(!app.anchor_is_row_start);
        settle(&mut app);

        assert_eq!(app.anchor, SourceOffset::new(48));
        assert!(app.anchor_is_row_start);
    }

    #[test]
    fn manual_navigation_suppresses_an_older_incremental_search_jump() {
        let mut text = "x".repeat(SOURCE_WINDOW_BYTES * 2);
        text.push_str("needle");
        let mut app = reader(&text, 16, 4);
        submit(&mut app, "needle");

        app.update(Action::LineDown).unwrap();
        settle(&mut app);

        assert_eq!(app.anchor, SourceOffset::new(16));
        assert_eq!(app.current_match.unwrap().end(), app.document.source_end());
        assert!(!app.search_jump_pending);
        assert!(app.viewport_request.is_none());
    }

    #[test]
    fn search_input_pauses_but_does_not_discard_structural_reflow() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2);
        let mut app = reader(&text, 16, 4);
        app.anchor = SourceOffset::from_usize(SOURCE_WINDOW_BYTES);
        app.update(Action::Resize(Geometry::new(20, 4))).unwrap();
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Reflow { .. })
        ));

        app.update(Action::BeginSearch).unwrap();
        assert!(!app.has_background_work());
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Reflow { .. })
        ));
        app.update(Action::SearchCancel).unwrap();
        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(65_520));
    }

    #[test]
    fn explicit_navigation_cancels_a_pending_document_end() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2);
        let mut app = reader(&text, 16, 4);
        app.update(Action::DocumentEnd).unwrap();
        app.advance_background().unwrap();
        assert!(app.locator.is_some());

        assert_eq!(app.update(Action::LineDown).unwrap(), Outcome::Changed);
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Move { .. })
        ));
        assert!(app.locator.is_none());
        assert!(!app.follow_end);
        settle(&mut app);

        app.update(Action::DocumentEnd).unwrap();
        assert_eq!(app.update(Action::BeginSearch).unwrap(), Outcome::Changed);
        assert!(app.viewport_request.is_none());
        assert!(app.locator.is_none());
    }

    #[test]
    fn document_end_supersedes_an_older_search_jump() {
        let mut text = "needle".to_owned();
        text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES * 2));
        let mut app = reader(&text, 16, 4);
        submit(&mut app, "needle");

        app.update(Action::DocumentEnd).unwrap();
        settle(&mut app);

        assert_eq!(app.current_match.unwrap().start(), SourceOffset::ZERO);
        assert_eq!(
            app.anchor,
            SourceOffset::from_usize(text.len() - text.len().rem_euclid(16))
        );
        assert!(app.follow_end);
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
        settle(&mut app);
        assert_ne!(app.current_match, Some(first));
        app.update(Action::PreviousMatch).unwrap();
        settle(&mut app);
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
    fn committed_search_is_incremental_and_cancelable() {
        let mut text = "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2);
        text.push_str("needle");
        let mut app = reader(&text, 16, 4);

        submit(&mut app, "needle");
        assert!(matches!(
            app.search_status(),
            SearchStatus::Committed {
                no_matches: false,
                searching: true,
                ..
            }
        ));
        assert_eq!(app.current_match, None);
        assert!(app.has_background_work());

        app.advance_background().unwrap();
        assert_eq!(app.current_match, None);
        assert_eq!(app.update(Action::SearchCancel).unwrap(), Outcome::Changed);
        assert_eq!(app.search_status(), SearchStatus::None);
        assert!(!app.has_background_work());
    }

    #[test]
    fn editing_a_new_query_pauses_the_committed_search() {
        let mut text = "x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2);
        text.push_str("needle");
        let mut app = reader(&text, 16, 4);
        submit(&mut app, "needle");
        let anchor = app.anchor;

        app.update(Action::BeginSearch).unwrap();
        assert!(!app.has_background_work());
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.anchor, anchor);
        assert_eq!(app.current_match, None);

        app.update(Action::SearchCancel).unwrap();
        assert!(app.has_background_work());
    }

    #[test]
    fn unusable_geometry_pauses_search_work() {
        let mut text = "x".repeat(SOURCE_WINDOW_BYTES * 2);
        text.push_str("needle");
        let mut app = reader(&text, 16, 4);
        submit(&mut app, "needle");

        app.update(Action::Resize(Geometry::new(10, 3))).unwrap();
        assert!(!app.has_background_work());
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.current_match, None);

        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle(&mut app);
        assert_eq!(app.current_match.unwrap().end(), app.document.source_end());
    }

    #[test]
    fn early_search_results_are_selected_before_scanning_finishes() {
        let mut text = "needle".to_owned();
        text.push_str(&"x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2));
        let mut app = reader(&text, 16, 4);

        submit(&mut app, "needle");
        app.advance_background().unwrap();

        assert_eq!(
            app.current_match.unwrap().start(),
            app.document.source_start()
        );
        assert!(matches!(
            app.search_status(),
            SearchStatus::Committed {
                searching: true,
                ..
            }
        ));
    }

    #[test]
    fn match_navigation_waits_for_incremental_search_without_losing_the_request() {
        let mut text = "cat".to_owned();
        text.push_str(&"x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2));
        text.push_str("cat");
        let mut app = reader(&text, 16, 4);
        submit(&mut app, "cat");
        app.advance_background().unwrap();
        let first = app.current_match.unwrap();

        app.update(Action::NextMatch).unwrap();
        assert_eq!(app.current_match, Some(first));
        assert!(matches!(
            app.search_status(),
            SearchStatus::Committed {
                searching: true,
                ..
            }
        ));
        settle(&mut app);

        assert_eq!(app.current_match.unwrap().end(), app.document.source_end());
    }

    #[test]
    fn a_new_query_replaces_pending_search_state() {
        let mut text = "alpha".to_owned();
        text.push_str(&"x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2));
        text.push_str("beta");
        let mut app = reader(&text, 16, 4);

        submit(&mut app, "alpha");
        app.advance_background().unwrap();
        assert!(app.current_match.is_some());
        submit(&mut app, "beta");
        assert_eq!(app.current_match, None);
        settle(&mut app);

        assert_eq!(app.committed_query, "beta");
        assert_eq!(app.current_match.unwrap().end(), app.document.source_end());
    }

    #[test]
    fn render_state_owns_rows_and_marks_all_matches() {
        let mut app = reader("cat cat", 16, 4);
        commit(&mut app, "cat");
        {
            let state = app.render_state().unwrap();
            let row = state.rows.get(0).unwrap();
            assert_eq!(row.spans[0].highlight, Highlight::Current);
            assert_eq!(row.spans[0].text(row.text), "cat");
            assert_eq!(row.spans[1].highlight, Highlight::None);
            assert_eq!(row.spans[1].text(row.text), " cat");
        }
        settle(&mut app);
        let state = app.render_state().unwrap();
        let row = state.rows.get(0).unwrap();
        assert_eq!(row.spans[0].highlight, Highlight::Current);
        assert_eq!(row.spans[0].text(row.text), "cat");
        assert_eq!(row.spans[2].highlight, Highlight::Match);
        assert_eq!(row.spans[2].text(row.text), "cat");
    }

    #[test]
    fn rendering_segments_each_visible_grapheme_once() {
        let mut app = reader("a\nb\nc\nd\ne\noutside\n", 16, 8);
        app.document_cache = DocumentCache::default();
        app.document_cache.reset_metrics();

        {
            let state = app.render_state().unwrap();
            assert_eq!(state.rows.len(), 5);
            assert!(state.rows.iter().all(|row| row.spans.len() <= 1));
        }

        let metrics = app.document_cache.metrics();
        assert_eq!(metrics.grapheme_emissions(), 10);
        assert_eq!(metrics.segmentation_runs(), 10);
    }

    #[test]
    fn repeated_rendering_reuses_the_unchanged_visible_body() {
        let mut app = reader("a\nb\nc\nd\ne\noutside\n", 16, 8);
        let first = app.render_state().unwrap().rows;
        app.document_cache.reset_metrics();

        let second = app.render_state().unwrap().rows;

        assert_eq!(second, first);
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), 0);
        assert_eq!(app.document_cache.metrics().grapheme_window_calls(), 0);
    }

    #[test]
    fn rendering_reads_one_grapheme_frontier_and_compacts_ascii_spans() {
        let mut app = reader(&"x".repeat(1024), 127, 4);
        app.document_cache = DocumentCache::with_window_bytes(128);
        app.document_cache.reset_metrics();

        {
            let state = app.render_state().unwrap();
            let row = state.rows.get(0).unwrap();
            assert_eq!(row.text.len(), 127);
            assert_eq!(row.spans.len(), 1);
            assert_eq!(row.spans[0].cell_width, DisplayColumn::new(127));
        }

        let metrics = app.document_cache.metrics();
        assert_eq!(metrics.grapheme_window_calls(), 2);
        assert_eq!(metrics.grapheme_window_returned_bytes(), 256);
        assert_eq!(metrics.grapheme_emissions(), 128);
        assert_eq!(metrics.segmentation_runs(), 129);
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
            app.render_state().unwrap().rows.get(1).unwrap().spans[0].highlight,
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
    fn cached_file_rendering_still_rejects_in_place_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing.txt");
        fs::write(&path, "first").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        app.render_state().unwrap();

        fs::write(&path, "other").unwrap();

        assert!(matches!(app.render_state(), Err(TutError::Load(_))));
    }

    #[test]
    fn cached_row_navigation_still_rejects_in_place_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-rows.txt");
        fs::write(&path, "x".repeat(64)).unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        app.update(Action::LineDown).unwrap();
        settle(&mut app);

        fs::write(&path, "y".repeat(64)).unwrap();
        app.update(Action::LineUp).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
        assert!(app.locator.is_none());
    }

    #[test]
    fn cached_match_navigation_still_rejects_in_place_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-matches.txt");
        fs::write(&path, "cat cat cat").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        commit(&mut app, "cat");
        app.update(Action::NextMatch).unwrap();
        settle(&mut app);
        assert!(app.match_block.is_some());

        fs::write(&path, "dog dog dog").unwrap();
        app.update(Action::NextMatch).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
    }

    #[test]
    fn file_backed_long_line_location_advances_in_bounded_steps() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("long-line.txt");
        let line_bytes = SOURCE_WINDOW_BYTES * 2;
        let mut text = "x".repeat(line_bytes);
        text.push_str("\nneedle\nend");
        fs::write(&path, text).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        settle(&mut app);

        let target = SourceOffset::from_usize(line_bytes + 1);
        let expected = {
            let height = usize::from(app.geometry.body_height().unwrap().get() / 2);
            let layout = app.layout.as_ref().unwrap();
            let mut reader = app.document.reader(&mut app.document_cache);
            layout
                .move_row_start(&mut reader, target, false, height)
                .unwrap()
        };
        let selected =
            SearchRange::new(target, target.checked_add("needle".len()).unwrap()).unwrap();

        app.schedule_search_jump(selected);
        assert!(!app.advance_background().unwrap());
        assert_eq!(app.anchor, SourceOffset::ZERO);
        assert!(app.locator.is_some());

        settle(&mut app);
        assert_eq!(app.anchor, expected);
    }

    #[test]
    fn file_index_and_search_take_fair_background_turns() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("search-book.txt");
        let mut text = "x".repeat(SOURCE_WINDOW_BYTES * 3);
        text.push_str("needle");
        fs::write(&path, text).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 6))).unwrap();
        submit(&mut app, "needle");
        let first_frontier = SourceOffset::from_usize(SOURCE_WINDOW_BYTES);

        assert!(!app.document.line_index_covers(first_frontier));
        assert!(!app.search_index.as_ref().unwrap().is_complete());
        assert!(!app.advance_background().unwrap());
        assert!(!app.document.line_index_covers(first_frontier));
        assert!(!app.advance_background().unwrap());
        assert!(app.document.line_index_covers(first_frontier));
        assert!(!app.search_index.as_ref().unwrap().is_complete());

        settle(&mut app);
        assert!(app.document.line_index_complete());
        assert!(app.search_index.as_ref().unwrap().is_complete());
        assert_eq!(app.current_match.unwrap().end(), app.document.source_end());
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

        let (advances, redraws) = settle_count(&mut app);

        assert_eq!(advances, 3);
        assert_eq!(redraws, 1);
        let complete = app.render_state().unwrap();
        assert_eq!(
            (complete.current_line, complete.total_lines),
            (Some(1), Some(30_001))
        );
    }

    #[test]
    fn local_navigation_does_not_wait_for_the_complete_file_index() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("large-book.txt");
        fs::write(&path, "line\n".repeat(30_000)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 6))).unwrap();

        app.update(Action::LineDown).unwrap();
        assert!(app.advance_background().unwrap());

        assert_eq!(app.anchor, SourceOffset::new(5));
        assert!(!app.document.line_index_complete());
        assert!(app.has_background_work());
    }

    #[test]
    fn document_end_waits_for_the_file_line_index() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("indexed-end.txt");
        let mut text = "line\n".repeat(15_000);
        text.push_str("end");
        fs::write(&path, text).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 6))).unwrap();

        app.update(Action::DocumentEnd).unwrap();
        assert!(!app.advance_background().unwrap());
        assert!(app.locator.is_none());
        assert_eq!(app.anchor, SourceOffset::ZERO);

        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(74_990));
        assert!(app.follow_end);
    }

    #[test]
    fn zero_width_source_has_one_owned_visible_cell() {
        let mut app = reader("\u{200b}", 16, 4);
        let state = app.render_state().unwrap();
        let row = state.rows.get(0).unwrap();
        let span = &row.spans[0];
        assert_eq!(span.text(row.text), "◌\u{200b}");
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
