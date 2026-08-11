use std::{mem::size_of, num::NonZeroU16, ops::Range};

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    document::{Document, DocumentCache},
    error::TutError,
    layout::{
        BodyHeight, ContentWidth, DOTTED_CIRCLE, DisplayColumn, DisplayProjection, GraphemeRange,
        MAX_RENDER_GRAPHEME_BYTES, ProjectedAtom, ProjectedRowSink, ProjectedRowsScanner,
        ProjectedScanAdvance, ProjectedScanMeter, REPLACEMENT_CHARACTER, ViewportLayout,
        ensure_viewport_layout, progress_percent,
    },
    line_index::LinePosition,
    locator::{LocatedViewport, RowDelta, RowNeighborhood, ViewportLocator, source_row_bound},
    search::{MAX_SEARCH_QUERY_BYTES, SearchHighlightKey, SearchRange, SearchSession},
    source::SourceOffset,
};

#[cfg(test)]
use crate::document::SOURCE_WINDOW_BYTES;

pub(super) const MIN_TERMINAL_COLUMNS: u16 = 16;
pub(super) const MIN_TERMINAL_ROWS: u16 = 4;
pub(super) const SEARCH_DRAFT_LIMIT_BYTES: usize = MAX_SEARCH_QUERY_BYTES;
pub(super) const MAX_REPEAT_COUNT: u16 = 9_999;
const CHROME_ROWS: u16 = 3;
const MAX_VISIBLE_RENDER_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRANSIENT_RENDER_TEXT_BYTES: usize =
    (u16::MAX as usize + 1) * (MAX_RENDER_GRAPHEME_BYTES + DOTTED_CIRCLE.len());

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
pub(super) enum ContentMode {
    Reading,
    SearchInput { draft: String, limit_hit: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Mode {
    Content(ContentMode),
    Help { return_to: ContentMode },
}

impl Mode {
    const fn content(&self) -> &ContentMode {
        match self {
            Self::Content(content) | Self::Help { return_to: content } => content,
        }
    }

    const fn is_help(&self) -> bool {
        matches!(self, Self::Help { .. })
    }

    const fn content_is_reading(&self) -> bool {
        matches!(self.content(), ContentMode::Reading)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Resize(Geometry),
    RepeatDigit(u8),
    RepeatBackspace,
    RepeatCancel,
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
    SearchRecall,
    SearchClearDraft,
    SearchCommit,
    SearchCancel,
    NextMatch,
    PreviousMatch,
    ShowHelp,
    DismissHelp,
    Interrupt,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Unchanged,
    Changed,
    Interrupt,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BackgroundWork {
    LineIndex,
    Viewport,
    Render,
    Search,
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
pub(super) struct RepeatStatus {
    value: NonZeroU16,
    limit_hit: bool,
}

impl RepeatStatus {
    pub(super) const fn value(self) -> u16 {
        self.value.get()
    }

    pub(super) const fn limit_hit(self) -> bool {
        self.limit_hit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewActivity {
    PreparingView,
    GoingToEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewportBoundary {
    Top,
    End,
    All,
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
    text: RenderTextRange,
    source: GraphemeRange,
    pub projection: RenderProjectionKind,
    pub cell_width: DisplayColumn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderTextRange {
    start: u32,
    end: u32,
}

impl RenderTextRange {
    fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end && end <= MAX_TRANSIENT_RENDER_TEXT_BYTES);
        Self {
            start: start as u32,
            end: end as u32,
        }
    }

    fn relative_to(self, base: u32) -> Range<usize> {
        debug_assert!(base <= self.start && base <= self.end);
        (self.start - base) as usize..(self.end - base) as usize
    }
}

impl RenderSpan {
    pub(super) fn from_projected(
        atom: ProjectedAtom<'_>,
        output: &mut String,
    ) -> Result<Self, TutError> {
        let pending = PendingRenderSpan::new(atom);
        let additional = pending
            .text_len()
            .ok_or(TutError::Allocation("visible row text"))?;
        output
            .try_reserve(additional)
            .map_err(|_| TutError::Allocation("visible row text"))?;
        Ok(pending.append_to(output))
    }

    fn text<'a>(&self, row: &'a str, row_start: u32) -> &'a str {
        row.get(self.text.relative_to(row_start))
            .expect("render spans retain valid row-text boundaries")
    }

    pub(super) fn standalone_text<'a>(&self, row: &'a str) -> &'a str {
        self.text(row, 0)
    }

    pub(super) const fn source(&self) -> GraphemeRange {
        self.source
    }

    pub(super) fn merge(&mut self, next: &Self) -> bool {
        if self.projection != next.projection
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

#[derive(Debug, Clone, Copy)]
enum PendingRenderText<'a> {
    Single(&'a str),
    DottedCircle(&'a str),
}

impl PendingRenderText<'_> {
    fn len(self) -> Option<usize> {
        match self {
            Self::Single(text) => Some(text.len()),
            Self::DottedCircle(source) => DOTTED_CIRCLE.len().checked_add(source.len()),
        }
    }

    fn append_to(self, output: &mut String) {
        match self {
            Self::Single(text) => output.push_str(text),
            Self::DottedCircle(source) => {
                output.push_str(DOTTED_CIRCLE);
                output.push_str(source);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingRenderSpan<'a> {
    source: GraphemeRange,
    projection: RenderProjectionKind,
    cell_width: DisplayColumn,
    text: PendingRenderText<'a>,
}

impl<'a> PendingRenderSpan<'a> {
    fn new(atom: ProjectedAtom<'a>) -> Self {
        let (projection, text) = match atom.projection() {
            DisplayProjection::Text(text) => {
                (RenderProjectionKind::Text, PendingRenderText::Single(text))
            }
            DisplayProjection::Spaces(count) => {
                let text = match count {
                    1 => " ",
                    2 => "  ",
                    3 => "   ",
                    4 => "    ",
                    _ => unreachable!("tab expansion is one through four cells"),
                };
                (
                    RenderProjectionKind::Spaces,
                    PendingRenderText::Single(text),
                )
            }
            DisplayProjection::Replacement => (
                RenderProjectionKind::Replacement,
                PendingRenderText::Single(REPLACEMENT_CHARACTER),
            ),
            DisplayProjection::DottedCircle(source) => (
                RenderProjectionKind::DottedCircle,
                PendingRenderText::DottedCircle(source),
            ),
        };
        Self {
            source: atom.source(),
            projection,
            cell_width: atom.width(),
            text,
        }
    }

    fn text_len(self) -> Option<usize> {
        self.text.len()
    }

    fn append_to(self, output: &mut String) -> RenderSpan {
        let start = output.len();
        self.text.append_to(output);
        RenderSpan {
            text: RenderTextRange::new(start, output.len()),
            source: self.source,
            projection: self.projection,
            cell_width: self.cell_width,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderRowRange {
    text: Range<usize>,
    spans: Range<usize>,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub(super) struct RenderRows {
    text: String,
    spans: Vec<RenderSpan>,
    rows: Vec<RenderRowRange>,
    #[cfg(test)]
    reserve_attempts: RenderReserveAttempts,
}

static EMPTY_RENDER_ROWS: RenderRows = RenderRows {
    text: String::new(),
    spans: Vec::new(),
    rows: Vec::new(),
    #[cfg(test)]
    reserve_attempts: RenderReserveAttempts::ZERO,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct RenderRow<'a> {
    pub text: &'a str,
    pub spans: &'a [RenderSpan],
    text_start: u32,
}

impl<'a> RenderRow<'a> {
    pub(super) fn span_text(&self, span: &RenderSpan) -> &'a str {
        span.text(self.text, self.text_start)
    }
}

impl RenderRows {
    pub(super) fn iter(&self) -> impl ExactSizeIterator<Item = RenderRow<'_>> + '_ {
        self.rows.iter().map(|row| {
            debug_assert!(row.text.start <= MAX_TRANSIENT_RENDER_TEXT_BYTES);
            RenderRow {
                text: self
                    .text
                    .get(row.text.clone())
                    .expect("render rows retain valid text boundaries"),
                spans: self
                    .spans
                    .get(row.spans.clone())
                    .expect("render rows retain valid span boundaries"),
                text_start: row.text.start as u32,
            }
        })
    }

    #[cfg(test)]
    pub(super) fn get(&self, index: usize) -> Option<RenderRow<'_>> {
        let row = self.rows.get(index)?;
        debug_assert!(row.text.start <= MAX_TRANSIENT_RENDER_TEXT_BYTES);
        Some(RenderRow {
            text: self.text.get(row.text.clone())?,
            spans: self.spans.get(row.spans.clone())?,
            text_start: row.text.start as u32,
        })
    }

    pub(super) const fn len(&self) -> usize {
        self.rows.len()
    }

    fn storage(&self) -> RenderStorage {
        RenderStorage {
            text: self.text.capacity(),
            spans: self.spans.capacity(),
            rows: self.rows.capacity(),
        }
    }

    fn clear(&mut self) {
        self.text.clear();
        self.spans.clear();
        self.rows.clear();
    }

    #[cfg(test)]
    const fn reserve_attempts(&self) -> RenderReserveAttempts {
        self.reserve_attempts
    }

    #[cfg(test)]
    fn storage_identity(
        &self,
    ) -> (
        *const Self,
        *const u8,
        *const RenderSpan,
        *const RenderRowRange,
    ) {
        (
            self,
            self.text.as_ptr(),
            self.spans.as_ptr(),
            self.rows.as_ptr(),
        )
    }

    #[cfg(test)]
    pub(super) fn storage_bytes(&self) -> usize {
        self.storage()
            .bytes()
            .expect("render-row storage capacities fit usize")
    }

    #[cfg(test)]
    pub(super) fn from_prebuilt_runs(
        run: &str,
        count: usize,
        cell_width: u16,
    ) -> Result<Self, TutError> {
        let text_bytes = run
            .len()
            .checked_mul(count)
            .ok_or(TutError::Allocation("test render rows"))?;
        let mut text = String::new();
        text.try_reserve_exact(text_bytes)
            .map_err(|_| TutError::Allocation("test render rows"))?;
        let mut spans = Vec::new();
        spans
            .try_reserve_exact(count)
            .map_err(|_| TutError::Allocation("test render rows"))?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(count)
            .map_err(|_| TutError::Allocation("test render rows"))?;

        for _ in 0..count {
            let start = text.len();
            text.push_str(run);
            let end = text.len();
            let span = spans.len();
            spans.push(RenderSpan {
                text: RenderTextRange::new(start, end),
                source: GraphemeRange::new(
                    SourceOffset::from_usize(start),
                    SourceOffset::from_usize(end),
                )
                .expect("prebuilt test runs are nonempty"),
                projection: RenderProjectionKind::Text,
                cell_width: DisplayColumn::new(u32::from(cell_width)),
            });
            rows.push(RenderRowRange {
                text: start..end,
                spans: span..span + 1,
            });
        }

        let rows = Self {
            text,
            spans,
            rows,
            reserve_attempts: RenderReserveAttempts::ZERO,
        };
        rows.storage().require(MAX_VISIBLE_RENDER_BYTES)?;
        Ok(rows)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RenderRowsView<'a> {
    rows: &'a RenderRows,
    ranges: &'a [SearchRange],
    current: Option<SearchRange>,
}

impl<'a> RenderRowsView<'a> {
    const fn new(
        rows: &'a RenderRows,
        ranges: &'a [SearchRange],
        current: Option<SearchRange>,
    ) -> Self {
        Self {
            rows,
            ranges,
            current,
        }
    }

    #[cfg(test)]
    pub(super) const fn empty() -> Self {
        Self::new(&EMPTY_RENDER_ROWS, &[], None)
    }

    #[cfg(test)]
    pub(super) const fn from_rows(rows: &'a RenderRows) -> Self {
        Self::new(rows, &[], None)
    }

    pub(super) fn iter(self) -> impl ExactSizeIterator<Item = RenderRow<'a>> + 'a {
        self.rows.iter()
    }

    #[cfg(test)]
    pub(super) fn get(self, index: usize) -> Option<RenderRow<'a>> {
        self.rows.get(index)
    }

    #[cfg(test)]
    pub(super) const fn len(self) -> usize {
        self.rows.len()
    }

    pub(super) const fn highlight_cursor(self) -> MatchCursor<'a> {
        MatchCursor::new(self.ranges, self.current)
    }

    #[cfg(test)]
    fn storage_identity(
        self,
    ) -> (
        *const RenderRows,
        *const u8,
        *const RenderSpan,
        *const RenderRowRange,
    ) {
        self.rows.storage_identity()
    }
}

#[derive(Debug)]
pub(super) struct RenderState<'a> {
    pub filename: &'a str,
    pub path: &'a str,
    pub rows: RenderRowsView<'a>,
    pub progress: u8,
    pub current_line: Option<u64>,
    pub total_lines: Option<u64>,
    pub status: SearchStatus<'a>,
    pub activity: Option<ViewActivity>,
    pub boundary: Option<ViewportBoundary>,
    pub repeat: Option<RepeatStatus>,
}

#[derive(Debug)]
pub(super) struct PendingState<'a> {
    pub filename: &'a str,
    pub path: &'a str,
    pub status: SearchStatus<'a>,
    pub repeat: Option<RepeatStatus>,
}

#[derive(Debug)]
pub(super) enum ViewState<'a> {
    Reader(RenderState<'a>),
    Pending(PendingState<'a>),
    Help { q_closes: bool },
}

struct RenderViewportCache {
    geometry: Geometry,
    anchor: SourceOffset,
    viewport: Viewport,
    at_end: bool,
    rows: RenderRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderKey {
    geometry: Geometry,
    anchor: SourceOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderAttempt {
    Reused,
    Fresh,
}

struct RenderJob {
    key: RenderKey,
    attempt: RenderAttempt,
    scanner: ProjectedRowsScanner<RenderRowsBuilder>,
}

enum RenderBody {
    Empty,
    Cached(RenderViewportCache),
    Scanning(RenderJob),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinePositionCacheKey {
    offset: SourceOffset,
    covered: bool,
    complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CachedLinePosition {
    key: LinePositionCacheKey,
    position: Option<LinePosition>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BackgroundSchedule {
    work: BackgroundWork,
    next_search_turn: Option<bool>,
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
    search: Option<SearchSession>,
    viewport_request: Option<ViewportRequest>,
    locator: Option<ViewportLocator>,
    row_neighborhood: RowNeighborhood,
    render_body: RenderBody,
    #[cfg(test)]
    render_limit: usize,
    line_position_cache: Option<CachedLinePosition>,
    queued_rows: i64,
    search_turn: bool,
    repeat: Option<RepeatStatus>,
    #[cfg(test)]
    fail_search_recall_reserve: bool,
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
            mode: Mode::Content(ContentMode::Reading),
            search: None,
            viewport_request: None,
            locator: None,
            row_neighborhood: RowNeighborhood::default(),
            render_body: RenderBody::Empty,
            #[cfg(test)]
            render_limit: MAX_VISIBLE_RENDER_BYTES,
            line_position_cache: None,
            queued_rows: 0,
            search_turn: true,
            repeat: None,
            #[cfg(test)]
            fail_search_recall_reserve: false,
        }
    }

    pub(super) const fn terminal_too_small(&self) -> bool {
        !self.geometry.is_usable()
    }

    pub(super) fn mode(&self) -> &Mode {
        &self.mode
    }

    pub(super) const fn repeat_active(&self) -> bool {
        self.repeat.is_some()
    }

    #[cfg(test)]
    pub(super) const fn repeat_status(&self) -> Option<RepeatStatus> {
        self.repeat
    }

    #[cfg(test)]
    fn current_match(&self) -> Option<SearchRange> {
        self.search.as_ref().and_then(SearchSession::current_match)
    }

    pub(super) fn search_status(&self) -> SearchStatus<'_> {
        match self.mode.content() {
            ContentMode::SearchInput { draft, limit_hit } => SearchStatus::Draft {
                draft,
                limit_hit: *limit_hit,
            },
            ContentMode::Reading => match &self.search {
                None => SearchStatus::None,
                Some(search) => SearchStatus::Committed {
                    query: search.query(),
                    no_matches: search.no_matches(),
                    searching: search.is_searching()
                        || matches!(self.viewport_request, Some(ViewportRequest::Search { .. })),
                },
            },
        }
    }

    #[cfg(test)]
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

    const fn render_key(&self) -> RenderKey {
        RenderKey {
            geometry: self.geometry,
            anchor: self.anchor,
        }
    }

    fn render_cache(&self) -> Option<&RenderViewportCache> {
        match &self.render_body {
            RenderBody::Cached(cache) => Some(cache),
            RenderBody::Empty | RenderBody::Scanning(_) => None,
        }
    }

    fn current_render_cache(&self) -> Option<&RenderViewportCache> {
        let key = self.render_key();
        self.render_cache()
            .filter(|cache| cache.geometry == key.geometry && cache.anchor == key.anchor)
    }

    fn reader_frame_ready(&self) -> bool {
        !self.geometry.is_usable() || self.current_render_cache().is_some()
    }

    pub(super) fn frame_ready(&self) -> bool {
        self.mode.is_help() || self.reader_frame_ready()
    }

    fn cancel_render_scan(&mut self) -> bool {
        if matches!(self.render_body, RenderBody::Scanning(_)) {
            self.render_body = RenderBody::Empty;
            true
        } else {
            false
        }
    }

    pub(super) fn render_state(&mut self) -> Result<RenderState<'_>, TutError> {
        debug_assert!(self.reader_frame_ready());
        let viewport = self.current_render_cache().map(|cache| cache.viewport);
        let boundary = self.viewport_boundary();
        if viewport.is_some() {
            self.document.validate()?;
        }
        self.prepare_search_highlights(viewport)?;
        let line = if viewport.is_some() {
            self.validated_line_position_for(viewport)?
        } else {
            self.line_position_for(viewport)?
        };
        let progress = self.progress_for(viewport);
        let (ranges, current) = if viewport.is_some() {
            let key = self
                .current_search_highlight_key()
                .expect("visible viewports retain a cached render body");
            self.search.as_ref().map_or((&[][..], None), |search| {
                (search.highlight_ranges(key), search.current_match())
            })
        } else {
            (&[][..], None)
        };
        let rows = self
            .current_render_cache()
            .map_or(&EMPTY_RENDER_ROWS, |cached| &cached.rows);
        Ok(RenderState {
            filename: self.document.display_name(),
            path: self.document.display_path(),
            rows: RenderRowsView::new(rows, ranges, current),
            progress,
            current_line: line.map(LinePosition::current),
            total_lines: line.and_then(LinePosition::total),
            status: self.search_status(),
            activity: self.view_activity(),
            boundary,
            repeat: self.repeat,
        })
    }

    fn viewport_boundary(&self) -> Option<ViewportBoundary> {
        let cache = self.current_render_cache()?;
        let at_top = cache.viewport.first_visible_start == self.document.source_start();
        match (at_top, cache.at_end) {
            (true, true) => Some(ViewportBoundary::All),
            (true, false) => Some(ViewportBoundary::Top),
            (false, true) => Some(ViewportBoundary::End),
            (false, false) => None,
        }
    }

    pub(super) fn view_state(&mut self) -> Result<ViewState<'_>, TutError> {
        if let Mode::Help { return_to } = &self.mode {
            return Ok(ViewState::Help {
                q_closes: matches!(return_to, ContentMode::Reading),
            });
        }
        if !self.reader_frame_ready() {
            return Ok(ViewState::Pending(PendingState {
                filename: self.document.display_name(),
                path: self.document.display_path(),
                status: self.search_status(),
                repeat: self.repeat,
            }));
        }
        self.render_state().map(ViewState::Reader)
    }

    const fn view_activity(&self) -> Option<ViewActivity> {
        match self.viewport_request {
            Some(ViewportRequest::End) => Some(ViewActivity::GoingToEnd),
            Some(ViewportRequest::Move { .. } | ViewportRequest::Reflow { .. }) => {
                Some(ViewActivity::PreparingView)
            }
            Some(ViewportRequest::Search { .. }) | None => None,
        }
    }

    fn prepare_search_highlights(&mut self, viewport: Option<Viewport>) -> Result<(), TutError> {
        let Some(_) = viewport else {
            if let Some(search) = &mut self.search {
                search.invalidate_highlights();
            }
            return Ok(());
        };
        if !matches!(self.mode, Mode::Content(ContentMode::Reading)) {
            if let Some(search) = &mut self.search {
                search.invalidate_highlights();
            }
            return Ok(());
        }
        let key = self
            .current_search_highlight_key()
            .expect("visible viewports retain rendered rows");
        let Some(search) = &mut self.search else {
            return Ok(());
        };
        search.prepare_highlights(key);
        Ok(())
    }

    fn current_search_highlight_key(&self) -> Option<SearchHighlightKey> {
        let cache = self.current_render_cache()?;
        Some(SearchHighlightKey::new(
            cache.geometry.columns,
            cache.geometry.rows,
            cache.anchor,
            cache.viewport.visible_end,
            cache.rows.spans.len(),
        ))
    }

    fn line_position_for(
        &mut self,
        viewport: Option<Viewport>,
    ) -> Result<Option<LinePosition>, TutError> {
        let key = self.line_position_cache_key(viewport);
        if let Some(cached) = self.cached_line_position(key) {
            self.document.validate()?;
            return Ok(cached.position);
        }
        self.read_line_position(key)
    }

    fn validated_line_position_for(
        &mut self,
        viewport: Option<Viewport>,
    ) -> Result<Option<LinePosition>, TutError> {
        let key = self.line_position_cache_key(viewport);
        if let Some(cached) = self.cached_line_position(key) {
            return Ok(cached.position);
        }
        self.read_line_position(key)
    }

    fn line_position_cache_key(&self, viewport: Option<Viewport>) -> LinePositionCacheKey {
        let offset = viewport.map_or(self.document.source_start(), |viewport| {
            viewport.first_visible_start
        });
        LinePositionCacheKey {
            offset,
            covered: self.document.line_index_covers(offset),
            complete: self.document.line_index_complete(),
        }
    }

    fn cached_line_position(&self, key: LinePositionCacheKey) -> Option<CachedLinePosition> {
        self.line_position_cache.filter(|cached| cached.key == key)
    }

    fn read_line_position(
        &mut self,
        key: LinePositionCacheKey,
    ) -> Result<Option<LinePosition>, TutError> {
        let mut reader = self.document.reader(&mut self.document_cache);
        let position = reader.line_position(key.offset)?;
        self.line_position_cache = Some(CachedLinePosition { key, position });
        Ok(position)
    }

    fn background_schedule(&self) -> Option<BackgroundSchedule> {
        let viewport_pending = self.geometry.is_usable()
            && self.viewport_request.is_some_and(|request| {
                self.mode.content_is_reading() || matches!(request, ViewportRequest::Reflow { .. })
            });
        if viewport_pending {
            let request = self
                .viewport_request
                .expect("viewport work retains its request");
            return Some(BackgroundSchedule {
                work: if self.viewport_request_is_ready(request) {
                    BackgroundWork::Viewport
                } else {
                    BackgroundWork::LineIndex
                },
                next_search_turn: None,
            });
        }

        if self.geometry.is_usable() && !self.reader_frame_ready() {
            return Some(BackgroundSchedule {
                work: BackgroundWork::Render,
                next_search_turn: None,
            });
        }

        let search_pending = self.mode.content_is_reading()
            && self.geometry.is_usable()
            && self.search.as_ref().is_some_and(SearchSession::has_work);
        let line_pending = !self.document.line_index_complete();
        if search_pending && (!line_pending || self.search_turn) {
            return Some(BackgroundSchedule {
                work: BackgroundWork::Search,
                next_search_turn: Some(false),
            });
        }
        line_pending.then_some(BackgroundSchedule {
            work: BackgroundWork::LineIndex,
            next_search_turn: Some(true),
        })
    }

    fn viewport_request_is_ready(&self, request: ViewportRequest) -> bool {
        if self.cached_viewport_location(request).is_some() {
            return true;
        }
        if matches!(
            request,
            ViewportRequest::Move {
                target,
                delta: RowDelta::Forward(_),
                ..
            } if self.anchor_is_row_start && target == self.anchor
        ) {
            return true;
        }
        match request {
            ViewportRequest::End => self.document.line_index_complete(),
            _ => self
                .document
                .line_index_covers(request.target(self.document.source_end())),
        }
    }

    fn cached_viewport_location(&self, request: ViewportRequest) -> Option<LocatedViewport> {
        let height = self.geometry.body_height()?;
        let layout = self.layout.as_ref()?;
        let (target, delta) = request.locator_parameters(self.document.source_end(), height);
        self.row_neighborhood.locate_target(
            layout.row_cache_key(),
            self.document.source_start(),
            self.document.source_end(),
            target,
            delta,
            height,
        )
    }

    pub(super) fn background_work(&self) -> Option<BackgroundWork> {
        self.background_schedule().map(|schedule| schedule.work)
    }

    #[cfg(test)]
    pub(super) fn has_background_work(&self) -> bool {
        self.background_work().is_some()
    }

    #[cfg(test)]
    pub(super) fn pending_highlight_cursors(&self) -> Option<(usize, usize, Option<usize>)> {
        self.search
            .as_ref()
            .and_then(SearchSession::pending_highlight_cursors)
    }

    #[cfg(test)]
    pub(super) fn published_highlight_count(&self) -> usize {
        let Some(key) = self.current_search_highlight_key() else {
            return 0;
        };
        self.search
            .as_ref()
            .map_or(0, |search| search.highlight_ranges(key).len())
    }

    pub(super) fn advance_background(&mut self) -> Result<bool, TutError> {
        let Some(schedule) = self.background_schedule() else {
            return Ok(false);
        };
        if let Some(search_turn) = schedule.next_search_turn {
            self.search_turn = search_turn;
        }
        match schedule.work {
            BackgroundWork::LineIndex => self.advance_line_index(),
            BackgroundWork::Viewport => self.advance_viewport_locator(),
            BackgroundWork::Render => self.advance_render(),
            BackgroundWork::Search => self.advance_search(),
        }
    }

    fn advance_viewport_locator(&mut self) -> Result<bool, TutError> {
        let request = self
            .viewport_request
            .expect("viewport work retains its request");
        if let Some(located) = self.cached_viewport_location(request) {
            self.document.validate()?;
            return Ok(self.finish_viewport_request(request, located));
        }
        if self.locator.is_none() {
            let height = self.geometry.body_height().expect("usable geometry");
            let (target, delta) = request.locator_parameters(self.document.source_end(), height);
            let row_bound =
                source_row_bound(self.document.source_start(), self.document.source_end());
            let target_is_known_row_start =
                request.is_move() && self.anchor_is_row_start && target == self.anchor;
            self.locator = Some(if target_is_known_row_start {
                ViewportLocator::from_row_start(target, delta, height, row_bound)?
            } else {
                ViewportLocator::new(target, delta, height, row_bound)?
            });
        }

        let located = {
            let layout = self.layout.as_ref().expect("usable geometry has a layout");
            let mut reader = self.document.reader(&mut self.document_cache);
            self.locator
                .as_mut()
                .expect("viewport locator was initialized")
                .advance(layout, &mut reader, &mut self.row_neighborhood)?
        };
        let Some(located) = located else {
            return Ok(false);
        };

        self.document.validate()?;
        Ok(self.finish_viewport_request(request, located))
    }

    fn finish_viewport_request(
        &mut self,
        request: ViewportRequest,
        located: LocatedViewport,
    ) -> bool {
        self.cancel_render_scan();
        self.viewport_request = None;
        self.locator = None;
        self.anchor = located.anchor;
        self.anchor_is_row_start = true;
        self.follow_end = request.follows_end(located.at_end);
        if request.is_move() {
            self.start_queued_move();
        }
        // Completing a request always changes the published foreground status, even when the
        // requested motion clamps to the existing anchor. The redraw clears that status.
        true
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
        let key = self
            .current_search_highlight_key()
            .expect("search work runs against a cached render body");
        let spans = match &self.render_body {
            RenderBody::Cached(cache) => &cache.rows.spans,
            RenderBody::Empty | RenderBody::Scanning(_) => {
                unreachable!("search work runs against a cached render body")
            }
        };
        let Some(search) = self.search.as_mut() else {
            return Ok(false);
        };
        let mut reader = self.document.reader(&mut self.search_cache);
        if !search.is_searching() {
            return search.advance_highlights(&mut reader, key, |index| {
                let source = spans[index].source;
                SearchRange::new(source.start(), source.end())
                    .expect("render spans retain nonempty source ranges")
            });
        }
        let step = search.advance(&mut reader)?;
        let mut changed = step.changed();
        if let Some(selected) = step.jump()
            && self.viewport_request.is_none()
            && !self.follow_end
        {
            changed |= self.schedule_search_jump(selected);
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
        if self.terminal_too_small()
            && !matches!(action, Action::Resize(_) | Action::Interrupt | Action::Quit)
        {
            return Ok(Outcome::Unchanged);
        }
        if action == Action::Interrupt {
            return Ok(Outcome::Interrupt);
        }
        if action == Action::Quit {
            return Ok(Outcome::Quit);
        }

        let reading = matches!(self.mode, Mode::Content(ContentMode::Reading));
        let editing = matches!(self.mode, Mode::Content(ContentMode::SearchInput { .. }));
        let content = matches!(self.mode, Mode::Content(_));
        let helping = self.mode.is_help();
        let changed = match action {
            Action::Resize(geometry) => self.resize(geometry),
            Action::RepeatDigit(digit) if reading => self.insert_repeat_digit(digit),
            Action::RepeatBackspace if reading => self.backspace_repeat(),
            Action::RepeatCancel if reading => self.cancel_repeat(),
            Action::LineDown if reading => self.move_rows_repeated(true, 1),
            Action::LineUp if reading => self.move_rows_repeated(false, 1),
            Action::PageDown if reading => self.move_rows_repeated(true, self.page_amount()),
            Action::PageUp if reading => self.move_rows_repeated(false, self.page_amount()),
            Action::HalfPageDown if reading => {
                self.move_rows_repeated(true, self.half_page_amount())
            }
            Action::HalfPageUp if reading => {
                self.move_rows_repeated(false, self.half_page_amount())
            }
            Action::DocumentStart if reading => self.document_start(),
            Action::DocumentEnd if reading => self.document_end(),
            Action::BeginSearch if reading => self.begin_search(),
            Action::SearchInsert(character) if editing => self.insert_search(character)?,
            Action::SearchBackspace if editing => self.backspace_search(),
            Action::SearchRecall if editing => self.recall_search()?,
            Action::SearchClearDraft if editing => self.clear_search_draft(),
            Action::SearchCommit if editing => self.commit_search()?,
            Action::SearchCancel if editing => self.cancel_search(),
            Action::SearchCancel if reading => self.cancel_committed_search(),
            Action::NextMatch if reading => self.select_match_repeated(true)?,
            Action::PreviousMatch if reading => self.select_match_repeated(false)?,
            Action::ShowHelp if content => self.show_help(),
            Action::DismissHelp if helping => self.dismiss_help(),
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

    fn insert_repeat_digit(&mut self, digit: u8) -> bool {
        if digit > 9 {
            return false;
        }
        let Some(repeat) = self.repeat.as_mut() else {
            let Some(value) = (digit != 0)
                .then(|| NonZeroU16::new(u16::from(digit)))
                .flatten()
            else {
                return false;
            };
            self.repeat = Some(RepeatStatus {
                value,
                limit_hit: false,
            });
            return true;
        };

        let next = u32::from(repeat.value.get()) * 10 + u32::from(digit);
        if next > u32::from(MAX_REPEAT_COUNT) {
            let changed = !repeat.limit_hit;
            repeat.limit_hit = true;
            return changed;
        }
        repeat.value = NonZeroU16::new(
            u16::try_from(next).expect("bounded repeat counts fit unsigned 16-bit integers"),
        )
        .expect("appending a repeat digit preserves a nonzero count");
        repeat.limit_hit = false;
        true
    }

    fn backspace_repeat(&mut self) -> bool {
        let Some(repeat) = self.repeat.as_mut() else {
            return false;
        };
        if repeat.limit_hit {
            repeat.limit_hit = false;
            return true;
        }
        let previous = repeat.value.get() / 10;
        if let Some(value) = NonZeroU16::new(previous) {
            repeat.value = value;
        } else {
            self.repeat = None;
        }
        true
    }

    fn cancel_repeat(&mut self) -> bool {
        self.repeat.take().is_some()
    }

    fn take_repeat_count(&mut self) -> (usize, bool) {
        self.repeat
            .take()
            .map_or((1, false), |repeat| (usize::from(repeat.value.get()), true))
    }

    fn move_rows_repeated(&mut self, downward: bool, amount: usize) -> bool {
        let (repeat, consumed) = self.take_repeat_count();
        let amount = amount
            .checked_mul(repeat)
            .expect("bounded repeat counts and terminal heights fit viewport row counts");
        self.move_rows(downward, amount) || consumed
    }

    fn resize(&mut self, geometry: Geometry) -> bool {
        let geometry_changed = self.geometry != geometry;
        if !geometry_changed {
            return !geometry.is_usable() && self.cancel_repeat();
        }
        self.render_body = RenderBody::Empty;
        if let Some(search) = &mut self.search {
            search.invalidate_highlights();
        }
        if self.geometry.content_width() != geometry.content_width() {
            self.row_neighborhood.clear();
            self.anchor_is_row_start = self.anchor == self.document.source_start();
        }
        self.geometry = geometry;
        self.locator = None;

        if !geometry.is_usable() {
            self.cancel_repeat();
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
        let canceled_search = self.cancel_search_motion();
        if downward && self.follow_end && self.viewport_request.is_none() {
            return canceled_search;
        }
        let canceled_render = self.cancel_render_scan();

        let amount = i64::try_from(amount).expect("viewport row counts fit in i64");
        let delta = if downward { amount } else { -amount };
        if self.viewport_request.is_some_and(ViewportRequest::is_move) {
            self.queued_rows = self.queued_rows.saturating_add(delta);
            return true;
        }

        let canceled_viewport = self.cancel_viewport_request();
        self.queued_rows = delta;
        let scheduled = self.start_queued_move();
        canceled_render || canceled_search || canceled_viewport || scheduled
    }

    fn document_start(&mut self) -> bool {
        let canceled_repeat = self.cancel_repeat();
        let source_start = self.document.source_start();
        let anchor_changed = self.anchor != source_start;
        let canceled_render = anchor_changed && self.cancel_render_scan();
        let canceled_viewport = self.cancel_viewport_request();
        let canceled_search = self.cancel_search_motion();
        let changed = canceled_render
            || canceled_viewport
            || canceled_search
            || anchor_changed
            || self.follow_end
            || canceled_repeat;
        self.anchor = source_start;
        self.anchor_is_row_start = true;
        self.follow_end = false;
        changed
    }

    fn document_end(&mut self) -> bool {
        let canceled_repeat = self.cancel_repeat();
        let canceled_search = self.cancel_search_motion();
        if self.viewport_request == Some(ViewportRequest::End) {
            return canceled_search || canceled_repeat;
        }
        if self.follow_end && self.viewport_request.is_none() {
            return canceled_search || canceled_repeat;
        }
        self.cancel_render_scan();
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
        self.cancel_render_scan();
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

    fn cancel_search_motion(&mut self) -> bool {
        self.search
            .as_mut()
            .is_some_and(SearchSession::cancel_motion)
    }

    fn begin_search(&mut self) -> bool {
        self.cancel_repeat();
        if !matches!(self.viewport_request, Some(ViewportRequest::Reflow { .. })) {
            self.cancel_viewport_request();
        }
        self.cancel_search_motion();
        if let Some(search) = &mut self.search {
            search.invalidate_highlights();
        }
        self.mode = Mode::Content(ContentMode::SearchInput {
            draft: String::new(),
            limit_hit: false,
        });
        true
    }

    fn show_help(&mut self) -> bool {
        self.cancel_repeat();
        let Mode::Content(return_to) =
            std::mem::replace(&mut self.mode, Mode::Content(ContentMode::Reading))
        else {
            return false;
        };
        self.mode = Mode::Help { return_to };
        true
    }

    fn dismiss_help(&mut self) -> bool {
        let Mode::Help { return_to } =
            std::mem::replace(&mut self.mode, Mode::Content(ContentMode::Reading))
        else {
            return false;
        };
        self.mode = Mode::Content(return_to);
        true
    }

    fn insert_search(&mut self, character: char) -> Result<bool, TutError> {
        let Mode::Content(ContentMode::SearchInput { draft, limit_hit }) = &mut self.mode else {
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
        let Mode::Content(ContentMode::SearchInput { draft, limit_hit }) = &mut self.mode else {
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

    fn recall_search(&mut self) -> Result<bool, TutError> {
        let Mode::Content(ContentMode::SearchInput { draft, .. }) = &self.mode else {
            return Ok(false);
        };
        if !draft.is_empty() {
            return Ok(false);
        }
        let Some(search) = &self.search else {
            return Ok(false);
        };

        let query = search.query();
        let mut recalled = String::new();
        #[cfg(test)]
        if self.fail_search_recall_reserve {
            return Err(TutError::Allocation("search query"));
        }
        recalled
            .try_reserve_exact(query.len())
            .map_err(|_| TutError::Allocation("search query"))?;
        recalled.push_str(query);

        let Mode::Content(ContentMode::SearchInput { draft, limit_hit }) = &mut self.mode else {
            unreachable!("search recall preserves search-input mode");
        };
        *draft = recalled;
        *limit_hit = false;
        Ok(true)
    }

    fn clear_search_draft(&mut self) -> bool {
        let Mode::Content(ContentMode::SearchInput { draft, limit_hit }) = &mut self.mode else {
            return false;
        };
        let changed = !draft.is_empty() || *limit_hit;
        draft.clear();
        *limit_hit = false;
        changed
    }

    fn cancel_search(&mut self) -> bool {
        self.mode = Mode::Content(ContentMode::Reading);
        true
    }

    fn cancel_committed_search(&mut self) -> bool {
        if self.search.is_none() {
            return false;
        }
        if matches!(self.viewport_request, Some(ViewportRequest::Search { .. })) {
            self.viewport_request = None;
            self.locator = None;
        }
        self.search = None;
        true
    }

    fn commit_search(&mut self) -> Result<bool, TutError> {
        let Mode::Content(ContentMode::SearchInput { draft, .. }) =
            std::mem::replace(&mut self.mode, Mode::Content(ContentMode::Reading))
        else {
            return Ok(false);
        };

        if draft.is_empty() {
            self.search = None;
            return Ok(true);
        }

        let mut reader = self.document.reader(&mut self.search_cache);
        reader.validate()?;
        self.search = SearchSession::new(&reader, draft, self.anchor)?;
        self.follow_end = false;
        self.search_turn = true;
        Ok(true)
    }

    fn select_match_repeated(&mut self, forward: bool) -> Result<bool, TutError> {
        let (count, consumed) = self.take_repeat_count();
        let changed = self.select_match(
            forward,
            u16::try_from(count).expect("repeat counts fit unsigned 16-bit integers"),
        )?;
        Ok(changed || consumed)
    }

    fn select_match(&mut self, forward: bool, count: u16) -> Result<bool, TutError> {
        let Some(search) = self.search.as_mut() else {
            return Ok(false);
        };
        let mut reader = self.document.reader(&mut self.search_cache);
        if !search.request_navigation(&mut reader, forward, count)? {
            return Ok(false);
        }
        self.cancel_viewport_request();
        self.follow_end = false;
        Ok(true)
    }

    fn schedule_search_jump(&mut self, selected: SearchRange) -> bool {
        let request = ViewportRequest::Search {
            target: selected.start(),
        };
        let changed =
            self.cancel_render_scan() || self.viewport_request != Some(request) || self.follow_end;
        self.viewport_request = Some(request);
        self.locator = None;
        self.queued_rows = 0;
        self.follow_end = false;
        changed
    }

    fn advance_render(&mut self) -> Result<bool, TutError> {
        let key = self.render_key();
        if self.current_render_cache().is_some() {
            return Ok(false);
        }

        let previous = std::mem::replace(&mut self.render_body, RenderBody::Empty);
        let job = match previous {
            RenderBody::Scanning(job) if job.key == key => job,
            RenderBody::Cached(cache) if cache.geometry == key.geometry => {
                match RenderRowsBuilder::reuse_with_limit(
                    cache.rows,
                    self.render_row_capacity(),
                    self.render_limit(),
                ) {
                    Ok(rows) => self.new_render_job(key, RenderAttempt::Reused, rows)?,
                    Err(error) if retryable_reused_render_error(&error) => {
                        let fresh = self.new_fresh_render_job(key)?;
                        self.render_body = RenderBody::Scanning(fresh);
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                }
            }
            RenderBody::Empty | RenderBody::Cached(_) | RenderBody::Scanning(_) => {
                self.new_fresh_render_job(key)?
            }
        };

        let RenderJob {
            key,
            attempt,
            scanner,
        } = job;
        let advance = {
            let mut reader = self.document.reader(&mut self.document_cache);
            let mut meter = ProjectedScanMeter::standard();
            scanner.advance(&mut reader, &mut meter)
        };
        match advance {
            Ok(ProjectedScanAdvance::Pending(scanner)) => {
                self.render_body = RenderBody::Scanning(RenderJob {
                    key,
                    attempt,
                    scanner,
                });
                Ok(false)
            }
            Ok(ProjectedScanAdvance::Complete { result, sink }) => {
                self.document.validate()?;
                let rows = sink.finish();
                debug_assert_eq!(rows.len(), result.rows);
                let at_end = result.next.is_none();
                let viewport = Viewport {
                    visible_rows: result.rows,
                    first_visible_start: key.anchor,
                    visible_end: result.end,
                };
                self.render_body = RenderBody::Cached(RenderViewportCache {
                    geometry: key.geometry,
                    anchor: key.anchor,
                    viewport,
                    at_end,
                    rows,
                });
                Ok(true)
            }
            Err(error)
                if attempt == RenderAttempt::Reused && retryable_reused_render_error(&error) =>
            {
                let fresh = self.new_fresh_render_job(key)?;
                self.render_body = RenderBody::Scanning(fresh);
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn render_row_capacity(&self) -> usize {
        let height = usize::from(
            self.geometry
                .body_height()
                .expect("render work requires usable geometry")
                .get(),
        );
        height.min(source_row_bound(self.document.source_start(), self.document.source_end()).get())
    }

    #[cfg(not(test))]
    const fn render_limit(&self) -> usize {
        MAX_VISIBLE_RENDER_BYTES
    }

    #[cfg(test)]
    const fn render_limit(&self) -> usize {
        self.render_limit
    }

    fn new_fresh_render_job(&mut self, key: RenderKey) -> Result<RenderJob, TutError> {
        let rows = RenderRowsBuilder::with_limit(self.render_row_capacity(), self.render_limit())?;
        self.new_render_job(key, RenderAttempt::Fresh, rows)
    }

    fn new_render_job(
        &mut self,
        key: RenderKey,
        attempt: RenderAttempt,
        rows: RenderRowsBuilder,
    ) -> Result<RenderJob, TutError> {
        let reader = self.document.reader(&mut self.document_cache);
        let scanner = self
            .layout
            .as_ref()
            .expect("render work retains a viewport layout")
            .start_visible_scan(&reader, key.anchor, rows)?;
        Ok(RenderJob {
            key,
            attempt,
            scanner,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderRowsCheckpoint {
    text: usize,
    spans: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderStorage {
    text: usize,
    spans: usize,
    rows: usize,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct RenderReserveAttempts {
    text: usize,
    spans: usize,
    rows: usize,
}

#[cfg(test)]
impl RenderReserveAttempts {
    const ZERO: Self = Self {
        text: 0,
        spans: 0,
        rows: 0,
    };
}

impl RenderStorage {
    fn bytes(self) -> Option<usize> {
        let spans = self.spans.checked_mul(size_of::<RenderSpan>())?;
        let rows = self.rows.checked_mul(size_of::<RenderRowRange>())?;
        self.text.checked_add(spans)?.checked_add(rows)
    }

    fn require(self, limit: usize) -> Result<Self, TutError> {
        match self.bytes() {
            Some(bytes) if bytes <= limit => Ok(self),
            Some(_) | None => Err(TutError::VisibleRenderTooLarge { limit }),
        }
    }
}

fn planned_render_capacity(length: usize, capacity: usize, additional: usize) -> Option<usize> {
    let required = length.checked_add(additional)?;
    if required <= capacity {
        return Some(capacity);
    }
    if capacity == 0 {
        return Some(required);
    }
    Some(required.max(capacity.checked_mul(2)?))
}

struct RenderRowsBuilder {
    text: String,
    spans: Vec<RenderSpan>,
    rows: Vec<RenderRowRange>,
    row_start: RenderRowsCheckpoint,
    limit: usize,
    #[cfg(test)]
    reserve_attempts: RenderReserveAttempts,
}

#[cfg(test)]
struct RenderedViewportRows {
    rows: RenderRows,
    visible_rows: usize,
    visible_end: SourceOffset,
}

#[cfg(test)]
fn project_render_rows_with_limit(
    document: &Document,
    cache: &mut DocumentCache,
    layout: &ViewportLayout,
    anchor: SourceOffset,
    row_capacity: usize,
    reusable: Option<RenderRows>,
    limit: usize,
) -> Result<RenderedViewportRows, TutError> {
    let Some(reusable) = reusable else {
        let rows = RenderRowsBuilder::with_limit(row_capacity, limit)?;
        return project_render_rows_once(document, cache, layout, anchor, rows);
    };

    let reused = RenderRowsBuilder::reuse_with_limit(reusable, row_capacity, limit)
        .and_then(|rows| project_render_rows_once(document, cache, layout, anchor, rows));
    match reused {
        Ok(rendered) => Ok(rendered),
        Err(error) => retry_reused_render(error, || {
            let rows = RenderRowsBuilder::with_limit(row_capacity, limit)?;
            project_render_rows_once(document, cache, layout, anchor, rows)
        }),
    }
}

#[cfg(test)]
fn project_render_rows_once(
    document: &Document,
    cache: &mut DocumentCache,
    layout: &ViewportLayout,
    anchor: SourceOffset,
    mut rows: RenderRowsBuilder,
) -> Result<RenderedViewportRows, TutError> {
    let mut reader = document.reader(cache);
    let (visible_rows, visible_end) =
        layout.project_visible_rows(&mut reader, anchor, &mut rows)?;
    Ok(RenderedViewportRows {
        rows: rows.finish(),
        visible_rows,
        visible_end,
    })
}

#[cfg(test)]
fn retry_reused_render<T>(
    error: TutError,
    fresh: impl FnOnce() -> Result<T, TutError>,
) -> Result<T, TutError> {
    if retryable_reused_render_error(&error) {
        fresh()
    } else {
        Err(error)
    }
}

fn retryable_reused_render_error(error: &TutError) -> bool {
    match error {
        TutError::VisibleRenderTooLarge { .. } => true,
        TutError::Allocation(context) => matches!(
            *context,
            "visible row text" | "visible row spans" | "visible rows"
        ),
        _ => false,
    }
}

impl RenderRowsBuilder {
    fn reuse_with_limit(
        mut rows: RenderRows,
        row_capacity: usize,
        limit: usize,
    ) -> Result<Self, TutError> {
        rows.storage().require(limit)?;
        rows.clear();
        let mut builder = Self {
            text: rows.text,
            spans: rows.spans,
            rows: rows.rows,
            row_start: RenderRowsCheckpoint { text: 0, spans: 0 },
            limit,
            #[cfg(test)]
            reserve_attempts: RenderReserveAttempts::ZERO,
        };
        if builder.rows.capacity() < row_capacity {
            RenderStorage {
                rows: row_capacity,
                ..builder.storage()
            }
            .require(builder.limit)?;
            #[cfg(test)]
            {
                builder.reserve_attempts.rows += 1;
            }
            builder
                .rows
                .try_reserve_exact(row_capacity)
                .map_err(|_| TutError::Allocation("visible rows"))?;
            builder.storage().require(builder.limit)?;
        }
        Ok(builder)
    }

    fn with_limit(row_capacity: usize, limit: usize) -> Result<Self, TutError> {
        RenderStorage {
            text: 0,
            spans: 0,
            rows: row_capacity,
        }
        .require(limit)?;
        let mut rows = Vec::new();
        rows.try_reserve_exact(row_capacity)
            .map_err(|_| TutError::Allocation("visible rows"))?;
        let builder = Self {
            text: String::new(),
            spans: Vec::new(),
            rows,
            row_start: RenderRowsCheckpoint { text: 0, spans: 0 },
            limit,
            #[cfg(test)]
            reserve_attempts: RenderReserveAttempts {
                text: 0,
                spans: 0,
                rows: 1,
            },
        };
        builder.storage().require(limit)?;
        Ok(builder)
    }

    fn finish(self) -> RenderRows {
        debug_assert_eq!(self.row_start.text, self.text.len());
        debug_assert_eq!(self.row_start.spans, self.spans.len());
        debug_assert!(self.storage().require(self.limit).is_ok());
        RenderRows {
            text: self.text,
            spans: self.spans,
            rows: self.rows,
            #[cfg(test)]
            reserve_attempts: self.reserve_attempts,
        }
    }

    fn storage(&self) -> RenderStorage {
        RenderStorage {
            text: self.text.capacity(),
            spans: self.spans.capacity(),
            rows: self.rows.capacity(),
        }
    }

    fn planned_storage(
        &self,
        text: usize,
        spans: usize,
        rows: usize,
    ) -> Result<RenderStorage, TutError> {
        let planned = RenderStorage {
            text: planned_render_capacity(self.text.len(), self.text.capacity(), text)
                .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?,
            spans: planned_render_capacity(self.spans.len(), self.spans.capacity(), spans)
                .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?,
            rows: planned_render_capacity(self.rows.len(), self.rows.capacity(), rows)
                .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?,
        };
        planned.require(self.limit)
    }

    fn require_storage(&self, storage: RenderStorage) -> Result<(), TutError> {
        storage.require(self.limit).map(|_| ())
    }

    fn reserve_to(&mut self, planned: RenderStorage) -> Result<(), TutError> {
        if planned.text > self.text.capacity() {
            let additional = planned
                .text
                .checked_sub(self.text.len())
                .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?;
            #[cfg(test)]
            {
                self.reserve_attempts.text += 1;
            }
            self.text
                .try_reserve_exact(additional)
                .map_err(|_| TutError::Allocation("visible row text"))?;
            self.require_storage(RenderStorage {
                text: self.text.capacity(),
                ..planned
            })?;
        }
        if planned.spans > self.spans.capacity() {
            let additional = planned
                .spans
                .checked_sub(self.spans.len())
                .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?;
            #[cfg(test)]
            {
                self.reserve_attempts.spans += 1;
            }
            self.spans
                .try_reserve_exact(additional)
                .map_err(|_| TutError::Allocation("visible row spans"))?;
            self.require_storage(RenderStorage {
                text: self.text.capacity(),
                spans: self.spans.capacity(),
                rows: planned.rows,
            })?;
        }
        if planned.rows > self.rows.capacity() {
            let additional = planned
                .rows
                .checked_sub(self.rows.len())
                .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?;
            #[cfg(test)]
            {
                self.reserve_attempts.rows += 1;
            }
            self.rows
                .try_reserve_exact(additional)
                .map_err(|_| TutError::Allocation("visible rows"))?;
        }
        self.storage().require(self.limit)?;
        Ok(())
    }

    fn reserve(&mut self, text: usize, spans: usize, rows: usize) -> Result<(), TutError> {
        let planned = self.planned_storage(text, spans, rows)?;
        self.reserve_to(planned)
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
        let pending = PendingRenderSpan::new(atom);
        let text = pending
            .text_len()
            .ok_or(TutError::VisibleRenderTooLarge { limit: self.limit })?;
        self.reserve(text, 1, 0)?;
        let storage = self.storage();
        let span = pending.append_to(&mut self.text);
        self.spans.push(span);
        debug_assert_eq!(self.storage(), storage);
        Ok(())
    }

    fn finish_row(&mut self, through: Self::Checkpoint, carry_tail: bool) -> Result<(), TutError> {
        debug_assert!(through.text >= self.row_start.text && through.text <= self.text.len());
        debug_assert!(through.spans >= self.row_start.spans && through.spans <= self.spans.len());
        self.reserve(0, 0, 1)?;
        let storage = self.storage();
        if !carry_tail {
            self.text.truncate(through.text);
            self.spans.truncate(through.spans);
        }
        self.rows.push(RenderRowRange {
            text: self.row_start.text..through.text,
            spans: self.row_start.spans..through.spans,
        });
        self.row_start = through;
        debug_assert_eq!(self.storage(), storage);
        Ok(())
    }
}

pub(super) struct MatchCursor<'a> {
    ranges: &'a [SearchRange],
    next: usize,
    spanning: Option<SearchRange>,
    current: Option<SearchRange>,
}

impl<'a> MatchCursor<'a> {
    const fn new(ranges: &'a [SearchRange], current: Option<SearchRange>) -> Self {
        Self {
            ranges,
            next: 0,
            spanning: None,
            current,
        }
    }

    pub(super) fn role_for(&mut self, atom: GraphemeRange) -> Highlight {
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
    use std::{fs, io::Cursor, path::Path};

    use tempfile::tempdir;

    use super::*;
    use crate::layout::DisplayAtoms;

    const BACKGROUND_STEP_LIMIT: usize = 100_000;

    #[derive(Debug, Clone, Copy)]
    enum ViewportRequestCase {
        Move,
        End,
        Search,
        Reflow,
    }

    impl ViewportRequestCase {
        const ALL: [Self; 4] = [Self::Move, Self::End, Self::Search, Self::Reflow];

        fn request(self, app: &mut App) -> ViewportRequest {
            let target = SourceOffset::new(35);
            match self {
                Self::Move => {
                    app.anchor = target;
                    app.anchor_is_row_start = false;
                    ViewportRequest::Move {
                        target,
                        delta: RowDelta::Forward(1),
                        follow_end: FollowEndPolicy::AtEnd,
                    }
                }
                Self::End => ViewportRequest::End,
                Self::Search => ViewportRequest::Search { target },
                Self::Reflow => ViewportRequest::Reflow { target },
            }
        }
    }

    fn reader(text: &str, columns: u16, rows: u16) -> App {
        let mut app = app_from_text(Path::new("/tmp/book.txt"), text.to_owned());
        app.update(Action::Resize(Geometry::new(columns, rows)))
            .unwrap();
        settle_frame(&mut app);
        app
    }

    fn projected_atom(text: &str) -> ProjectedAtom<'_> {
        DisplayAtoms::new(text)
            .next()
            .expect("test text contains a grapheme")
            .project(
                DisplayColumn::ZERO,
                ContentWidth::new(u16::MAX).expect("maximum terminal width is nonzero"),
            )
            .expect("test graphemes are not line feeds")
    }

    fn highlighted_row(state: &RenderState<'_>, index: usize) -> Vec<(String, Highlight)> {
        let mut cursor = state.rows.highlight_cursor();
        for (row_index, row) in state.rows.iter().enumerate() {
            let spans = row
                .spans
                .iter()
                .map(|span| {
                    (
                        row.span_text(span).to_owned(),
                        cursor.role_for(span.source()),
                    )
                })
                .collect();
            if row_index == index {
                return spans;
            }
        }
        panic!("rendered row {index} is missing");
    }

    fn submit(app: &mut App, query: &str) {
        app.update(Action::BeginSearch).unwrap();
        for character in query.chars() {
            app.update(Action::SearchInsert(character)).unwrap();
        }
        app.update(Action::SearchCommit).unwrap();
    }

    fn enter_repeat(app: &mut App, digits: &str) {
        for digit in digits.chars() {
            app.update(Action::RepeatDigit(
                u8::try_from(digit.to_digit(10).expect("repeat fixtures use digits"))
                    .expect("decimal digits fit u8"),
            ))
            .unwrap();
        }
    }

    fn commit(app: &mut App, query: &str) {
        submit(app, query);
        settle(app);
    }

    fn settle(app: &mut App) {
        settle_count(app);
    }

    fn settle_frame(app: &mut App) {
        for _ in 0..BACKGROUND_STEP_LIMIT {
            if app.reader_frame_ready() {
                return;
            }
            app.advance_background().unwrap();
        }
        panic!("render work exceeded the test step limit");
    }

    fn settle_viewport(app: &mut App) {
        for _ in 0..BACKGROUND_STEP_LIMIT {
            if app.viewport_request.is_none() {
                return;
            }
            app.advance_background().unwrap();
        }
        panic!("viewport work exceeded the test step limit");
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

    fn published_boundary(app: &mut App) -> Option<ViewportBoundary> {
        app.render_state().unwrap().boundary
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
        let row_bound = source_row_bound(app.document.source_start(), app.document.source_end());
        let mut locator = ViewportLocator::new(target, delta, height, row_bound).unwrap();
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

    fn cache_location(app: &mut App, target: SourceOffset, delta: RowDelta) -> LocatedViewport {
        let height = app.geometry.body_height().unwrap();
        let row_bound = source_row_bound(app.document.source_start(), app.document.source_end());
        let mut locator = ViewportLocator::new(target, delta, height, row_bound).unwrap();
        for _ in 0..BACKGROUND_STEP_LIMIT {
            let layout = app.layout.as_ref().unwrap();
            let mut reader = app.document.reader(&mut app.document_cache);
            if let Some(located) = locator
                .advance(layout, &mut reader, &mut app.row_neighborhood)
                .unwrap()
            {
                return located;
            }
        }
        panic!("viewport cache priming exceeded the test step limit");
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
    fn completed_frames_distinguish_visual_viewport_boundaries() {
        let mut plain = reader("a", 16, 4);
        let mut trailing_line_feed = reader("\n", 16, 4);
        let plain_cache = plain.current_render_cache().unwrap();
        let trailing_cache = trailing_line_feed.current_render_cache().unwrap();

        assert_eq!(plain_cache.viewport, trailing_cache.viewport);
        assert!(plain_cache.at_end);
        assert!(!trailing_cache.at_end);
        assert_eq!(published_boundary(&mut plain), Some(ViewportBoundary::All));
        assert_eq!(
            published_boundary(&mut trailing_line_feed),
            Some(ViewportBoundary::Top)
        );

        for (text, rows, expected) in [
            ("", 4, ViewportBoundary::All),
            ("a", 4, ViewportBoundary::All),
            ("\n", 4, ViewportBoundary::Top),
            ("\n", 5, ViewportBoundary::All),
            ("\u{feff}a\r\n", 4, ViewportBoundary::Top),
            ("\u{feff}a\r\n", 5, ViewportBoundary::All),
        ] {
            let mut app = reader(text, 16, rows);
            assert_eq!(
                published_boundary(&mut app),
                Some(expected),
                "text={text:?}, rows={rows}"
            );
        }

        trailing_line_feed.update(Action::DocumentEnd).unwrap();
        settle(&mut trailing_line_feed);
        assert_eq!(
            published_boundary(&mut trailing_line_feed),
            Some(ViewportBoundary::End)
        );
    }

    #[test]
    fn soft_wrapped_boundaries_follow_navigation_and_resize() {
        let mut app = reader(&"x".repeat(80), 16, 5);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::Top));

        app.update(Action::LineDown).unwrap();
        settle(&mut app);
        assert_eq!(published_boundary(&mut app), None);

        app.update(Action::DocumentEnd).unwrap();
        settle(&mut app);
        assert!(app.follow_end);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::End));

        app.update(Action::Resize(Geometry::new(80, 5))).unwrap();
        settle(&mut app);
        assert!(app.follow_end);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::All));

        app.update(Action::Resize(Geometry::new(16, 5))).unwrap();
        settle(&mut app);
        assert!(app.follow_end);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::End));

        app.update(Action::DocumentStart).unwrap();
        settle(&mut app);
        assert!(!app.follow_end);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::Top));
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
    fn revisiting_document_end_on_a_long_line_uses_no_grapheme_scans() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2 + 17);
        let mut app = reader(&text, 16, 7);
        app.update(Action::DocumentEnd).unwrap();
        settle(&mut app);
        let end_anchor = app.anchor;

        app.update(Action::DocumentStart).unwrap();
        app.document_cache.reset_metrics();
        app.update(Action::DocumentEnd).unwrap();

        assert!(app.advance_background().unwrap());
        assert_eq!(app.anchor, end_anchor);
        assert!(app.follow_end);
        assert!(app.viewport_request.is_none());
        assert!(app.locator.is_none());
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), 0);
        assert_eq!(app.document_cache.metrics().grapheme_window_calls(), 0);
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
    fn absolute_row_cache_matches_the_locator_for_every_viewport_request() {
        let text = "x".repeat(83);
        for case in ViewportRequestCase::ALL {
            let mut app = reader(&text, 16, 7);
            let request = case.request(&mut app);
            let height = app.geometry.body_height().unwrap();
            let (target, delta) = request.locator_parameters(app.document.source_end(), height);
            let expected = cache_location(&mut app, target, delta);
            app.viewport_request = Some(request);
            app.locator = None;
            app.document_cache.reset_metrics();

            app.advance_viewport_locator().unwrap();

            assert_eq!(app.anchor, expected.anchor, "case={case:?}");
            assert_eq!(
                app.follow_end,
                request.follows_end(expected.at_end),
                "case={case:?}"
            );
            assert!(app.viewport_request.is_none(), "case={case:?}");
            assert!(app.locator.is_none(), "case={case:?}");
            assert_eq!(
                app.document_cache.metrics().grapheme_emissions(),
                0,
                "case={case:?}"
            );
        }
    }

    #[test]
    fn active_locators_reuse_row_edges_observed_by_an_earlier_step() {
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), 16, 7);
        let target = SourceOffset::from_usize(SOURCE_WINDOW_BYTES);
        let request = ViewportRequest::Reflow { target };
        let height = app.geometry.body_height().unwrap();
        let (_, delta) = request.locator_parameters(app.document.source_end(), height);
        let expected = cache_location(&mut app, target, delta);
        app.viewport_request = Some(request);
        let row_bound = source_row_bound(app.document.source_start(), app.document.source_end());
        app.locator = Some(ViewportLocator::new(target, delta, height, row_bound).unwrap());
        app.document_cache.reset_metrics();

        assert!(app.advance_viewport_locator().unwrap());

        assert_eq!(app.anchor, expected.anchor);
        assert!(app.viewport_request.is_none());
        assert!(app.locator.is_none());
        assert_eq!(app.document_cache.metrics().window_calls(), 0);
        assert_eq!(app.document_cache.metrics().grapheme_window_calls(), 0);
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), 0);
    }

    #[test]
    fn deep_reflow_remains_background_while_cached_moves_and_searches_are_immediate() {
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
        advance_until(&mut app, |app| app.current_match().is_some());
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Search { .. })
        ));
        app.document_cache.reset_metrics();
        assert!(app.advance_background().unwrap());
        assert_eq!(app.anchor, SourceOffset::from_usize(131_020));
        assert!(app.viewport_request.is_none());
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), 0);
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
        settle_viewport(&mut app);

        assert_eq!(app.anchor, anchor.checked_add(width).unwrap());
        let height = usize::from(app.geometry.body_height().unwrap().get());
        assert_eq!(
            app.document_cache.metrics().grapheme_emissions(),
            (height + 1) * (width + 1)
        );
    }

    #[test]
    fn warm_forward_frontier_scans_only_one_tail_row() {
        let width = 16usize;
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), width as u16, 7);
        let anchor = SourceOffset::from_usize(width * 2_000);
        app.anchor = anchor;
        app.document_cache = DocumentCache::default();

        app.update(Action::LineDown).unwrap();
        settle_viewport(&mut app);
        assert_eq!(app.anchor, anchor.checked_add(width).unwrap());

        app.document_cache.reset_metrics();
        app.update(Action::LineDown).unwrap();
        assert!(app.advance_background().unwrap());

        assert_eq!(app.anchor, anchor.checked_add(width * 2).unwrap());
        assert!(app.viewport_request.is_none());
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), width + 1);
    }

    #[test]
    fn known_forward_rows_advance_ahead_of_the_partial_line_index() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("forward-ahead-of-index.txt");
        fs::write(&path, "x".repeat(SOURCE_WINDOW_BYTES * 3)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        let anchor = SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2);
        app.anchor = anchor;
        app.anchor_is_row_start = true;
        app.row_neighborhood.clear();
        assert!(!app.document.line_index_covers(anchor));

        app.update(Action::LineDown).unwrap();

        assert_eq!(app.background_work(), Some(BackgroundWork::Viewport));
        assert!(app.advance_background().unwrap());
        assert_eq!(app.anchor, anchor.checked_add(16).unwrap());
        assert!(!app.document.line_index_covers(anchor));

        app.row_neighborhood.clear();
        app.update(Action::LineUp).unwrap();
        assert_eq!(app.background_work(), Some(BackgroundWork::LineIndex));
    }

    #[test]
    fn cached_backward_rows_advance_ahead_of_the_partial_line_index() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("cached-ahead-of-index.txt");
        fs::write(&path, "x".repeat(SOURCE_WINDOW_BYTES * 3)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        let target = SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2);
        let request = ViewportRequest::Move {
            target,
            delta: RowDelta::Backward(1),
            follow_end: FollowEndPolicy::Never,
        };
        let expected = cache_location(&mut app, target, RowDelta::Backward(1));
        assert!(!app.document.line_index_covers(target));
        app.anchor = target;
        app.anchor_is_row_start = true;
        app.viewport_request = Some(request);
        app.document_cache.reset_metrics();

        assert_eq!(app.background_work(), Some(BackgroundWork::Viewport));
        assert!(app.advance_background().unwrap());

        assert_eq!(app.anchor, expected.anchor);
        assert!(app.viewport_request.is_none());
        assert_eq!(app.document_cache.metrics().window_calls(), 0);
        assert_eq!(app.document_cache.metrics().grapheme_window_calls(), 0);
    }

    #[test]
    fn known_forward_rows_validate_files_before_advancing() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-forward-row.txt");
        fs::write(&path, "x".repeat(SOURCE_WINDOW_BYTES * 3)).unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        let anchor = SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2);
        app.anchor = anchor;
        app.anchor_is_row_start = true;
        app.row_neighborhood.clear();
        app.update(Action::LineDown).unwrap();
        assert_eq!(app.background_work(), Some(BackgroundWork::Viewport));

        fs::write(path, "y".repeat(SOURCE_WINDOW_BYTES * 3)).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
        assert_eq!(app.anchor, anchor);
        assert!(app.viewport_request.is_some());
    }

    #[test]
    fn cached_forward_frontiers_validate_files_before_reuse() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-forward-frontier.txt");
        fs::write(&path, "x".repeat(SOURCE_WINDOW_BYTES * 3)).unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        let initial = SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2);
        app.anchor = initial;
        app.anchor_is_row_start = true;
        app.row_neighborhood.clear();

        app.update(Action::LineDown).unwrap();
        settle_viewport(&mut app);
        let warm_anchor = initial.checked_add(16).unwrap();
        assert_eq!(app.anchor, warm_anchor);

        app.update(Action::LineDown).unwrap();
        let request = app.viewport_request.unwrap();
        assert_eq!(app.cached_viewport_location(request), None);
        fs::write(path, "y".repeat(SOURCE_WINDOW_BYTES * 3)).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
        assert_eq!(app.anchor, warm_anchor);
        assert!(app.viewport_request.is_some());
    }

    #[test]
    fn resized_forward_frontiers_match_a_cold_layout_oracle() {
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), 16, 7);
        app.anchor = SourceOffset::from_usize(16 * 2_000);
        app.row_neighborhood.clear();
        app.update(Action::LineDown).unwrap();
        settle_viewport(&mut app);

        app.update(Action::Resize(Geometry::new(17, 7))).unwrap();
        let target = app.anchor;
        let expected = locate(&mut app, target, RowDelta::Forward(0));
        app.document_cache.reset_metrics();
        settle_viewport(&mut app);

        assert_eq!(app.anchor, expected.anchor);
        assert!(app.document_cache.metrics().grapheme_emissions() > 0);
    }

    #[test]
    fn recent_row_edges_make_repeated_long_line_navigation_constant_work() {
        let width = 16usize;
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), width as u16, 7);
        let initial = SourceOffset::from_usize(width * 2_000);
        app.anchor = initial;

        for _ in 0..100 {
            app.update(Action::LineDown).unwrap();
            settle_viewport(&mut app);
        }
        assert_eq!(app.anchor, initial.checked_add(width * 100).unwrap());

        app.document_cache.reset_metrics();
        for _ in 0..100 {
            app.update(Action::LineUp).unwrap();
            settle_viewport(&mut app);
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
    fn repeat_prefix_is_bounded_editable_and_explicitly_cancelable() {
        let mut app = reader("body", 16, 4);

        assert_eq!(
            app.update(Action::RepeatDigit(0)).unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(app.repeat_status(), None);
        enter_repeat(&mut app, "9999");
        assert_eq!(
            app.repeat_status(),
            Some(RepeatStatus {
                value: NonZeroU16::new(MAX_REPEAT_COUNT).unwrap(),
                limit_hit: false,
            })
        );

        assert_eq!(
            app.update(Action::RepeatDigit(0)).unwrap(),
            Outcome::Changed
        );
        assert_eq!(app.repeat_status().unwrap().value(), MAX_REPEAT_COUNT);
        assert!(app.repeat_status().unwrap().limit_hit());
        assert_eq!(
            app.update(Action::RepeatDigit(1)).unwrap(),
            Outcome::Unchanged
        );

        assert_eq!(
            app.update(Action::RepeatBackspace).unwrap(),
            Outcome::Changed
        );
        assert_eq!(app.repeat_status().unwrap().value(), MAX_REPEAT_COUNT);
        assert!(!app.repeat_status().unwrap().limit_hit());
        assert_eq!(
            app.update(Action::RepeatBackspace).unwrap(),
            Outcome::Changed
        );
        assert_eq!(app.repeat_status().unwrap().value(), 999);
        for expected in [99, 9] {
            app.update(Action::RepeatBackspace).unwrap();
            assert_eq!(app.repeat_status().unwrap().value(), expected);
        }
        app.update(Action::RepeatBackspace).unwrap();
        assert_eq!(app.repeat_status(), None);
        assert_eq!(
            app.update(Action::RepeatBackspace).unwrap(),
            Outcome::Unchanged
        );

        enter_repeat(&mut app, "12");
        assert_eq!(app.update(Action::RepeatCancel).unwrap(), Outcome::Changed);
        assert_eq!(app.repeat_status(), None);
    }

    #[test]
    fn counted_relative_moves_batch_into_the_existing_row_queue() {
        let text = (0..30).map(|line| format!("{line}\n")).collect::<String>();

        let mut lines = reader(&text, 16, 6);
        enter_repeat(&mut lines, "9");
        assert_eq!(lines.update(Action::LineDown).unwrap(), Outcome::Changed);
        assert_eq!(lines.queued_rows, 6);
        assert!(matches!(
            lines.viewport_request,
            Some(ViewportRequest::Move {
                delta: RowDelta::Forward(3),
                ..
            })
        ));
        settle(&mut lines);
        assert_eq!(lines.render_state().unwrap().current_line, Some(10));
        assert_eq!(lines.repeat_status(), None);

        let mut pages = reader(&text, 16, 6);
        enter_repeat(&mut pages, "2");
        pages.update(Action::PageDown).unwrap();
        settle(&mut pages);
        assert_eq!(pages.render_state().unwrap().current_line, Some(5));

        let mut halves = reader(&text, 16, 7);
        enter_repeat(&mut halves, "3");
        halves.update(Action::HalfPageDown).unwrap();
        settle(&mut halves);
        assert_eq!(halves.render_state().unwrap().current_line, Some(7));

        let mut resized_pages = reader(&text, 16, 6);
        enter_repeat(&mut resized_pages, "2");
        resized_pages
            .update(Action::Resize(Geometry::new(20, 7)))
            .unwrap();
        settle(&mut resized_pages);
        assert_eq!(resized_pages.repeat_status().unwrap().value(), 2);
        resized_pages.update(Action::PageDown).unwrap();
        settle(&mut resized_pages);
        assert_eq!(resized_pages.render_state().unwrap().current_line, Some(7));
    }

    #[test]
    fn counted_match_navigation_batches_and_wraps_in_both_directions() {
        let mut app = reader("x x x x x", 16, 4);
        commit(&mut app, "x");
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::ZERO);

        enter_repeat(&mut app, "3");
        app.update(Action::NextMatch).unwrap();
        assert_eq!(app.search.as_ref().unwrap().pending_navigation(), 3);
        settle(&mut app);
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(6));

        enter_repeat(&mut app, "4");
        app.update(Action::PreviousMatch).unwrap();
        assert_eq!(app.search.as_ref().unwrap().pending_navigation(), -4);
        settle(&mut app);
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(8));
        assert_eq!(app.repeat_status(), None);
    }

    #[test]
    fn absolute_modes_and_tiny_resize_clear_repeat_while_usable_resize_preserves_it() {
        let mut app = reader("0\n1\n2\n3\n4", 16, 6);
        enter_repeat(&mut app, "12");
        app.update(Action::Resize(Geometry::new(20, 6))).unwrap();
        assert_eq!(app.repeat_status().unwrap().value(), 12);

        app.update(Action::DocumentEnd).unwrap();
        assert_eq!(app.repeat_status(), None);
        settle(&mut app);
        enter_repeat(&mut app, "2");
        app.update(Action::DocumentStart).unwrap();
        assert_eq!(app.repeat_status(), None);

        enter_repeat(&mut app, "3");
        app.update(Action::BeginSearch).unwrap();
        assert_eq!(app.repeat_status(), None);
        assert!(matches!(
            app.mode(),
            Mode::Content(ContentMode::SearchInput { .. })
        ));
        app.update(Action::SearchCancel).unwrap();

        enter_repeat(&mut app, "4");
        app.update(Action::ShowHelp).unwrap();
        assert_eq!(app.repeat_status(), None);
        app.update(Action::DismissHelp).unwrap();

        enter_repeat(&mut app, "5");
        app.update(Action::Resize(Geometry::new(15, 6))).unwrap();
        assert!(app.terminal_too_small());
        assert_eq!(app.repeat_status(), None);
    }

    #[test]
    fn repeat_cancel_precedes_clearing_a_committed_search() {
        let mut app = reader("cat dog cat", 16, 4);
        commit(&mut app, "cat");
        enter_repeat(&mut app, "2");

        app.update(Action::RepeatCancel).unwrap();
        assert!(matches!(
            app.search_status(),
            SearchStatus::Committed { .. }
        ));
        app.update(Action::SearchCancel).unwrap();
        assert_eq!(app.search_status(), SearchStatus::None);
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
        assert_eq!(
            app.current_match().unwrap().end(),
            app.document.source_end()
        );
        assert!(!app.search.as_ref().unwrap().jump_pending());
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
        assert!(app.has_background_work());
        assert!(matches!(
            app.viewport_request,
            Some(ViewportRequest::Reflow { .. })
        ));
        settle_viewport(&mut app);
        assert!(app.viewport_request.is_none());
        app.update(Action::SearchCancel).unwrap();
        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(65_520));
    }

    #[test]
    fn help_is_drawable_while_reader_render_continues() {
        let mut app = app_from_text(Path::new("help-render.txt"), "x".repeat(4096));
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();
        assert!(!app.reader_frame_ready());
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));

        assert_eq!(app.update(Action::ShowHelp).unwrap(), Outcome::Changed);
        assert!(app.frame_ready());
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Help { q_closes: true }
        ));
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
        assert!(!app.advance_background().unwrap());
        assert!(matches!(app.mode, Mode::Help { .. }));

        assert_eq!(app.update(Action::DismissHelp).unwrap(), Outcome::Changed);
        assert!(matches!(app.mode, Mode::Content(ContentMode::Reading)));
        assert!(!app.frame_ready());
        settle_frame(&mut app);
        assert!(app.frame_ready());
    }

    #[test]
    fn pending_view_is_static_and_reader_readiness_stays_separate() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("資料e\u{301}.txt");
        fs::write(&path, "x".repeat(4096)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();
        assert!(!app.reader_frame_ready());
        assert!(!app.frame_ready());

        app.document_cache.reset_metrics();
        app.search_cache.reset_metrics();
        let stability_checks = app.document.stability_checks();
        let line_position_cache = app.line_position_cache;
        let pending_highlights = app.pending_highlight_cursors();

        let ViewState::Pending(state) = app.view_state().unwrap() else {
            panic!("an incomplete reader frame exposes only the static pending shell");
        };
        assert_eq!(state.filename, "資料é.txt");
        assert!(state.path.ends_with("資料é.txt"));
        assert_eq!(state.status, SearchStatus::None);

        assert_eq!(app.document_cache.metrics(), Default::default());
        assert_eq!(app.search_cache.metrics(), Default::default());
        assert_eq!(app.document.stability_checks(), stability_checks);
        assert_eq!(app.line_position_cache, line_position_cache);
        assert_eq!(app.pending_highlight_cursors(), pending_highlights);
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
    }

    #[test]
    fn partial_render_steps_do_not_publish_viewport_boundaries() {
        let mut app = app_from_text(Path::new("partial-boundary.txt"), "x".repeat(4097));
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();
        let mut incomplete_steps = 0;

        while !app.reader_frame_ready() {
            assert!(app.current_render_cache().is_none());
            assert!(matches!(app.view_state().unwrap(), ViewState::Pending(_)));
            let changed = app.advance_background().unwrap();
            if !app.reader_frame_ready() {
                assert!(!changed);
                incomplete_steps += 1;
            }
        }

        assert!(incomplete_steps > 1);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::Top));
    }

    #[test]
    fn foreground_activity_is_derived_from_existing_view_requests() {
        let mut app = reader("0\n1\n2\n3\n4\n", 16, 4);

        assert_eq!(app.update(Action::LineDown).unwrap(), Outcome::Changed);
        let ViewState::Reader(state) = app.view_state().unwrap() else {
            panic!("the old complete frame remains visible while locating a move");
        };
        assert_eq!(state.activity, Some(ViewActivity::PreparingView));

        settle(&mut app);
        assert_eq!(app.update(Action::DocumentEnd).unwrap(), Outcome::Changed);
        let ViewState::Reader(state) = app.view_state().unwrap() else {
            panic!("the old complete frame remains visible while locating the end");
        };
        assert_eq!(state.activity, Some(ViewActivity::GoingToEnd));

        settle(&mut app);
        let ViewState::Reader(state) = app.view_state().unwrap() else {
            panic!("settled readers expose a complete frame");
        };
        assert_eq!(state.activity, None);
    }

    #[test]
    fn passive_line_indexing_has_no_foreground_activity() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("passive-index.txt");
        fs::write(&path, "line\n".repeat(30_000)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 6))).unwrap();
        settle_frame(&mut app);

        assert_eq!(app.background_work(), Some(BackgroundWork::LineIndex));
        let ViewState::Reader(state) = app.view_state().unwrap() else {
            panic!("a complete reader frame remains drawable during passive indexing");
        };
        assert_eq!(state.activity, None);
    }

    #[test]
    fn clamped_motion_clears_its_foreground_activity() {
        let mut app = reader("only", 16, 4);
        assert_eq!(app.update(Action::LineUp).unwrap(), Outcome::Changed);
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Reader(RenderState {
                activity: Some(ViewActivity::PreparingView),
                ..
            })
        ));

        assert!(app.advance_background().unwrap());
        assert!(app.viewport_request.is_none());
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Reader(RenderState { activity: None, .. })
        ));
    }

    #[test]
    fn help_round_trip_preserves_search_drafts_and_reader_state() {
        let mut app = reader("alpha beta alpha", 20, 4);
        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('α')).unwrap();
        app.update(Action::SearchInsert('?')).unwrap();

        assert_eq!(app.update(Action::ShowHelp).unwrap(), Outcome::Changed);
        assert!(matches!(
            &app.mode,
            Mode::Help {
                return_to: ContentMode::SearchInput { draft, limit_hit: false }
            } if draft == "α?"
        ));
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Help { q_closes: false }
        ));
        assert_eq!(app.update(Action::LineDown).unwrap(), Outcome::Unchanged);

        assert_eq!(app.update(Action::DismissHelp).unwrap(), Outcome::Changed);
        assert!(matches!(
            &app.mode,
            Mode::Content(ContentMode::SearchInput { draft, limit_hit: false })
                if draft == "α?"
        ));
        assert_eq!(app.anchor, SourceOffset::ZERO);
    }

    #[test]
    fn help_view_does_not_invalidate_search_highlights() {
        let mut app = reader("cat cat", 16, 4);
        commit(&mut app, "cat");
        app.render_state().unwrap();
        assert!(app.pending_highlight_cursors().is_some());
        let pending = app.pending_highlight_cursors();

        app.update(Action::ShowHelp).unwrap();
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Help { q_closes: true }
        ));
        assert_eq!(app.pending_highlight_cursors(), pending);
        app.update(Action::DismissHelp).unwrap();

        settle(&mut app);
        app.render_state().unwrap();
        let published = app.published_highlight_count();
        assert!(published > 0);
        app.update(Action::ShowHelp).unwrap();
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Help { q_closes: true }
        ));
        assert_eq!(app.published_highlight_count(), published);
    }

    #[test]
    fn help_uses_the_underlying_mode_for_background_policy() {
        let mut app = reader("0\n1\n2\n3\n4\n", 16, 4);
        app.update(Action::LineDown).unwrap();
        assert!(app.viewport_request.is_some());
        app.update(Action::ShowHelp).unwrap();
        settle_viewport(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(2));
        assert!(matches!(
            app.mode,
            Mode::Help {
                return_to: ContentMode::Reading
            }
        ));

        app.update(Action::DismissHelp).unwrap();
        submit(&mut app, "4");
        assert!(app.search.as_ref().unwrap().has_work());
        app.update(Action::BeginSearch).unwrap();
        app.update(Action::ShowHelp).unwrap();
        assert!(matches!(
            app.mode,
            Mode::Help {
                return_to: ContentMode::SearchInput { .. }
            }
        ));
        assert!(!matches!(
            app.background_work(),
            Some(BackgroundWork::Viewport | BackgroundWork::Search)
        ));
        assert!(app.search.as_ref().unwrap().has_work());
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

        assert_eq!(app.current_match().unwrap().start(), SourceOffset::ZERO);
        assert_eq!(
            app.anchor,
            SourceOffset::from_usize(text.len() - text.len().rem_euclid(16))
        );
        assert!(app.follow_end);
    }

    #[test]
    fn document_start_cancels_viewport_and_search_work_together() {
        let mut app = reader("cat cat", 16, 4);
        commit(&mut app, "cat");
        let requested = {
            let mut reader = app.document.reader(&mut app.search_cache);
            app.search
                .as_mut()
                .unwrap()
                .request_navigation(&mut reader, true, 1)
                .unwrap()
        };
        assert!(requested);
        app.viewport_request = Some(ViewportRequest::Reflow { target: app.anchor });

        assert_eq!(app.update(Action::DocumentStart).unwrap(), Outcome::Changed);
        assert!(app.viewport_request.is_none());
        assert!(!app.search.as_ref().unwrap().is_searching());
    }

    #[test]
    fn pending_viewport_row_scans_are_discarded_by_cancel_and_resize() {
        let text = "x".repeat(2_048);
        let mut canceled = reader(&text, u16::MAX, 4);
        canceled.update(Action::LineDown).unwrap();
        assert!(!canceled.advance_background().unwrap());
        assert!(canceled.locator.is_some());

        assert_eq!(
            canceled.update(Action::DocumentStart).unwrap(),
            Outcome::Changed
        );
        assert!(canceled.viewport_request.is_none());
        assert!(canceled.locator.is_none());

        let mut resized = reader(&text, u16::MAX, 4);
        resized.update(Action::LineDown).unwrap();
        assert!(!resized.advance_background().unwrap());
        assert!(resized.locator.is_some());

        assert_eq!(
            resized
                .update(Action::Resize(Geometry::new(u16::MAX - 1, 4)))
                .unwrap(),
            Outcome::Changed
        );
        assert!(resized.viewport_request.is_some());
        assert!(resized.locator.is_none());
        settle(&mut resized);
        assert!(resized.viewport_request.is_none());
        assert!(resized.locator.is_none());
    }

    #[test]
    fn tiny_geometry_freezes_state_except_for_control_and_resize() {
        let mut app = reader("line", 10, 3);
        assert!(app.terminal_too_small());
        assert_eq!(app.update(Action::BeginSearch).unwrap(), Outcome::Unchanged);
        assert!(matches!(app.mode(), Mode::Content(ContentMode::Reading)));
        assert_eq!(app.update(Action::Interrupt).unwrap(), Outcome::Interrupt);
        assert_eq!(app.update(Action::Quit).unwrap(), Outcome::Quit);
    }

    #[test]
    fn search_editing_is_transactional_and_grapheme_aware() {
        let mut app = reader("alpha beta alpha", 16, 5);
        commit(&mut app, "alpha");
        let first = app.current_match().unwrap();
        app.update(Action::NextMatch).unwrap();
        settle(&mut app);
        assert_ne!(app.current_match(), Some(first));
        app.update(Action::PreviousMatch).unwrap();
        settle(&mut app);
        assert_eq!(app.current_match(), Some(first));

        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('e')).unwrap();
        app.update(Action::SearchInsert('\u{301}')).unwrap();
        app.update(Action::SearchBackspace).unwrap();
        assert!(matches!(
            app.search_status(),
            SearchStatus::Draft { draft: "", .. }
        ));
        app.update(Action::SearchCancel).unwrap();
        assert_eq!(app.search.as_ref().unwrap().query(), "alpha");
    }

    #[test]
    fn current_search_recall_is_empty_only_transactional_and_source_idle() {
        let query = "e\u{301}終";
        let mut app = reader(&format!("zero {query} one {query}"), 20, 5);
        commit(&mut app, query);
        app.update(Action::NextMatch).unwrap();
        settle(&mut app);
        let selected = app.current_match();
        let anchor = app.anchor;

        app.update(Action::BeginSearch).unwrap();
        app.document_cache.reset_metrics();
        app.search_cache.reset_metrics();
        assert_eq!(app.update(Action::SearchRecall).unwrap(), Outcome::Changed);
        assert!(matches!(
            app.search_status(),
            SearchStatus::Draft {
                draft,
                limit_hit: false
            } if draft == query
        ));
        assert_eq!(
            app.update(Action::SearchRecall).unwrap(),
            Outcome::Unchanged
        );
        assert_eq!(app.anchor, anchor);
        assert_eq!(app.current_match(), selected);
        assert!(app.viewport_request.is_none());
        assert_eq!(app.queued_rows, 0);
        assert!(!app.has_background_work());
        assert!(!app.advance_background().unwrap());
        for metrics in [app.document_cache.metrics(), app.search_cache.metrics()] {
            assert_eq!(metrics.byte_window_calls(), 0);
            assert_eq!(metrics.window_calls(), 0);
            assert_eq!(metrics.grapheme_window_calls(), 0);
            assert_eq!(metrics.grapheme_emissions(), 0);
            assert_eq!(metrics.segmentation_runs(), 0);
        }

        assert_eq!(
            app.update(Action::SearchClearDraft).unwrap(),
            Outcome::Changed
        );
        app.update(Action::SearchInsert('x')).unwrap();
        assert_eq!(
            app.update(Action::SearchRecall).unwrap(),
            Outcome::Unchanged
        );
        app.update(Action::SearchCancel).unwrap();
        assert_eq!(app.search.as_ref().unwrap().query(), query);
        assert_eq!(app.current_match(), selected);

        let mut fresh = reader("body", 16, 4);
        fresh.update(Action::BeginSearch).unwrap();
        fresh.fail_search_recall_reserve = true;
        assert_eq!(
            fresh.update(Action::SearchRecall).unwrap(),
            Outcome::Unchanged
        );
    }

    #[test]
    fn recalled_exact_limit_queries_and_limit_notices_clear_in_one_action() {
        let mut query = "界".repeat((SEARCH_DRAFT_LIMIT_BYTES - 1) / "界".len());
        query.push('x');
        assert_eq!(query.len(), SEARCH_DRAFT_LIMIT_BYTES);
        let mut app = reader(&query, 16, 4);
        commit(&mut app, &query);

        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchRecall).unwrap();
        assert!(matches!(
            app.search_status(),
            SearchStatus::Draft {
                draft,
                limit_hit: false
            } if draft == query
        ));
        app.update(Action::SearchInsert('z')).unwrap();
        assert!(matches!(
            app.search_status(),
            SearchStatus::Draft {
                draft,
                limit_hit: true
            } if draft == query
        ));

        assert_eq!(
            app.update(Action::SearchClearDraft).unwrap(),
            Outcome::Changed
        );
        assert_eq!(
            app.search_status(),
            SearchStatus::Draft {
                draft: "",
                limit_hit: false,
            }
        );
        assert_eq!(
            app.update(Action::SearchClearDraft).unwrap(),
            Outcome::Unchanged
        );
        app.update(Action::SearchCancel).unwrap();
        assert_eq!(app.search.as_ref().unwrap().query(), query);
    }

    #[test]
    fn search_recall_allocation_failure_is_atomic() {
        let mut app = reader("alpha beta alpha", 16, 5);
        commit(&mut app, "alpha");
        app.update(Action::NextMatch).unwrap();
        settle(&mut app);
        let selected = app.current_match();
        let anchor = app.anchor;
        app.update(Action::BeginSearch).unwrap();
        let mode = app.mode.clone();
        let viewport_request = app.viewport_request;
        let queued_rows = app.queued_rows;
        app.document_cache.reset_metrics();
        app.search_cache.reset_metrics();
        app.fail_search_recall_reserve = true;

        assert!(matches!(
            app.update(Action::SearchRecall),
            Err(TutError::Allocation("search query"))
        ));
        assert_eq!(app.mode, mode);
        assert_eq!(app.search.as_ref().unwrap().query(), "alpha");
        assert_eq!(app.current_match(), selected);
        assert_eq!(app.anchor, anchor);
        assert_eq!(app.viewport_request, viewport_request);
        assert_eq!(app.queued_rows, queued_rows);
        assert!(!app.has_background_work());
        for metrics in [app.document_cache.metrics(), app.search_cache.metrics()] {
            assert_eq!(metrics.byte_window_calls(), 0);
            assert_eq!(metrics.window_calls(), 0);
            assert_eq!(metrics.grapheme_window_calls(), 0);
            assert_eq!(metrics.grapheme_emissions(), 0);
            assert_eq!(metrics.segmentation_runs(), 0);
        }
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
        assert_eq!(app.current_match(), None);
        assert!(app.has_background_work());

        app.advance_background().unwrap();
        assert_eq!(app.current_match(), None);
        assert_eq!(app.update(Action::SearchCancel).unwrap(), Outcome::Changed);
        assert_eq!(app.search_status(), SearchStatus::None);
        assert!(!app.has_background_work());
    }

    #[test]
    fn search_commit_reuses_the_visible_anchor_without_scanning_document_rows() {
        let mut app = reader(&"x".repeat(SOURCE_WINDOW_BYTES * 2), 16, 7);
        app.update(Action::LineDown).unwrap();
        settle(&mut app);
        let anchor = app.anchor;
        assert!(anchor > app.document.source_start());

        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('x')).unwrap();
        app.document_cache.reset_metrics();
        app.search_cache.reset_metrics();

        assert_eq!(app.update(Action::SearchCommit).unwrap(), Outcome::Changed);
        for metrics in [app.document_cache.metrics(), app.search_cache.metrics()] {
            assert_eq!(metrics.window_calls(), 0);
            assert_eq!(metrics.grapheme_window_calls(), 0);
            assert_eq!(metrics.grapheme_emissions(), 0);
            assert_eq!(metrics.segmentation_runs(), 0);
        }

        settle(&mut app);
        assert_eq!(app.current_match().unwrap().start(), anchor);
    }

    #[test]
    fn search_commit_rejects_changed_files_before_starting_the_session() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-search.txt");
        fs::write(&path, "cat").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('c')).unwrap();

        fs::write(path, "changed").unwrap();

        assert!(matches!(
            app.update(Action::SearchCommit),
            Err(TutError::Load(_))
        ));
        assert!(app.search.is_none());
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
        assert_eq!(app.current_match(), None);

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
        assert_eq!(app.current_match(), None);

        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle(&mut app);
        assert_eq!(
            app.current_match().unwrap().end(),
            app.document.source_end()
        );
    }

    #[test]
    fn early_search_results_are_selected_before_scanning_finishes() {
        let mut text = "needle".to_owned();
        text.push_str(&"x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2));
        let mut app = reader(&text, 16, 4);

        submit(&mut app, "needle");
        app.advance_background().unwrap();

        assert_eq!(
            app.current_match().unwrap().start(),
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
        let first = app.current_match().unwrap();

        app.update(Action::NextMatch).unwrap();
        assert_eq!(app.current_match(), Some(first));
        assert!(matches!(
            app.search_status(),
            SearchStatus::Committed {
                searching: true,
                ..
            }
        ));
        settle(&mut app);

        assert_eq!(
            app.current_match().unwrap().end(),
            app.document.source_end()
        );
    }

    #[test]
    fn a_new_query_replaces_pending_search_state() {
        let mut text = "alpha".to_owned();
        text.push_str(&"x".repeat(crate::document::SOURCE_WINDOW_BYTES * 2));
        text.push_str("beta");
        let mut app = reader(&text, 16, 4);

        submit(&mut app, "alpha");
        app.advance_background().unwrap();
        assert!(app.current_match().is_some());
        submit(&mut app, "beta");
        assert_eq!(app.current_match(), None);
        settle(&mut app);

        assert_eq!(app.search.as_ref().unwrap().query(), "beta");
        assert_eq!(
            app.current_match().unwrap().end(),
            app.document.source_end()
        );
    }

    #[test]
    fn equal_length_query_replacement_discards_all_derived_state() {
        let mut app = reader("cat dog dog cat", 16, 5);
        commit(&mut app, "cat");
        app.update(Action::NextMatch).unwrap();
        settle(&mut app);
        app.render_state().unwrap();
        settle(&mut app);
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(12));
        assert!(app.search.as_ref().unwrap().has_cached_block());

        submit(&mut app, "dog");
        let search = app.search.as_ref().unwrap();
        assert_eq!(search.query(), "dog");
        assert_eq!(search.current_match(), None);
        assert!(!search.has_cached_block());
        settle(&mut app);
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(4));

        app.update(Action::NextMatch).unwrap();
        settle(&mut app);
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(8));
    }

    #[test]
    fn one_byte_source_caps_render_rows_under_maximum_geometry() {
        let mut app = app_from_text(Path::new("one-byte.txt"), "\n".to_owned());
        app.update(Action::Resize(Geometry::new(16, 32_768)))
            .unwrap();

        assert_eq!(app.render_row_capacity(), 2);
        settle_frame(&mut app);

        let rows = &app.render_cache().unwrap().rows;
        assert_eq!(rows.len(), 2);
        assert!(rows.storage().rows >= app.render_row_capacity());
        assert_eq!(rows.reserve_attempts().rows, 1);
    }

    #[test]
    fn visible_render_budget_accepts_exact_capacity_and_rejects_the_next_atom_atomically() {
        let atom = projected_atom("a");
        let mut builder = RenderRowsBuilder::with_limit(1, usize::MAX).unwrap();
        let text = PendingRenderSpan::new(atom).text_len().unwrap();
        let planned = builder.planned_storage(text, 1, 0).unwrap();
        builder.reserve_to(planned).unwrap();
        let limit = builder.storage().bytes().unwrap();
        builder.limit = limit;

        let fill = builder.text.capacity().min(builder.spans.capacity());
        assert!(fill > 0);
        for _ in 0..fill {
            builder.push(atom).unwrap();
        }
        assert_eq!(builder.storage().bytes(), Some(limit));

        let before_text = builder.text.clone();
        let before_spans = builder.spans.clone();
        let before_rows = builder.rows.clone();
        let before_start = builder.row_start;
        let before_storage = builder.storage();
        let error = builder.push(atom).unwrap_err();

        assert!(matches!(
            error,
            TutError::VisibleRenderTooLarge { limit: actual } if actual == limit
        ));
        assert_eq!(builder.text, before_text);
        assert_eq!(builder.spans, before_spans);
        assert_eq!(builder.rows, before_rows);
        assert_eq!(builder.row_start, before_start);
        assert_eq!(builder.storage(), before_storage);
    }

    #[test]
    fn allocator_capacity_rounding_is_rechecked_against_the_aggregate_limit() {
        let planned = RenderStorage {
            text: 8,
            spans: 2,
            rows: 1,
        };
        let limit = planned.bytes().unwrap();
        assert!(planned.require(limit).is_ok());

        let rounded = RenderStorage {
            text: planned.text + 1,
            ..planned
        };
        assert!(matches!(
            rounded.require(limit),
            Err(TutError::VisibleRenderTooLarge { limit: actual }) if actual == limit
        ));
    }

    #[test]
    fn poisoned_reused_capacity_retries_with_fresh_storage() {
        let mut app = reader(&"x".repeat(16), 16, 4);
        let row_capacity = usize::from(app.geometry.body_height().unwrap().get());
        let layout = app.layout.as_ref().unwrap();
        let fresh = project_render_rows_with_limit(
            &app.document,
            &mut app.document_cache,
            layout,
            app.anchor,
            row_capacity,
            None,
            usize::MAX,
        )
        .unwrap();
        let fresh_storage = fresh.rows.storage();

        let mut retained_text = String::new();
        retained_text.try_reserve_exact(1024).unwrap();
        let mut retained_rows = Vec::new();
        retained_rows.try_reserve_exact(row_capacity).unwrap();
        let reusable = RenderRows {
            text: retained_text,
            spans: Vec::new(),
            rows: retained_rows,
            reserve_attempts: RenderReserveAttempts::ZERO,
        };
        let reused_storage = reusable.storage();
        let limit = reused_storage
            .bytes()
            .unwrap()
            .max(fresh_storage.bytes().unwrap());
        reused_storage.require(limit).unwrap();
        fresh_storage.require(limit).unwrap();
        assert!(matches!(
            RenderStorage {
                text: reused_storage.text,
                spans: fresh_storage.spans,
                rows: fresh_storage.rows,
            }
            .require(limit),
            Err(TutError::VisibleRenderTooLarge { limit: actual }) if actual == limit
        ));

        let actual = project_render_rows_with_limit(
            &app.document,
            &mut app.document_cache,
            layout,
            app.anchor,
            row_capacity,
            Some(reusable),
            limit,
        )
        .unwrap();

        assert_eq!(actual.rows, fresh.rows);
        assert_eq!(actual.visible_rows, fresh.visible_rows);
        assert_eq!(actual.visible_end, fresh.visible_end);
    }

    #[test]
    fn asynchronous_reuse_failures_schedule_one_later_fresh_attempt() {
        let mut app = reader(&"x".repeat(16), 16, 4);
        let row_capacity = app.render_row_capacity();
        let layout = app.layout.as_ref().unwrap();
        let fresh = project_render_rows_with_limit(
            &app.document,
            &mut app.document_cache,
            layout,
            app.anchor,
            row_capacity,
            None,
            usize::MAX,
        )
        .unwrap();
        let fresh_storage = fresh.rows.storage();

        let mut retained_text = String::new();
        retained_text.try_reserve_exact(1024).unwrap();
        let mut retained_rows = Vec::new();
        retained_rows.try_reserve_exact(row_capacity).unwrap();
        let reusable = RenderRows {
            text: retained_text,
            spans: Vec::new(),
            rows: retained_rows,
            reserve_attempts: RenderReserveAttempts::ZERO,
        };
        let reused_storage = reusable.storage();
        let limit = reused_storage
            .bytes()
            .unwrap()
            .max(fresh_storage.bytes().unwrap());
        app.render_limit = limit;
        app.render_body = RenderBody::Cached(RenderViewportCache {
            geometry: app.geometry,
            anchor: SourceOffset::new(1),
            viewport: Viewport {
                visible_rows: 1,
                first_visible_start: SourceOffset::new(1),
                visible_end: SourceOffset::new(1),
            },
            at_end: false,
            rows: reusable,
        });

        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
        assert!(!app.advance_background().unwrap());
        assert!(matches!(
            app.render_body,
            RenderBody::Scanning(RenderJob {
                attempt: RenderAttempt::Fresh,
                ..
            })
        ));
        assert!(!app.frame_ready());

        assert!(app.advance_background().unwrap());
        let cached = app.current_render_cache().unwrap();
        assert_eq!(cached.rows, fresh.rows);
        assert_eq!(cached.viewport.visible_rows, fresh.visible_rows);
        assert_eq!(cached.viewport.visible_end, fresh.visible_end);
    }

    #[test]
    fn asynchronous_fresh_failures_do_not_schedule_a_third_attempt() {
        let mut app = reader("x", 16, 4);
        app.render_limit = std::mem::size_of::<RenderRowRange>();
        match &mut app.render_body {
            RenderBody::Cached(cache) => cache.anchor = SourceOffset::new(1),
            RenderBody::Empty | RenderBody::Scanning(_) => panic!("test starts with cached rows"),
        }

        assert!(!app.advance_background().unwrap());
        assert!(matches!(
            app.render_body,
            RenderBody::Scanning(RenderJob {
                attempt: RenderAttempt::Fresh,
                ..
            })
        ));

        assert!(matches!(
            app.advance_background(),
            Err(TutError::VisibleRenderTooLarge { .. })
        ));
        assert!(matches!(app.render_body, RenderBody::Empty));
    }

    #[test]
    fn fresh_render_failures_are_returned_without_another_retry() {
        let mut fresh_attempts = 0;
        let error: Result<(), TutError> =
            retry_reused_render(TutError::VisibleRenderTooLarge { limit: 7 }, || {
                fresh_attempts += 1;
                Err(TutError::VisibleRenderTooLarge { limit: 11 })
            });

        assert_eq!(fresh_attempts, 1);
        assert!(matches!(
            error,
            Err(TutError::VisibleRenderTooLarge { limit: 11 })
        ));
        for context in ["visible row text", "visible row spans", "visible rows"] {
            assert!(retryable_reused_render_error(&TutError::Allocation(
                context
            )));
        }
        assert!(!retryable_reused_render_error(&TutError::Load(
            crate::error::LoadError::Allocation("file window")
        )));
        assert!(!retryable_reused_render_error(&TutError::Search(
            crate::error::SearchError::Allocation
        )));
        assert!(!retryable_reused_render_error(&TutError::Allocation(
            "search query"
        )));
    }

    #[test]
    fn discarded_provisional_tail_releases_length_but_not_capacity() {
        let mut builder = RenderRowsBuilder::with_limit(1, usize::MAX).unwrap();
        builder.push(projected_atom("a")).unwrap();
        let through = builder.checkpoint();
        builder.push(projected_atom("b")).unwrap();
        let storage = builder.storage();

        builder.finish_row(through, false).unwrap();

        assert_eq!(builder.text, "a");
        assert_eq!(builder.spans.len(), 1);
        assert_eq!(builder.rows.len(), 1);
        assert_eq!(builder.row_start, through);
        assert_eq!(builder.storage(), storage);
    }

    #[test]
    fn carried_provisional_tail_becomes_the_next_row_without_copying() {
        let mut builder = RenderRowsBuilder::with_limit(2, usize::MAX).unwrap();
        builder.push(projected_atom("é")).unwrap();
        let through = builder.checkpoint();
        builder.push(projected_atom("終")).unwrap();
        let text_pointer = builder.text.as_ptr();
        let spans_pointer = builder.spans.as_ptr();
        let spans = builder.spans.clone();
        let storage = builder.storage();

        builder.finish_row(through, true).unwrap();

        assert_eq!(builder.text, "é終");
        assert_eq!(builder.spans.len(), 2);
        assert_eq!(builder.row_start, through);
        assert_eq!(builder.text.as_ptr(), text_pointer);
        assert_eq!(builder.spans.as_ptr(), spans_pointer);
        assert_eq!(builder.spans, spans);
        assert_eq!(builder.storage(), storage);

        let end = builder.checkpoint();
        builder.finish_row(end, false).unwrap();
        let rows = builder.finish();
        assert_eq!(rows.spans[0].text, RenderTextRange::new(0, 2));
        assert_eq!(rows.spans[1].text, RenderTextRange::new(2, 5));
        assert_eq!(
            rows.iter().map(|row| row.text).collect::<Vec<_>>(),
            vec!["é", "終"]
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.span_text(&row.spans[0]))
                .collect::<Vec<_>>(),
            vec!["é", "終"]
        );
    }

    #[test]
    fn visible_render_budget_calculations_reject_integer_overflow() {
        assert_eq!(planned_render_capacity(usize::MAX, usize::MAX, 1), None);
        assert_eq!(
            planned_render_capacity(usize::MAX - 1, usize::MAX - 1, 1),
            None
        );
        let overflow = RenderStorage {
            text: 0,
            spans: usize::MAX,
            rows: 0,
        };
        assert_eq!(overflow.bytes(), None);
        assert!(matches!(
            overflow.require(MAX_VISIBLE_RENDER_BYTES),
            Err(TutError::VisibleRenderTooLarge {
                limit: MAX_VISIBLE_RENDER_BYTES
            })
        ));
        assert!(matches!(
            RenderRowsBuilder::with_limit(usize::MAX, MAX_VISIBLE_RENDER_BYTES),
            Err(TutError::VisibleRenderTooLarge {
                limit: MAX_VISIBLE_RENDER_BYTES
            })
        ));
    }

    #[test]
    fn normal_unicode_at_the_terminal_cell_limit_fits_the_visible_render_budget() {
        let cells = 512_usize * 1024;
        assert!(cells.is_power_of_two());
        let maximum_scalar_bytes = '\u{10ffff}'.len_utf8();
        let text = cells
            .checked_mul(DOTTED_CIRCLE.len() + maximum_scalar_bytes)
            .unwrap();
        let maximum_geometric_text_capacity = text.checked_mul(2).unwrap() - 1;
        let estimate = RenderStorage {
            text: maximum_geometric_text_capacity,
            spans: cells,
            rows: cells / usize::from(MIN_TERMINAL_COLUMNS),
        };

        assert_eq!(size_of::<RenderSpan>(), 32);
        assert!(MAX_VISIBLE_RENDER_BYTES <= u32::MAX as usize);
        assert!(MAX_TRANSIENT_RENDER_TEXT_BYTES <= u32::MAX as usize);
        assert_eq!(estimate.bytes(), Some(24 * 1024 * 1024 - 1));
        assert!(estimate.bytes().unwrap() < MAX_VISIBLE_RENDER_BYTES);
        assert!(estimate.require(MAX_VISIBLE_RENDER_BYTES).is_ok());
    }

    #[test]
    fn pathological_zero_width_rendering_is_bounded_by_injected_budget() {
        let mut cluster = String::new();
        cluster.extend(std::iter::repeat_n('\u{301}', 512));
        assert_eq!(cluster.len(), 1024);
        let atom = projected_atom(&cluster);
        let pending = PendingRenderSpan::new(atom);
        assert_eq!(
            pending.text_len(),
            Some(DOTTED_CIRCLE.len() + cluster.len())
        );

        let limit = 64 * 1024;
        let mut builder = RenderRowsBuilder::with_limit(1, limit).unwrap();
        let mut accepted = 0;
        loop {
            let before_text = builder.text.len();
            let before_spans = builder.spans.len();
            let before_storage = builder.storage();
            match builder.push(atom) {
                Ok(()) => {
                    accepted += 1;
                    assert!(builder.storage().bytes().unwrap() <= limit);
                }
                Err(TutError::VisibleRenderTooLarge { limit: actual }) => {
                    assert_eq!(actual, limit);
                    assert_eq!(builder.text.len(), before_text);
                    assert_eq!(builder.spans.len(), before_spans);
                    assert_eq!(builder.storage(), before_storage);
                    break;
                }
                Err(error) => panic!("unexpected render error: {}", error.message()),
            }
        }

        assert!(accepted > 0);
        assert!(accepted < 512 * 1024);
    }

    #[test]
    fn render_state_borrows_rows_and_marks_all_matches() {
        let mut app = reader("cat cat", 16, 4);
        commit(&mut app, "cat");
        let storage = {
            let state = app.render_state().unwrap();
            let row = highlighted_row(&state, 0);
            assert_eq!(row[0], ("c".to_owned(), Highlight::Current));
            assert_eq!(row[3], (" ".to_owned(), Highlight::None));
            assert_eq!(row[4], ("c".to_owned(), Highlight::None));
            state.rows.storage_identity()
        };
        settle(&mut app);
        let state = app.render_state().unwrap();
        assert_eq!(state.rows.storage_identity(), storage);
        let row = highlighted_row(&state, 0);
        assert_eq!(row[0], ("c".to_owned(), Highlight::Current));
        assert_eq!(row[4], ("c".to_owned(), Highlight::Match));
    }

    #[test]
    fn exact_cap_dense_frame_starts_without_target_visits_or_reservations() {
        let text = "x\0".repeat(131_072);
        let mut app = reader(&text, 4096, 67);
        commit(&mut app, "x");
        let search = app.search.as_ref().unwrap();
        assert_eq!(search.highlight_reserve_attempts(), 0);
        assert!(search.pending_highlight_cursors().is_none());

        {
            let state = app.render_state().unwrap();
            let span_count = state.rows.iter().map(|row| row.spans.len()).sum::<usize>();
            assert_eq!(span_count, crate::search::MAX_DISPLAY_HIGHLIGHT_RANGES);
            assert!(state.rows.ranges.is_empty());
        }
        let search = app.search.as_ref().unwrap();
        assert!(search.pending_highlight_cursors().is_some());
        assert_eq!(search.highlight_reserve_attempts(), 0);

        settle(&mut app);
        let state = app.render_state().unwrap();
        let mut cursor = state.rows.highlight_cursor();
        let mut current = 0;
        let mut matches = 0;
        let mut plain = 0;
        for row in state.rows.iter() {
            for span in row.spans {
                match cursor.role_for(span.source()) {
                    Highlight::Current => current += 1,
                    Highlight::Match => matches += 1,
                    Highlight::None => plain += 1,
                }
            }
        }
        assert_eq!((current, matches, plain), (1, 131_071, 131_072));
    }

    #[test]
    fn legal_dense_frames_disable_optional_highlights_without_hiding_the_current_match() {
        let text = "x\0".repeat(256_000);
        assert_eq!(text.len(), 512_000);
        let mut app = reader(&text, 4096, 128);
        commit(&mut app, "x");
        let viewport = app.current_render_cache().unwrap().viewport;
        let visible = viewport.first_visible_start..viewport.visible_end;

        {
            let state = app.render_state().unwrap();
            let span_count = state.rows.iter().map(|row| row.spans.len()).sum::<usize>();
            assert_eq!(span_count, 512_000);
            assert!(span_count > crate::search::MAX_DISPLAY_HIGHLIGHT_RANGES);
            assert!(state.rows.ranges.is_empty());
            let first = state.rows.get(0).unwrap().spans[0].source();
            assert_eq!(
                state.rows.highlight_cursor().role_for(first),
                Highlight::Current
            );
        }

        let search = app.search.as_ref().unwrap();
        assert!(search.highlights_disabled_for(&visible));
        assert_eq!(search.highlight_reserve_attempts(), 0);
        let state = app.render_state().unwrap();
        assert!(state.rows.ranges.is_empty());
        let first = state.rows.get(0).unwrap().spans[0].source();
        assert_eq!(
            state.rows.highlight_cursor().role_for(first),
            Highlight::Current
        );
    }

    #[test]
    fn rendering_segments_each_visible_grapheme_once() {
        let mut app = reader("a\nb\nc\nd\ne\noutside\n", 16, 8);
        app.document_cache = DocumentCache::default();
        app.render_body = RenderBody::Empty;
        app.document_cache.reset_metrics();
        settle_frame(&mut app);

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
    fn render_jobs_publish_only_complete_frames_after_bounded_atom_steps() {
        let mut app = app_from_text(Path::new("bounded-render.txt"), "x".repeat(4096));
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();

        let mut steps = 0;
        while !app.frame_ready() {
            app.document_cache.reset_metrics();
            assert_eq!(app.background_work(), Some(BackgroundWork::Render));
            let changed = app.advance_background().unwrap();
            steps += 1;
            assert!(app.document_cache.metrics().grapheme_emissions() <= 1024);
            assert!(
                app.document_cache
                    .metrics()
                    .grapheme_window_returned_bytes()
                    <= SOURCE_WINDOW_BYTES + 4
            );
            if !changed {
                assert!(app.render_cache().is_none());
                assert!(matches!(app.render_body, RenderBody::Scanning(_)));
            }
        }

        assert_eq!(steps, 5);
        let state = app.render_state().unwrap();
        assert_eq!(state.rows.len(), 1);
        assert_eq!(state.rows.get(0).unwrap().text.len(), 4096);
    }

    #[test]
    fn asynchronous_frames_match_synchronous_unicode_projection() {
        let cases = [
            (
                "\u{feff}\talpha\r\n界🙂\u{200d}↔\u{fe0f}🙂 e\u{301}\n",
                16,
                8,
            ),
            ("a b\tc\n\n中🙂\rfinal", 17, 7),
        ];

        for (text, columns, rows) in cases {
            let mut app = app_from_text(Path::new("unicode-render.txt"), text.to_owned());
            app.update(Action::Resize(Geometry::new(columns, rows)))
                .unwrap();
            let anchor = app.anchor;
            let row_capacity = app.render_row_capacity();
            let expected = project_render_rows_with_limit(
                &app.document,
                &mut app.document_cache,
                app.layout.as_ref().unwrap(),
                anchor,
                row_capacity,
                None,
                MAX_VISIBLE_RENDER_BYTES,
            )
            .unwrap();

            settle_frame(&mut app);

            let actual = app.current_render_cache().unwrap();
            assert_eq!(actual.rows, expected.rows);
            assert_eq!(actual.viewport.first_visible_start, anchor);
            assert_eq!(actual.viewport.visible_rows, expected.visible_rows);
            assert_eq!(actual.viewport.visible_end, expected.visible_end);
        }
    }

    #[test]
    fn maximum_graphemes_require_independent_bounded_render_steps() {
        const CLUSTERS: usize = 32;
        let cluster_bytes = usize::try_from(crate::document::MAX_FILE_BYTES).unwrap() / CLUSTERS;
        let mut cluster = String::from('é');
        cluster.extend(std::iter::repeat_n(
            '\u{301}',
            (cluster_bytes - 'é'.len_utf8()) / '\u{301}'.len_utf8(),
        ));
        assert_eq!(cluster.len(), cluster_bytes);
        let text = cluster.repeat(CLUSTERS);
        assert_eq!(
            text.len(),
            usize::try_from(crate::document::MAX_FILE_BYTES).unwrap()
        );
        let mut app = app_from_text(Path::new("oversized-render.txt"), text);
        app.update(Action::Resize(Geometry::new(32, 4))).unwrap();

        let mut steps = 0;
        let mut emissions = 0;
        let mut segmented_bytes = 0;
        while !app.frame_ready() {
            app.document_cache.reset_metrics();
            let changed = app.advance_background().unwrap();
            steps += 1;
            let metrics = app.document_cache.metrics();
            let step_emissions = metrics.grapheme_emissions();
            emissions += step_emissions;
            segmented_bytes += metrics.segmentation_advanced_bytes();
            assert!(step_emissions <= 1);
            assert!(
                metrics.grapheme_window_returned_bytes() <= cluster_bytes + SOURCE_WINDOW_BYTES + 4
            );
            if !changed {
                assert!(app.render_cache().is_none());
            }
        }

        assert!(steps >= CLUSTERS);
        assert_eq!(emissions, CLUSTERS);
        assert_eq!(segmented_bytes, crate::document::MAX_FILE_BYTES as usize);
        let row = app.render_state().unwrap().rows.get(0).unwrap();
        assert_eq!(row.spans.len(), CLUSTERS);
        assert_eq!(row.text, REPLACEMENT_CHARACTER.repeat(CLUSTERS));
    }

    #[test]
    fn navigation_and_resize_discard_pending_render_jobs() {
        let text = "x".repeat(4096);
        let mut moved = app_from_text(Path::new("move-render.txt"), text.clone());
        moved
            .update(Action::Resize(Geometry::new(4096, 4)))
            .unwrap();
        assert!(!moved.advance_background().unwrap());
        assert!(matches!(moved.render_body, RenderBody::Scanning(_)));

        moved.update(Action::LineDown).unwrap();
        assert!(matches!(moved.render_body, RenderBody::Empty));
        assert!(moved.viewport_request.is_some());

        let mut resized = app_from_text(Path::new("resize-render.txt"), text.clone());
        resized
            .update(Action::Resize(Geometry::new(4096, 4)))
            .unwrap();
        assert!(!resized.advance_background().unwrap());
        resized
            .update(Action::Resize(Geometry::new(2048, 4)))
            .unwrap();
        assert!(matches!(resized.render_body, RenderBody::Empty));
        assert_eq!(resized.background_work(), Some(BackgroundWork::Render));

        let mut ended = app_from_text(Path::new("end-render.txt"), text.clone());
        ended
            .update(Action::Resize(Geometry::new(4096, 4)))
            .unwrap();
        assert!(!ended.advance_background().unwrap());
        ended.update(Action::DocumentEnd).unwrap();
        assert!(matches!(ended.render_body, RenderBody::Empty));
        assert_eq!(ended.viewport_request, Some(ViewportRequest::End));

        let mut jumped = app_from_text(Path::new("search-jump-render.txt"), text);
        jumped
            .update(Action::Resize(Geometry::new(4096, 4)))
            .unwrap();
        assert!(!jumped.advance_background().unwrap());
        assert!(jumped.schedule_search_jump(
            SearchRange::new(SourceOffset::new(32), SourceOffset::new(33)).unwrap()
        ));
        assert!(matches!(jumped.render_body, RenderBody::Empty));
        assert_eq!(
            jumped.viewport_request,
            Some(ViewportRequest::Search {
                target: SourceOffset::new(32)
            })
        );
    }

    #[test]
    fn search_drafts_preserve_pending_render_jobs_for_the_same_frame() {
        let mut app = app_from_text(Path::new("search-render.txt"), "x".repeat(4096));
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();
        assert!(!app.advance_background().unwrap());
        let key = match &app.render_body {
            RenderBody::Scanning(job) => job.key,
            RenderBody::Empty | RenderBody::Cached(_) => panic!("render job is pending"),
        };

        app.update(Action::BeginSearch).unwrap();
        app.update(Action::SearchInsert('q')).unwrap();

        assert!(matches!(
            app.mode,
            Mode::Content(ContentMode::SearchInput { .. })
        ));
        assert!(matches!(
            &app.render_body,
            RenderBody::Scanning(job) if job.key == key
        ));
        assert_eq!(app.background_work(), Some(BackgroundWork::Render));
    }

    #[test]
    fn document_start_preserves_a_pending_frame_at_the_same_anchor() {
        let mut app = app_from_text(Path::new("start-render.txt"), "x".repeat(4096));
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();
        assert!(!app.advance_background().unwrap());

        assert_eq!(
            app.update(Action::DocumentStart).unwrap(),
            Outcome::Unchanged
        );
        assert!(matches!(app.render_body, RenderBody::Scanning(_)));

        let mut remaining = 0;
        while !app.frame_ready() {
            app.advance_background().unwrap();
            remaining += 1;
        }
        assert_eq!(remaining, 4);
    }

    #[test]
    fn repeated_rendering_reuses_the_unchanged_visible_body() {
        let mut app = reader("a\nb\nc\nd\ne\noutside\n", 16, 8);
        let first = app.render_state().unwrap().rows.storage_identity();
        let cached = app.render_cache().unwrap().rows.storage_identity();
        assert_eq!(first, cached);
        app.document_cache.reset_metrics();

        let second = app.render_state().unwrap().rows.storage_identity();

        assert_eq!(second, first);
        assert_eq!(app.document_cache.metrics().grapheme_emissions(), 0);
        assert_eq!(app.document_cache.metrics().grapheme_window_calls(), 0);
    }

    #[test]
    fn moved_viewports_reuse_render_storage_without_reserving() {
        let mut app = reader(&"x".repeat(1024), 16, 7);
        app.render_state().unwrap();
        let first = app.render_cache().unwrap();
        let (_, first_text, first_spans, first_rows) = first.rows.storage_identity();
        let first_storage = first.rows.storage();
        let first_anchor = app.anchor;
        assert!(first.rows.reserve_attempts().text > 0);
        assert!(first.rows.reserve_attempts().spans > 0);
        assert_eq!(first.rows.reserve_attempts().rows, 1);

        app.update(Action::LineDown).unwrap();
        settle(&mut app);
        assert_ne!(app.anchor, first_anchor);
        app.render_state().unwrap();

        let second = app.render_cache().unwrap();
        let (_, second_text, second_spans, second_rows) = second.rows.storage_identity();
        assert_eq!(
            (second_text, second_spans, second_rows),
            (first_text, first_spans, first_rows)
        );
        assert_eq!(second.rows.storage(), first_storage);
        assert_eq!(second.rows.reserve_attempts(), RenderReserveAttempts::ZERO);
    }

    #[test]
    fn reused_render_storage_grows_within_its_budget() {
        let cluster = "\u{301}".repeat(512);
        let mut app = reader(&format!("a\n{cluster}\n"), 16, 4);
        app.render_state().unwrap();
        let first_storage = app.render_cache().unwrap().rows.storage();

        app.update(Action::LineDown).unwrap();
        settle(&mut app);
        app.render_state().unwrap();

        let rows = &app.render_cache().unwrap().rows;
        assert!(rows.storage().text > first_storage.text);
        assert!(rows.reserve_attempts().text > 0);
        assert!(rows.storage().bytes().unwrap() <= MAX_VISIBLE_RENDER_BYTES);
    }

    #[test]
    fn geometry_changes_discard_render_storage() {
        let mut app = reader(&"x".repeat(4096), 127, 7);
        app.render_state().unwrap();
        let first_storage = app.render_cache().unwrap().rows.storage();

        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle(&mut app);
        app.render_state().unwrap();

        let rows = &app.render_cache().unwrap().rows;
        assert_eq!(rows.reserve_attempts().rows, 1);
        assert!(rows.storage().bytes().unwrap() < first_storage.bytes().unwrap());
        assert!(rows.storage().bytes().unwrap() <= MAX_VISIBLE_RENDER_BYTES);
    }

    #[test]
    fn search_input_redraws_reuse_the_cached_body_and_line_position() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("search-redraw.txt");
        fs::write(&path, "x".repeat(SOURCE_WINDOW_BYTES * 2)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
        app.update(Action::LineDown).unwrap();
        settle(&mut app);
        let (rows, line) = {
            let state = app.render_state().unwrap();
            (state.rows.storage_identity(), state.current_line)
        };
        app.document_cache.reset_metrics();

        app.update(Action::BeginSearch).unwrap();
        assert_eq!(app.render_state().unwrap().rows.storage_identity(), rows);
        for character in "query".chars() {
            app.update(Action::SearchInsert(character)).unwrap();
            let state = app.render_state().unwrap();
            assert_eq!(state.rows.storage_identity(), rows);
            assert_eq!(state.current_line, line);
        }

        let metrics = app.document_cache.metrics();
        assert_eq!(metrics.byte_window_calls(), 0);
        assert_eq!(metrics.window_calls(), 0);
        assert_eq!(metrics.grapheme_window_calls(), 0);
        assert_eq!(metrics.grapheme_emissions(), 0);
    }

    #[test]
    fn empty_tiny_views_reuse_complete_line_coordinates_without_reads() {
        let mut app = app_from_text(Path::new("empty.txt"), String::new());
        let first = app.render_state().unwrap();
        assert_eq!((first.current_line, first.total_lines), (Some(1), Some(1)));
        app.document_cache.reset_metrics();

        let second = app.render_state().unwrap();

        assert_eq!(
            (second.current_line, second.total_lines),
            (Some(1), Some(1))
        );
        let metrics = app.document_cache.metrics();
        assert_eq!(metrics.byte_window_calls(), 0);
        assert_eq!(metrics.window_calls(), 0);
        assert_eq!(metrics.grapheme_window_calls(), 0);
    }

    #[test]
    fn rendering_reads_one_grapheme_frontier_and_retains_atom_boundaries() {
        let mut app = reader(&"x".repeat(1024), 127, 4);
        app.document_cache = DocumentCache::with_window_bytes(128);
        app.render_body = RenderBody::Empty;
        app.document_cache.reset_metrics();
        settle_frame(&mut app);

        {
            let state = app.render_state().unwrap();
            let row = state.rows.get(0).unwrap();
            assert_eq!(row.text.len(), 127);
            assert_eq!(row.spans.len(), 127);
            assert_eq!(row.spans[0].cell_width, DisplayColumn::new(1));
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
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(6));
        assert_eq!(
            highlighted_row(&app.render_state().unwrap(), 1)[0].1,
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
        assert_eq!(app.current_match().unwrap().start(), SourceOffset::new(9));
        let state = app.render_state().unwrap();
        assert!((0..state.rows.len()).any(|index| {
            highlighted_row(&state, index)
                .iter()
                .any(|(_, highlight)| *highlight == Highlight::Current)
        }));
    }

    #[test]
    fn cached_file_rendering_still_rejects_in_place_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing.txt");
        fs::write(&path, "first").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle_frame(&mut app);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::All));

        fs::write(&path, "other").unwrap();

        assert!(matches!(app.render_state(), Err(TutError::Load(_))));
    }

    #[test]
    fn cached_file_frames_use_one_freshness_barrier_without_source_reads() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("stable-cached-frame.txt");
        fs::write(&path, "first\nsecond").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle_frame(&mut app);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::Top));
        app.document_cache.reset_metrics();

        let before = app.document.stability_checks();
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::Top));
        assert_eq!(app.document.stability_checks(), before + 1);
        assert_eq!(
            app.document_cache.metrics(),
            crate::document::DocumentMetrics::default()
        );

        fs::write(path, "other\ncontent").unwrap();
        let before = app.document.stability_checks();
        assert!(matches!(app.render_state(), Err(TutError::Load(_))));
        assert_eq!(app.document.stability_checks(), before + 1);
    }

    #[test]
    fn standalone_line_position_cache_hits_validate_the_document() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("standalone-line-position.txt");
        fs::write(&path, "first\nsecond").unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle_frame(&mut app);
        app.render_state().unwrap();
        let viewport = app.render_cache().unwrap().viewport;
        app.document_cache.reset_metrics();

        let before = app.document.stability_checks();
        assert_eq!(
            app.line_position_for(Some(viewport))
                .unwrap()
                .unwrap()
                .current(),
            1
        );
        assert_eq!(app.document.stability_checks(), before + 1);
        assert_eq!(
            app.document_cache.metrics(),
            crate::document::DocumentMetrics::default()
        );
    }

    #[test]
    fn standalone_line_position_misses_use_only_source_read_barriers() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("uncached-line-position.txt");
        fs::write(&path, "first\nsecond").unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        assert!(app.line_position_cache.is_none());

        let before = app.document.stability_checks();
        assert_eq!(app.line_position_for(None).unwrap().unwrap().current(), 1);

        assert_eq!(app.document.stability_checks(), before + 4);
        let metrics = app.document_cache.metrics();
        assert_eq!(metrics.byte_window_calls(), 1);
        assert_eq!(metrics.window_calls(), 1);
    }

    #[test]
    fn standard_input_cached_frames_do_not_check_file_stability() {
        let mut input = Cursor::new("first\nsecond");
        let document = crate::document::load_standard_input(&mut input).unwrap();
        let mut app = App::new(document);
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle_frame(&mut app);

        app.render_state().unwrap();
        app.render_state().unwrap();

        assert_eq!(app.document.stability_checks(), 0);
    }

    #[test]
    fn pending_file_rendering_rejects_changes_before_publishing_rows() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-pending-render.txt");
        fs::write(&path, "x".repeat(4096)).unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(4096, 4))).unwrap();

        assert!(!app.advance_background().unwrap());
        assert!(matches!(app.render_body, RenderBody::Scanning(_)));
        fs::write(path, "y".repeat(4096)).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
        assert!(matches!(app.render_body, RenderBody::Empty));
        assert!(!app.frame_ready());
    }

    #[test]
    fn recycled_file_rendering_still_rejects_in_place_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-recycled.txt");
        fs::write(&path, "x".repeat(128)).unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle_frame(&mut app);
        app.render_state().unwrap();

        app.update(Action::LineDown).unwrap();
        settle_viewport(&mut app);
        fs::write(path, "y".repeat(128)).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
        assert!(app.render_cache().is_none());
    }

    #[test]
    fn cached_line_positions_reject_file_changes_without_source_reads() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-line-position.txt");
        fs::write(&path, "first\nsecond").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        settle_frame(&mut app);
        let viewport = {
            let state = app.render_state().unwrap();
            assert_eq!(state.current_line, Some(1));
            app.render_cache().unwrap().viewport
        };
        app.document_cache.reset_metrics();

        fs::write(path, "changed contents").unwrap();

        assert!(matches!(
            app.line_position_for(Some(viewport)),
            Err(TutError::Load(_))
        ));
        let metrics = app.document_cache.metrics();
        assert_eq!(metrics.byte_window_calls(), 0);
        assert_eq!(metrics.window_calls(), 0);
    }

    #[test]
    fn line_position_cache_tracks_partial_coverage_and_index_completion() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("partial-line-position.txt");
        fs::write(&path, "a\n".repeat(40_000)).unwrap();
        let mut app = App::new(crate::document::load(path).unwrap());
        let offset = SourceOffset::new(2);
        let viewport = Some(Viewport {
            visible_rows: 1,
            first_visible_start: offset,
            visible_end: offset.checked_add(1).unwrap(),
        });

        assert!(!app.document.line_index_covers(offset));
        assert!(!app.document.line_index_complete());
        assert_eq!(app.line_position_for(viewport).unwrap(), None);
        assert_eq!(
            app.line_position_cache.unwrap().key,
            LinePositionCacheKey {
                offset,
                covered: false,
                complete: false,
            }
        );

        assert!(
            app.document
                .advance_line_index(&mut app.document_cache)
                .unwrap()
        );
        assert!(app.document.line_index_covers(offset));
        assert!(!app.document.line_index_complete());
        let covered = app.line_position_for(viewport).unwrap().unwrap();
        assert_eq!((covered.current(), covered.total()), (2, None));
        assert_eq!(
            app.line_position_cache.unwrap().key,
            LinePositionCacheKey {
                offset,
                covered: true,
                complete: false,
            }
        );

        while app
            .document
            .advance_line_index(&mut app.document_cache)
            .unwrap()
        {}
        assert!(app.document.line_index_complete());
        let complete = app.line_position_for(viewport).unwrap().unwrap();
        assert_eq!((complete.current(), complete.total()), (2, Some(40_001)));
        assert_eq!(
            app.line_position_cache.unwrap().key,
            LinePositionCacheKey {
                offset,
                covered: true,
                complete: true,
            }
        );
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
    fn absolute_row_cache_rejects_file_changes_for_every_viewport_request() {
        let directory = tempdir().unwrap();
        for case in ViewportRequestCase::ALL {
            let path = directory.path().join(format!("changing-{case:?}.txt"));
            fs::write(&path, "x".repeat(83)).unwrap();
            let mut app = App::new(crate::document::load(path.clone()).unwrap());
            app.update(Action::Resize(Geometry::new(16, 7))).unwrap();
            let request = case.request(&mut app);
            let height = app.geometry.body_height().unwrap();
            let (target, delta) = request.locator_parameters(app.document.source_end(), height);
            let expected = cache_location(&mut app, target, delta);
            let key = app.layout.as_ref().unwrap().row_cache_key();
            assert_eq!(
                app.row_neighborhood.locate_target(
                    key,
                    app.document.source_start(),
                    app.document.source_end(),
                    target,
                    delta,
                    height,
                ),
                Some(expected),
                "case={case:?}"
            );
            app.viewport_request = Some(request);
            app.locator = None;
            let old_anchor = app.anchor;
            let old_follow_end = app.follow_end;

            fs::write(&path, "y".repeat(83)).unwrap();

            assert!(
                matches!(app.advance_viewport_locator(), Err(TutError::Load(_))),
                "case={case:?}"
            );
            assert_eq!(app.anchor, old_anchor, "case={case:?}");
            assert_eq!(app.follow_end, old_follow_end, "case={case:?}");
            assert_eq!(app.viewport_request, Some(request), "case={case:?}");
            assert!(app.locator.is_none(), "case={case:?}");
        }
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
        assert!(app.search.as_ref().unwrap().has_cached_block());

        fs::write(&path, "dog dog dog").unwrap();
        app.update(Action::NextMatch).unwrap();

        assert!(matches!(app.advance_background(), Err(TutError::Load(_))));
    }

    #[test]
    fn cached_no_match_navigation_still_rejects_in_place_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-no-match.txt");
        fs::write(&path, "cat").unwrap();
        let mut app = App::new(crate::document::load(path.clone()).unwrap());
        app.update(Action::Resize(Geometry::new(16, 4))).unwrap();
        commit(&mut app, "dog");
        assert!(app.search.as_ref().unwrap().no_matches());

        fs::write(&path, "dog").unwrap();

        assert!(matches!(
            app.update(Action::NextMatch),
            Err(TutError::Load(_))
        ));
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
        settle_frame(&mut app);
        submit(&mut app, "needle");
        let first_frontier = SourceOffset::from_usize(SOURCE_WINDOW_BYTES);

        assert!(!app.document.line_index_covers(first_frontier));
        assert!(!app.search.as_ref().unwrap().index_complete());
        assert_eq!(app.background_work(), Some(BackgroundWork::Search));
        assert!(!app.advance_background().unwrap());
        assert!(!app.document.line_index_covers(first_frontier));
        assert_eq!(app.background_work(), Some(BackgroundWork::LineIndex));
        assert!(!app.advance_background().unwrap());
        assert!(app.document.line_index_covers(first_frontier));
        assert!(!app.search.as_ref().unwrap().index_complete());

        settle(&mut app);
        assert!(app.document.line_index_complete());
        assert!(app.search.as_ref().unwrap().index_complete());
        assert_eq!(
            app.current_match().unwrap().end(),
            app.document.source_end()
        );
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
        settle_frame(&mut app);

        let initial = app.render_state().unwrap();
        assert_eq!((initial.current_line, initial.total_lines), (Some(1), None));
        assert_eq!(initial.boundary, Some(ViewportBoundary::Top));
        assert!(app.has_background_work());

        let (advances, redraws) = settle_count(&mut app);

        assert_eq!(advances, 3);
        assert_eq!(redraws, 1);
        let complete = app.render_state().unwrap();
        assert_eq!(
            (complete.current_line, complete.total_lines),
            (Some(1), Some(30_001))
        );
        assert_eq!(complete.boundary, Some(ViewportBoundary::Top));
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
        settle_frame(&mut app);
        assert!(!app.document.line_index_complete());
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::Top));

        app.update(Action::DocumentEnd).unwrap();
        assert!(matches!(
            app.view_state().unwrap(),
            ViewState::Reader(RenderState {
                activity: Some(ViewActivity::GoingToEnd),
                boundary: Some(ViewportBoundary::Top),
                ..
            })
        ));
        assert!(!app.advance_background().unwrap());
        assert!(app.locator.is_none());
        assert_eq!(app.anchor, SourceOffset::ZERO);

        settle(&mut app);
        assert_eq!(app.anchor, SourceOffset::new(74_990));
        assert!(app.follow_end);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::End));
    }

    #[test]
    fn zero_width_source_has_one_owned_visible_cell() {
        let mut app = reader("\u{200b}", 16, 4);
        let state = app.render_state().unwrap();
        let row = state.rows.get(0).unwrap();
        let span = &row.spans[0];
        assert_eq!(row.span_text(span), "◌\u{200b}");
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
        assert!(!app.follow_end);
        assert_eq!(published_boundary(&mut app), Some(ViewportBoundary::End));
    }
}
