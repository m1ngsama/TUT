use std::{mem::size_of, num::NonZeroUsize, ops::Range};

use crate::{
    document::{DocumentId, DocumentReader, SOURCE_WINDOW_BYTES},
    error::{SearchError, TutError},
    source::SourceOffset,
};

const SEARCH_INDEX_MEMORY_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_DISPLAY_HIGHLIGHT_RANGES: usize =
    SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES / size_of::<SearchRange>();
const INITIAL_CHECKPOINT_INTERVAL_BYTES: u64 = SOURCE_WINDOW_BYTES as u64;
const INITIAL_CHECKPOINT_RESERVATION: usize = 1024;
pub(super) const MAX_SEARCH_QUERY_BYTES: usize = 4096;
const MATCH_BLOCK_START_LIMIT: usize = SOURCE_WINDOW_BYTES + MAX_SEARCH_QUERY_BYTES;
const DISPLAY_HIGHLIGHT_MERGE_BUDGET: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SearchHighlightKey {
    columns: u16,
    rows: u16,
    anchor: SourceOffset,
    visible_end: SourceOffset,
    target_count: usize,
}

impl SearchHighlightKey {
    pub(super) const fn new(
        columns: u16,
        rows: u16,
        anchor: SourceOffset,
        visible_end: SourceOffset,
        target_count: usize,
    ) -> Self {
        Self {
            columns,
            rows,
            anchor,
            visible_end,
            target_count,
        }
    }

    const fn visible(self) -> Range<SourceOffset> {
        self.anchor..self.visible_end
    }

    const fn target_count(self) -> usize {
        self.target_count
    }
}

#[derive(Debug)]
struct SearchQuery(String);

impl SearchQuery {
    fn new(text: String) -> Result<Option<Self>, TutError> {
        if text.is_empty() {
            return Ok(None);
        }
        if text.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(SearchError::QueryTooLong {
                limit: MAX_SEARCH_QUERY_BYTES,
            }
            .into());
        }
        Ok(Some(Self(text)))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SearchRange {
    start: SourceOffset,
    end: SourceOffset,
}

impl SearchRange {
    pub(super) const fn new(start: SourceOffset, end: SourceOffset) -> Option<Self> {
        if start.get() < end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub(super) const fn start(self) -> SourceOffset {
        self.start
    }

    pub(super) const fn end(self) -> SourceOffset {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchCheckpoint {
    scan_at: SourceOffset,
    previous_match: Option<SearchRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchIndexLimits {
    initial_interval_bytes: u64,
    max_checkpoints: usize,
}

impl SearchIndexLimits {
    const DEFAULT: Self = Self {
        initial_interval_bytes: INITIAL_CHECKPOINT_INTERVAL_BYTES,
        max_checkpoints: SEARCH_INDEX_MEMORY_BUDGET_BYTES / size_of::<SearchCheckpoint>(),
    };

    #[cfg(test)]
    const fn new(initial_interval_bytes: u64, max_checkpoints: usize) -> Option<Self> {
        if initial_interval_bytes == 0 || max_checkpoints < 2 {
            None
        } else {
            Some(Self {
                initial_interval_bytes,
                max_checkpoints,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SearchAdvance {
    selected: Option<SearchRange>,
    completed: bool,
}

#[derive(Debug)]
struct SearchNavigation {
    document_id: DocumentId,
    cursor: SourceOffset,
    source_end: SourceOffset,
    current: SearchRange,
    previous: Option<SearchRange>,
    block: MatchBlockCache,
    direction: NavigationDirection,
    wrap: Option<SearchRange>,
    #[cfg(test)]
    window_scans: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationDirection {
    Forward,
    Backward,
}

#[derive(Debug)]
struct MatchBlock {
    document_id: DocumentId,
    query_len: usize,
    start: SourceOffset,
    next: SourceOffset,
    previous: Option<SearchRange>,
    storage: MatchBlockStorage,
    selected: Option<usize>,
}

#[derive(Debug, Default)]
struct MatchBlockStorage {
    starts: Vec<u32>,
    #[cfg(test)]
    reserve_attempts: usize,
}

#[derive(Debug)]
enum MatchBlockCache {
    Ready(MatchBlock),
    Spare(MatchBlockStorage),
}

impl Default for MatchBlockCache {
    fn default() -> Self {
        Self::Spare(MatchBlockStorage::default())
    }
}

impl SearchAdvance {
    const fn selected(self) -> Option<SearchRange> {
        self.selected
    }

    const fn completed(self) -> bool {
        self.completed
    }
}

#[derive(Debug)]
struct SearchIndex {
    document_id: DocumentId,
    source_start: SourceOffset,
    source_end: SourceOffset,
    scanned_to: SourceOffset,
    query_len: usize,
    checkpoints: Vec<SearchCheckpoint>,
    checkpoint_interval_bytes: u64,
    max_checkpoints: usize,
    first_match: Option<SearchRange>,
    last_match: Option<SearchRange>,
    selection_anchor: SourceOffset,
    initial_selection: Option<SearchRange>,
    selection_delivered: bool,
    complete: bool,
    #[cfg(test)]
    checkpoint_reserve_attempts: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SearchStep {
    changed: bool,
    jump: Option<SearchRange>,
}

impl SearchStep {
    pub(super) const fn changed(self) -> bool {
        self.changed
    }

    pub(super) const fn jump(self) -> Option<SearchRange> {
        self.jump
    }
}

#[derive(Debug)]
pub(super) struct SearchSession {
    query: SearchQuery,
    index: SearchIndex,
    current_match: Option<SearchRange>,
    navigation: Option<SearchNavigation>,
    match_block: MatchBlockCache,
    pending_navigation: i64,
    highlights: SearchHighlightState,
    jump_pending: bool,
}

impl SearchSession {
    pub(super) fn new(
        reader: &DocumentReader<'_>,
        text: String,
        selection_anchor: SourceOffset,
    ) -> Result<Option<Self>, TutError> {
        let Some(query) = SearchQuery::new(text)? else {
            return Ok(None);
        };
        let index = SearchIndex::with_limits(
            reader,
            query.as_str(),
            selection_anchor,
            SearchIndexLimits::DEFAULT,
        )?;
        let jump_pending = !index.is_complete();
        Ok(Some(Self {
            query,
            index,
            current_match: None,
            navigation: None,
            match_block: MatchBlockCache::default(),
            pending_navigation: 0,
            highlights: SearchHighlightState::default(),
            jump_pending,
        }))
    }

    pub(super) fn query(&self) -> &str {
        self.query.as_str()
    }

    pub(super) const fn current_match(&self) -> Option<SearchRange> {
        self.current_match
    }

    pub(super) const fn no_matches(&self) -> bool {
        self.index.is_complete() && !self.index.has_matches()
    }

    pub(super) const fn is_searching(&self) -> bool {
        !self.index.is_complete() || self.navigation.is_some() || self.pending_navigation != 0
    }

    pub(super) const fn has_work(&self) -> bool {
        self.is_searching() || matches!(&self.highlights, SearchHighlightState::Pending(_))
    }

    pub(super) fn highlight_ranges(&self, key: SearchHighlightKey) -> &[SearchRange] {
        self.highlights
            .ready()
            .filter(|highlights| highlights.covers(key))
            .map_or(&[], PublishedSearchHighlights::ranges)
    }

    pub(super) fn prepare_highlights(&mut self, key: SearchHighlightKey) {
        if self.highlights.covers(key) {
            return;
        }
        let mut storage = self.highlights.take_storage();
        storage.ranges.clear();
        if !self.index.is_complete() || !self.index.has_matches() {
            self.highlights = SearchHighlightState::Absent(storage);
            return;
        }
        if key.target_count() > MAX_DISPLAY_HIGHLIGHT_RANGES {
            self.highlights = SearchHighlightState::Disabled { key, storage };
            return;
        }
        if !self.index.highlightable(&key.visible()) {
            self.highlights = SearchHighlightState::Absent(storage);
            return;
        }
        self.highlights =
            SearchHighlightState::Pending(self.index.display_highlight_job(key, storage));
    }

    pub(super) fn invalidate_highlights(&mut self) {
        let mut storage = self.highlights.take_storage();
        storage.ranges.clear();
        self.highlights = SearchHighlightState::Absent(storage);
    }

    fn cancel_pending_highlights(&mut self) {
        if matches!(self.highlights, SearchHighlightState::Pending(_)) {
            self.invalidate_highlights();
        }
    }

    pub(super) fn advance_highlights(
        &mut self,
        reader: &mut DocumentReader<'_>,
        key: SearchHighlightKey,
        mut target_at: impl FnMut(usize) -> SearchRange,
    ) -> Result<bool, TutError> {
        let state = std::mem::take(&mut self.highlights);
        let SearchHighlightState::Pending(mut job) = state else {
            self.highlights = state;
            return Ok(false);
        };
        if job.key != key {
            let mut storage = job.into_storage();
            storage.ranges.clear();
            self.highlights = SearchHighlightState::Absent(storage);
            return Ok(false);
        }
        match job.advance(
            reader,
            self.query.as_str(),
            &mut self.match_block,
            &mut target_at,
        ) {
            Ok(false) => {
                self.highlights = SearchHighlightState::Pending(job);
                Ok(false)
            }
            Ok(true) => {
                self.highlights = SearchHighlightState::Ready(job.publish());
                Ok(true)
            }
            Err(error) => {
                let mut storage = job.into_storage();
                storage.ranges.clear();
                self.highlights = SearchHighlightState::Absent(storage);
                Err(error)
            }
        }
    }

    pub(super) fn cancel_motion(&mut self) -> bool {
        let changed =
            self.navigation.is_some() || self.pending_navigation != 0 || self.jump_pending;
        if let Some(mut navigation) = self.navigation.take() {
            self.match_block = navigation.take_block();
        }
        self.pending_navigation = 0;
        self.jump_pending = false;
        changed
    }

    pub(super) fn request_navigation(
        &mut self,
        reader: &mut DocumentReader<'_>,
        forward: bool,
    ) -> Result<bool, TutError> {
        if reader.document_id() != self.index.document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        if self.index.is_complete() && self.current_match.is_none() {
            reader.validate()?;
            return Ok(false);
        }
        if self.current_match.is_none() {
            self.jump_pending = true;
        }
        self.cancel_pending_highlights();
        self.pending_navigation = if forward {
            self.pending_navigation.saturating_add(1)
        } else {
            self.pending_navigation.saturating_sub(1)
        };
        Ok(true)
    }

    pub(super) fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
    ) -> Result<SearchStep, TutError> {
        let initial_scan = !self.index.is_complete();
        let selection_should_jump;
        let advance = if initial_scan {
            selection_should_jump = self.jump_pending;
            self.index.advance(reader, self.query.as_str())?
        } else {
            if self.navigation.is_none() {
                if self.pending_navigation == 0 {
                    return Ok(SearchStep {
                        changed: false,
                        jump: None,
                    });
                }
                let Some(current) = self.current_match else {
                    let changed = self.pending_navigation != 0;
                    self.pending_navigation = 0;
                    self.jump_pending = false;
                    return Ok(SearchStep {
                        changed,
                        jump: None,
                    });
                };
                let forward = self.pending_navigation > 0;
                self.pending_navigation -= if forward { 1 } else { -1 };
                self.navigation = self.index.navigation_with_block(
                    current,
                    forward,
                    std::mem::take(&mut self.match_block),
                );
            }
            let Some(navigation) = self.navigation.as_mut() else {
                let changed = self.pending_navigation != 0;
                self.pending_navigation = 0;
                return Ok(SearchStep {
                    changed,
                    jump: None,
                });
            };
            selection_should_jump = true;
            navigation.advance(reader, self.query.as_str())?
        };
        let mut changed = advance.completed();
        if initial_scan && (advance.selected().is_some() || advance.completed()) {
            self.jump_pending = false;
        }
        if advance.completed() && self.navigation.is_some() {
            self.match_block = self
                .navigation
                .as_mut()
                .map_or_else(MatchBlockCache::default, SearchNavigation::take_block);
            self.navigation = None;
        }
        let jump = if let Some(selected) = advance.selected() {
            changed |= self.current_match != Some(selected);
            self.current_match = Some(selected);
            selection_should_jump.then_some(selected)
        } else {
            None
        };
        Ok(SearchStep { changed, jump })
    }

    #[cfg(test)]
    pub(super) const fn index_complete(&self) -> bool {
        self.index.is_complete()
    }

    #[cfg(test)]
    pub(super) const fn jump_pending(&self) -> bool {
        self.jump_pending
    }

    #[cfg(test)]
    pub(super) const fn has_cached_block(&self) -> bool {
        self.match_block.is_ready()
    }

    #[cfg(test)]
    pub(super) fn highlights_disabled_for(&self, visible: &Range<SourceOffset>) -> bool {
        matches!(
            &self.highlights,
            SearchHighlightState::Disabled {
                key,
                ..
            } if key.visible() == *visible
        )
    }

    #[cfg(test)]
    pub(super) const fn highlight_reserve_attempts(&self) -> usize {
        match &self.highlights {
            SearchHighlightState::Absent(storage)
            | SearchHighlightState::Disabled { storage, .. } => storage.reserve_attempts,
            SearchHighlightState::Pending(job) => job.storage.reserve_attempts,
            SearchHighlightState::Ready(highlights) => highlights.storage.reserve_attempts,
        }
    }

    #[cfg(test)]
    fn pending_highlight_progress(
        &self,
    ) -> Option<(DisplayHighlightPhase, usize, usize, Option<usize>)> {
        let SearchHighlightState::Pending(job) = &self.highlights else {
            return None;
        };
        Some((
            job.phase,
            job.target_index,
            job.match_index,
            job.pending_target,
        ))
    }

    #[cfg(test)]
    pub(super) fn pending_highlight_cursors(&self) -> Option<(usize, usize, Option<usize>)> {
        self.pending_highlight_progress()
            .map(|(_, target, found, pending)| (target, found, pending))
    }

    #[cfg(test)]
    fn pending_highlight_ranges(&self) -> &[SearchRange] {
        match &self.highlights {
            SearchHighlightState::Pending(job) => &job.storage.ranges,
            SearchHighlightState::Absent(_)
            | SearchHighlightState::Ready(_)
            | SearchHighlightState::Disabled { .. } => &[],
        }
    }
}

impl SearchIndex {
    #[cfg(test)]
    fn new(
        reader: &DocumentReader<'_>,
        needle: &str,
        selection_anchor: SourceOffset,
    ) -> Result<Option<Self>, TutError> {
        if needle.is_empty() {
            return Ok(None);
        }
        if needle.len() > MAX_SEARCH_QUERY_BYTES {
            return Err(SearchError::QueryTooLong {
                limit: MAX_SEARCH_QUERY_BYTES,
            }
            .into());
        }
        Self::with_limits(reader, needle, selection_anchor, SearchIndexLimits::DEFAULT).map(Some)
    }

    fn with_limits(
        reader: &DocumentReader<'_>,
        needle: &str,
        selection_anchor: SourceOffset,
        limits: SearchIndexLimits,
    ) -> Result<Self, TutError> {
        let source_start = reader.source_start();
        let source_end = reader.source_end();
        let selection_anchor = selection_anchor.clamp(source_start, source_end);
        let mut checkpoints = Vec::new();
        let initial_reservation = initial_checkpoint_reservation(source_start, source_end, limits);
        checkpoints
            .try_reserve_exact(initial_reservation)
            .map_err(|_| SearchError::Allocation)?;
        checkpoints.push(SearchCheckpoint {
            scan_at: source_start,
            previous_match: None,
        });

        Ok(Self {
            document_id: reader.document_id(),
            source_start,
            source_end,
            scanned_to: source_start,
            query_len: needle.len(),
            checkpoints,
            checkpoint_interval_bytes: limits.initial_interval_bytes,
            max_checkpoints: limits.max_checkpoints,
            first_match: None,
            last_match: None,
            selection_anchor,
            initial_selection: None,
            selection_delivered: false,
            complete: source_start == source_end,
            #[cfg(test)]
            checkpoint_reserve_attempts: 1,
        })
    }

    const fn is_complete(&self) -> bool {
        self.complete
    }

    const fn has_matches(&self) -> bool {
        self.first_match.is_some()
    }

    fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<SearchAdvance, TutError> {
        if reader.document_id() != self.document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        debug_assert_eq!(needle.len(), self.query_len);
        if self.complete {
            return Ok(SearchAdvance {
                selected: None,
                completed: false,
            });
        }

        let mut first_match = self.first_match;
        let mut last_match = self.last_match;
        let mut initial_selection = self.initial_selection;
        let next = scan_window(reader, needle, self.scanned_to, self.source_end, |range| {
            first_match.get_or_insert(range);
            last_match = Some(range);
            if initial_selection.is_none() && range.end() > self.selection_anchor {
                initial_selection = Some(range);
            }
            Ok(())
        })?;
        self.first_match = first_match;
        self.last_match = last_match;
        self.initial_selection = initial_selection;
        self.scanned_to = next;
        self.record_checkpoint()?;

        let completed = next == self.source_end;
        if completed {
            self.complete = true;
            if self.initial_selection.is_none() {
                self.initial_selection = self.first_match;
            }
        }
        let selected = if self.selection_delivered {
            None
        } else {
            self.initial_selection
        };
        self.selection_delivered |= selected.is_some();

        Ok(SearchAdvance {
            selected,
            completed,
        })
    }

    #[cfg(test)]
    fn navigation(&self, current: SearchRange, forward: bool) -> Option<SearchNavigation> {
        self.navigation_with_block(current, forward, MatchBlockCache::default())
    }

    fn navigation_with_block(
        &self,
        current: SearchRange,
        forward: bool,
        mut block: MatchBlockCache,
    ) -> Option<SearchNavigation> {
        if !self.complete {
            return None;
        }
        let direction = if forward {
            NavigationDirection::Forward
        } else {
            NavigationDirection::Backward
        };
        let checkpoint = (!forward).then(|| self.checkpoint_at_or_before(current.start()));
        if block.ready_mut().is_some_and(|block| {
            let compatible =
                block.document_id == self.document_id && block.query_len == self.query_len;
            let contains = compatible && block.locate(current).is_some();
            !(contains || compatible && forward && block.previous == Some(current))
        }) {
            block.make_spare();
        }
        let cursor = if forward {
            block.ready().map_or(current.end(), |block| block.start)
        } else {
            block.ready().map_or_else(
                || {
                    checkpoint
                        .expect("backward navigation starts from a checkpoint")
                        .scan_at
                },
                |block| block.start,
            )
        };
        let previous = if forward {
            Some(current)
        } else {
            block.ready().map_or_else(
                || {
                    checkpoint
                        .expect("backward navigation starts from a checkpoint")
                        .previous_match
                        .filter(|range| range.start() < current.start())
                },
                |block| block.previous,
            )
        };
        Some(SearchNavigation {
            document_id: self.document_id,
            cursor,
            source_end: self.source_end,
            current,
            previous,
            block,
            direction,
            wrap: if forward {
                self.first_match
            } else {
                self.last_match
            },
            #[cfg(test)]
            window_scans: 0,
        })
    }

    #[cfg(test)]
    fn highlights(&self, visible: Range<SourceOffset>) -> Option<SearchHighlights> {
        self.make_highlights(
            visible,
            SearchHighlightStorage::default(),
            HighlightMode::Exact,
        )
    }

    fn display_highlight_job(
        &self,
        key: SearchHighlightKey,
        storage: SearchHighlightStorage,
    ) -> DisplayHighlightJob {
        let visible = key.visible();
        debug_assert!(self.highlightable(&visible));
        let earliest = visible
            .start
            .checked_sub(self.query_len.saturating_sub(1))
            .unwrap_or(self.source_start)
            .max(self.source_start);
        let checkpoint = self.checkpoint_at_or_before(earliest);
        DisplayHighlightJob {
            key,
            document_id: self.document_id,
            cursor: checkpoint.scan_at,
            needed_from: earliest,
            previous: checkpoint.previous_match,
            source_end: self.source_end,
            target_index: 0,
            match_index: 0,
            match_end: 0,
            pending_target: None,
            storage,
            phase: if key.target_count() == 0 || checkpoint.scan_at >= self.source_end {
                DisplayHighlightPhase::Validate
            } else {
                DisplayHighlightPhase::NeedBlock
            },
            #[cfg(test)]
            last_step: DisplayHighlightStepMetrics::default(),
        }
    }

    #[cfg(test)]
    fn display_highlights(
        &self,
        visible: Range<SourceOffset>,
        targets: impl IntoIterator<Item = SearchRange>,
        mut storage: SearchHighlightStorage,
    ) -> Result<DisplayHighlightPreparation, TutError> {
        if !self.highlightable(&visible) {
            return Ok(DisplayHighlightPreparation::Unavailable);
        }
        storage.ranges.clear();
        let mut targets = targets.into_iter();
        if targets.size_hint().0 > MAX_DISPLAY_HIGHLIGHT_RANGES {
            return Ok(DisplayHighlightPreparation::Disabled(storage));
        }
        for target in &mut targets {
            debug_assert!(target.start() < visible.end && target.end() > visible.start);
            debug_assert!(
                storage
                    .ranges
                    .last()
                    .is_none_or(|last: &SearchRange| { last.end() <= target.start() })
            );
            if !reserve_display_target(&mut storage)? {
                storage.ranges.clear();
                return Ok(DisplayHighlightPreparation::Disabled(storage));
            }
            storage.ranges.push(target);
        }
        Ok(DisplayHighlightPreparation::Ready(self.new_highlights(
            visible,
            storage,
            HighlightMode::Display { read: 0, write: 0 },
        )))
    }

    fn highlightable(&self, visible: &Range<SourceOffset>) -> bool {
        self.complete
            && visible.start < visible.end
            && visible.start < self.source_end
            && visible.end > self.source_start
    }

    #[cfg(test)]
    fn make_highlights(
        &self,
        visible: Range<SourceOffset>,
        storage: SearchHighlightStorage,
        mode: HighlightMode,
    ) -> Option<SearchHighlights> {
        if !self.highlightable(&visible) {
            return None;
        }
        Some(self.new_highlights(visible, storage, mode))
    }

    #[cfg(test)]
    fn new_highlights(
        &self,
        visible: Range<SourceOffset>,
        storage: SearchHighlightStorage,
        mode: HighlightMode,
    ) -> SearchHighlights {
        let earliest = visible
            .start
            .checked_sub(self.query_len.saturating_sub(1))
            .unwrap_or(self.source_start)
            .max(self.source_start);
        let checkpoint = self.checkpoint_at_or_before(earliest);
        let mut highlights = SearchHighlights {
            document_id: self.document_id,
            visible,
            cursor: checkpoint.scan_at,
            needed_from: earliest,
            previous: checkpoint.previous_match,
            source_end: self.source_end,
            storage,
            mode,
            complete: false,
        };
        if checkpoint.scan_at >= self.source_end {
            highlights.finish();
        }
        highlights
    }

    fn record_checkpoint(&mut self) -> Result<(), TutError> {
        let last = self
            .checkpoints
            .last()
            .expect("search indexes retain their source-start checkpoint");
        if self.scanned_to.get() - last.scan_at.get() < self.checkpoint_interval_bytes {
            return Ok(());
        }
        while self.checkpoints.len() >= self.max_checkpoints {
            self.compact()?;
            let last = self
                .checkpoints
                .last()
                .expect("search indexes retain their source-start checkpoint");
            if self.scanned_to.get() - last.scan_at.get() < self.checkpoint_interval_bytes {
                return Ok(());
            }
        }
        self.reserve_checkpoint()?;
        self.checkpoints.push(SearchCheckpoint {
            scan_at: self.scanned_to,
            previous_match: self.last_match,
        });
        Ok(())
    }

    fn compact(&mut self) -> Result<(), TutError> {
        self.checkpoint_interval_bytes = self
            .checkpoint_interval_bytes
            .checked_mul(2)
            .ok_or(SearchError::CoordinateOverflow)?;
        let mut index = 0;
        self.checkpoints.retain(|_| {
            let keep = index % 2 == 0;
            index += 1;
            keep
        });
        Ok(())
    }

    fn reserve_checkpoint(&mut self) -> Result<(), TutError> {
        if self.checkpoints.len() < self.checkpoints.capacity() {
            return Ok(());
        }
        let remaining = self.max_checkpoints - self.checkpoints.len();
        let additional = self.checkpoints.capacity().max(1).min(remaining);
        #[cfg(test)]
        {
            self.checkpoint_reserve_attempts += 1;
        }
        self.checkpoints
            .try_reserve_exact(additional)
            .map_err(|_| SearchError::Allocation)?;
        Ok(())
    }

    fn checkpoint_at_or_before(&self, offset: SourceOffset) -> SearchCheckpoint {
        let insertion = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.scan_at <= offset);
        self.checkpoints[insertion.saturating_sub(1)]
    }
}

fn initial_checkpoint_reservation(
    source_start: SourceOffset,
    source_end: SourceOffset,
    limits: SearchIndexLimits,
) -> usize {
    let source_len = source_end.get() - source_start.get();
    let checkpoints = source_len
        .checked_div(limits.initial_interval_bytes)
        .expect("search checkpoint intervals are nonzero")
        .saturating_add(1);
    usize::try_from(checkpoints)
        .unwrap_or(usize::MAX)
        .min(INITIAL_CHECKPOINT_RESERVATION)
        .min(limits.max_checkpoints)
}

impl MatchBlockCache {
    const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    const fn ready(&self) -> Option<&MatchBlock> {
        match self {
            Self::Ready(block) => Some(block),
            Self::Spare(_) => None,
        }
    }

    const fn ready_mut(&mut self) -> Option<&mut MatchBlock> {
        match self {
            Self::Ready(block) => Some(block),
            Self::Spare(_) => None,
        }
    }

    fn make_spare(&mut self) {
        let storage = match std::mem::take(self) {
            Self::Ready(block) => block.storage,
            Self::Spare(storage) => storage,
        };
        *self = Self::Spare(storage);
    }

    fn scan(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
        document_id: DocumentId,
        source_end: SourceOffset,
        start: SourceOffset,
        previous: Option<SearchRange>,
    ) -> Result<(), TutError> {
        if reader.document_id() != document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        self.make_spare();
        let Self::Spare(mut storage) = std::mem::take(self) else {
            unreachable!("match block caches become spare before scanning");
        };
        storage.starts.clear();
        let mut block = MatchBlock {
            document_id,
            query_len: needle.len(),
            start,
            next: start,
            previous,
            storage,
            selected: None,
        };
        match scan_window(reader, needle, start, source_end, |range| {
            block.push(range.start())
        }) {
            Ok(next) => {
                block.next = next;
                *self = Self::Ready(block);
                Ok(())
            }
            Err(error) => {
                block.storage.starts.clear();
                *self = Self::Spare(block.storage);
                Err(error)
            }
        }
    }
}

impl MatchBlock {
    fn starts(&self) -> &[u32] {
        &self.storage.starts
    }

    fn push(&mut self, start: SourceOffset) -> Result<(), TutError> {
        let starts = &mut self.storage.starts;
        if starts.len() >= MATCH_BLOCK_START_LIMIT {
            return Err(SearchError::Allocation.into());
        }
        let relative = start
            .get()
            .checked_sub(self.start.get())
            .and_then(|relative| u32::try_from(relative).ok())
            .ok_or(SearchError::CoordinateOverflow)?;
        if starts.len() == starts.capacity() {
            let remaining = MATCH_BLOCK_START_LIMIT - starts.len();
            let additional = starts.capacity().max(256).min(remaining);
            #[cfg(test)]
            {
                self.storage.reserve_attempts = self.storage.reserve_attempts.saturating_add(1);
            }
            starts
                .try_reserve_exact(additional)
                .map_err(|_| SearchError::Allocation)?;
        }
        starts.push(relative);
        Ok(())
    }

    fn range(&self, index: usize) -> SearchRange {
        let relative = usize::try_from(self.starts()[index])
            .expect("u32 match offsets fit the process address space");
        let start = self
            .start
            .checked_add(relative)
            .expect("match blocks retain validated source coordinates");
        let end = start
            .checked_add(self.query_len)
            .expect("match blocks retain validated source coordinates");
        SearchRange::new(start, end).expect("nonempty queries produce nonempty ranges")
    }

    #[cfg(test)]
    fn overlapping(&self, visible: &Range<SourceOffset>) -> impl Iterator<Item = SearchRange> + '_ {
        let (first, last) = self.overlapping_indices(visible);
        (first..last).map(|index| self.range(index))
    }

    fn overlapping_indices(&self, visible: &Range<SourceOffset>) -> (usize, usize) {
        debug_assert_ne!(self.query_len, 0);
        let overlap = u64::try_from(self.query_len - 1).expect("query lengths fit u64");
        let earliest = visible.start.get().saturating_sub(overlap);
        let lower = earliest.saturating_sub(self.start.get());
        let upper = visible.end.get().saturating_sub(self.start.get());
        let first = self
            .starts()
            .partition_point(|start| u64::from(*start) < lower);
        let last = self
            .starts()
            .partition_point(|start| u64::from(*start) < upper);

        (first, last)
    }

    fn locate(&mut self, range: SearchRange) -> Option<usize> {
        if let Some(index) = self.selected
            && self.range(index) == range
        {
            return Some(index);
        }
        let relative = range.start().get().checked_sub(self.start.get())?;
        let relative = u32::try_from(relative).ok()?;
        let index = self.starts().binary_search(&relative).ok()?;
        if self.range(index) != range {
            return None;
        }
        self.selected = Some(index);
        Some(index)
    }

    fn select(&mut self, index: usize) -> SearchRange {
        self.selected = Some(index);
        self.range(index)
    }

    fn first(&mut self) -> Option<SearchRange> {
        (!self.starts().is_empty()).then(|| self.select(0))
    }

    fn last_or_previous(&self) -> Option<SearchRange> {
        self.starts()
            .len()
            .checked_sub(1)
            .map(|index| self.range(index))
            .or(self.previous)
    }

    fn successor(&mut self, current: SearchRange) -> Option<SearchRange> {
        if self.previous == Some(current) {
            return self.first();
        }
        let index = self.locate(current)?.checked_add(1)?;
        (index < self.starts().len()).then(|| self.select(index))
    }

    fn predecessor(&mut self, current: SearchRange) -> Option<SearchRange> {
        let index = self.locate(current)?;
        index.checked_sub(1).map(|index| self.select(index))
    }

    fn last_before(&mut self, before: SourceOffset) -> Option<SearchRange> {
        let relative = before.get().saturating_sub(self.start.get());
        let index = self
            .starts()
            .partition_point(|start| u64::from(*start) < relative)
            .checked_sub(1)?;
        Some(self.select(index))
    }

    fn clear_selection(&mut self) {
        self.selected = None;
    }
}

impl SearchNavigation {
    fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<SearchAdvance, TutError> {
        if reader.document_id() != self.document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        match self.direction {
            NavigationDirection::Forward => self.advance_forward(reader, needle),
            NavigationDirection::Backward => self.advance_backward(reader, needle),
        }
    }

    fn take_block(&mut self) -> MatchBlockCache {
        std::mem::take(&mut self.block)
    }

    #[cfg(test)]
    const fn window_scans(&self) -> usize {
        self.window_scans
    }

    fn advance_forward(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<SearchAdvance, TutError> {
        let reused_block = self.block.is_ready();
        if reused_block {
            reader.validate()?;
        }
        if let Some(block) = self.block.ready_mut() {
            if let Some(selected) = block.successor(self.current) {
                return Ok(SearchAdvance {
                    selected: Some(selected),
                    completed: true,
                });
            }
            self.cursor = block.next;
            self.previous = block.last_or_previous();
            if self.cursor >= self.source_end {
                block.clear_selection();
                return Ok(SearchAdvance {
                    selected: self.wrap,
                    completed: true,
                });
            }
        }
        if self.cursor >= self.source_end {
            if !reused_block {
                reader.validate()?;
            }
            return Ok(SearchAdvance {
                selected: self.wrap,
                completed: true,
            });
        }

        self.block.scan(
            reader,
            needle,
            self.document_id,
            self.source_end,
            self.cursor,
            self.previous,
        )?;
        #[cfg(test)]
        {
            self.window_scans += 1;
        }
        let block = self
            .block
            .ready_mut()
            .expect("successful scans publish a ready match block");
        self.cursor = block.next;
        self.previous = block.last_or_previous();
        let selected = block.first();
        let completed = selected.is_some() || self.cursor == self.source_end;
        if completed && selected.is_none() {
            block.clear_selection();
        }
        Ok(SearchAdvance {
            selected: if completed {
                selected.or(self.wrap)
            } else {
                None
            },
            completed,
        })
    }

    fn advance_backward(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<SearchAdvance, TutError> {
        let reused_block = self.block.is_ready();
        if reused_block {
            reader.validate()?;
        }
        if let Some(block) = self.block.ready_mut() {
            if block.locate(self.current).is_some() {
                let selected = block.predecessor(self.current).or(block.previous);
                if selected.is_none() || selected == block.previous {
                    block.clear_selection();
                }
                return Ok(SearchAdvance {
                    selected: selected.or(self.wrap),
                    completed: true,
                });
            }
            self.cursor = block.next;
            self.previous = block.last_or_previous();
        }
        if self.cursor >= self.current.start() {
            if !reused_block {
                reader.validate()?;
            }
            return Ok(SearchAdvance {
                selected: self.previous.or(self.wrap),
                completed: true,
            });
        }

        self.block.scan(
            reader,
            needle,
            self.document_id,
            self.source_end,
            self.cursor,
            self.previous,
        )?;
        #[cfg(test)]
        {
            self.window_scans += 1;
        }
        let block = self
            .block
            .ready_mut()
            .expect("successful scans publish a ready match block");
        self.cursor = block.next;
        self.previous = block.last_or_previous();
        if self.cursor < self.current.start() {
            return Ok(SearchAdvance {
                selected: None,
                completed: false,
            });
        }

        let selected = block.last_before(self.current.start()).or(block.previous);
        if selected.is_none() || selected == block.previous {
            block.clear_selection();
        }
        Ok(SearchAdvance {
            selected: selected.or(self.wrap),
            completed: true,
        })
    }
}

#[derive(Debug)]
enum SearchHighlightState {
    Absent(SearchHighlightStorage),
    Pending(DisplayHighlightJob),
    Ready(PublishedSearchHighlights),
    Disabled {
        key: SearchHighlightKey,
        storage: SearchHighlightStorage,
    },
}

impl Default for SearchHighlightState {
    fn default() -> Self {
        Self::Absent(SearchHighlightStorage::default())
    }
}

impl SearchHighlightState {
    fn covers(&self, key: SearchHighlightKey) -> bool {
        match self {
            Self::Absent(_) => false,
            Self::Pending(job) => job.key == key,
            Self::Ready(highlights) => highlights.covers(key),
            Self::Disabled { key: disabled, .. } => *disabled == key,
        }
    }

    const fn ready(&self) -> Option<&PublishedSearchHighlights> {
        match self {
            Self::Ready(highlights) => Some(highlights),
            Self::Absent(_) | Self::Pending(_) | Self::Disabled { .. } => None,
        }
    }

    fn take_storage(&mut self) -> SearchHighlightStorage {
        match std::mem::take(self) {
            Self::Absent(storage) | Self::Disabled { storage, .. } => storage,
            Self::Pending(job) => job.into_storage(),
            Self::Ready(highlights) => highlights.into_storage(),
        }
    }
}

#[derive(Debug)]
struct PublishedSearchHighlights {
    key: SearchHighlightKey,
    storage: SearchHighlightStorage,
}

impl PublishedSearchHighlights {
    fn into_storage(self) -> SearchHighlightStorage {
        self.storage
    }

    fn covers(&self, key: SearchHighlightKey) -> bool {
        self.key == key
    }

    fn ranges(&self) -> &[SearchRange] {
        &self.storage.ranges
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayHighlightPhase {
    NeedBlock,
    Merge,
    Grow,
    Validate,
}

#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DisplayHighlightStepMetrics {
    phase: Option<DisplayHighlightPhase>,
    comparisons: usize,
    output_attempts: usize,
}

#[derive(Debug)]
struct DisplayHighlightJob {
    key: SearchHighlightKey,
    document_id: DocumentId,
    cursor: SourceOffset,
    needed_from: SourceOffset,
    previous: Option<SearchRange>,
    source_end: SourceOffset,
    target_index: usize,
    match_index: usize,
    match_end: usize,
    pending_target: Option<usize>,
    storage: SearchHighlightStorage,
    phase: DisplayHighlightPhase,
    #[cfg(test)]
    last_step: DisplayHighlightStepMetrics,
}

impl DisplayHighlightJob {
    fn into_storage(self) -> SearchHighlightStorage {
        self.storage
    }

    fn publish(self) -> PublishedSearchHighlights {
        debug_assert_eq!(self.phase, DisplayHighlightPhase::Validate);
        PublishedSearchHighlights {
            key: self.key,
            storage: self.storage,
        }
    }

    fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
        cached: &mut MatchBlockCache,
        target_at: &mut impl FnMut(usize) -> SearchRange,
    ) -> Result<bool, TutError> {
        if reader.document_id() != self.document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        #[cfg(test)]
        {
            self.last_step = DisplayHighlightStepMetrics {
                phase: Some(self.phase),
                ..DisplayHighlightStepMetrics::default()
            };
        }
        match self.phase {
            DisplayHighlightPhase::NeedBlock => self.need_block(reader, needle, cached)?,
            DisplayHighlightPhase::Merge => self.merge(cached, target_at),
            DisplayHighlightPhase::Grow => self.grow()?,
            DisplayHighlightPhase::Validate => {
                reader.validate()?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn need_block(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
        cached: &mut MatchBlockCache,
    ) -> Result<(), TutError> {
        debug_assert_eq!(self.phase, DisplayHighlightPhase::NeedBlock);
        if self.target_index >= self.key.target_count()
            || self.cursor >= self.source_end
            || self.cursor >= self.key.visible_end
        {
            self.phase = DisplayHighlightPhase::Validate;
            return Ok(());
        }
        let reusable = cached.ready().is_some_and(|block| {
            block.document_id == self.document_id
                && block.query_len == needle.len()
                && block.start <= self.needed_from
                && self.needed_from < block.next
        });
        if reusable {
            reader.validate()?;
        } else {
            cached.scan(
                reader,
                needle,
                self.document_id,
                self.source_end,
                self.cursor,
                self.previous,
            )?;
        }
        let block = cached
            .ready()
            .expect("reused and successfully scanned match blocks are ready");
        (self.match_index, self.match_end) = block.overlapping_indices(&self.key.visible());
        self.phase = DisplayHighlightPhase::Merge;
        Ok(())
    }

    fn merge(
        &mut self,
        cached: &MatchBlockCache,
        target_at: &mut impl FnMut(usize) -> SearchRange,
    ) {
        debug_assert_eq!(self.phase, DisplayHighlightPhase::Merge);
        let block = cached
            .ready()
            .expect("merge phases retain a ready match block");
        let mut operations = 0;

        if let Some(index) = self.pending_target {
            let target = target_at(index);
            debug_assert!(self.can_append(target));
            self.append(target);
            self.pending_target = None;
            operations += 1;
            #[cfg(test)]
            {
                self.last_step.output_attempts += 1;
            }
        }

        while operations < DISPLAY_HIGHLIGHT_MERGE_BUDGET
            && self.pending_target.is_none()
            && self.target_index < self.key.target_count()
            && self.match_index < self.match_end
        {
            let target_index = self.target_index;
            let target = target_at(target_index);
            let found = block.range(self.match_index);
            operations += 1;
            #[cfg(test)]
            {
                self.last_step.comparisons += 1;
            }
            if target.end() <= found.start() {
                self.target_index += 1;
                continue;
            }
            if target.start() >= found.end() {
                self.match_index += 1;
                continue;
            }

            self.target_index += 1;
            let can_append = self.can_append(target);
            if operations >= DISPLAY_HIGHLIGHT_MERGE_BUDGET {
                self.pending_target = Some(target_index);
                if !can_append {
                    self.phase = DisplayHighlightPhase::Grow;
                }
                break;
            }
            if !can_append {
                self.pending_target = Some(target_index);
                self.phase = DisplayHighlightPhase::Grow;
                break;
            }
            self.append(target);
            operations += 1;
            #[cfg(test)]
            {
                self.last_step.output_attempts += 1;
            }
        }

        if self.pending_target.is_some() {
            return;
        }
        if self.target_index >= self.key.target_count() {
            self.phase = DisplayHighlightPhase::Validate;
        } else if self.match_index >= self.match_end {
            self.cursor = block.next;
            self.needed_from = block.next;
            self.previous = block.last_or_previous();
            self.phase = if self.cursor >= self.source_end || self.cursor >= self.key.visible_end {
                DisplayHighlightPhase::Validate
            } else {
                DisplayHighlightPhase::NeedBlock
            };
        }
    }

    fn can_append(&self, range: SearchRange) -> bool {
        self.storage
            .ranges
            .last()
            .is_some_and(|last| last.end() == range.start())
            || self.storage.ranges.len() < self.storage.ranges.capacity()
    }

    fn append(&mut self, range: SearchRange) {
        if let Some(last) = self.storage.ranges.last_mut()
            && last.end() == range.start()
        {
            last.end = range.end();
        } else {
            debug_assert!(self.storage.ranges.len() < self.storage.ranges.capacity());
            self.storage.ranges.push(range);
        }
    }

    fn grow(&mut self) -> Result<(), TutError> {
        debug_assert_eq!(self.phase, DisplayHighlightPhase::Grow);
        let ranges = &mut self.storage.ranges;
        debug_assert!(ranges.len() < MAX_DISPLAY_HIGHLIGHT_RANGES);
        debug_assert_eq!(ranges.len(), ranges.capacity());
        let remaining = MAX_DISPLAY_HIGHLIGHT_RANGES - ranges.len();
        let additional = ranges.capacity().max(256).min(remaining);
        #[cfg(test)]
        {
            self.storage.reserve_attempts = self.storage.reserve_attempts.saturating_add(1);
            if self.storage.fail_reserve {
                self.storage.fail_reserve = false;
                return Err(SearchError::Allocation.into());
            }
        }
        ranges
            .try_reserve_exact(additional)
            .map_err(|_| SearchError::Allocation)?;
        self.phase = DisplayHighlightPhase::Merge;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Debug)]
enum DisplayHighlightPreparation {
    Unavailable,
    Ready(SearchHighlights),
    Disabled(SearchHighlightStorage),
}

#[cfg(test)]
#[derive(Debug)]
struct SearchHighlights {
    document_id: DocumentId,
    visible: Range<SourceOffset>,
    cursor: SourceOffset,
    needed_from: SourceOffset,
    previous: Option<SearchRange>,
    source_end: SourceOffset,
    storage: SearchHighlightStorage,
    mode: HighlightMode,
    complete: bool,
}

#[derive(Debug, Default)]
struct SearchHighlightStorage {
    ranges: Vec<SearchRange>,
    #[cfg(test)]
    reserve_attempts: usize,
    #[cfg(test)]
    fail_reserve: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy)]
enum HighlightMode {
    #[cfg(test)]
    Exact,
    Display {
        read: usize,
        write: usize,
    },
}

#[cfg(test)]
impl SearchHighlights {
    fn into_storage(self) -> SearchHighlightStorage {
        self.storage
    }

    const fn is_complete(&self) -> bool {
        self.complete
    }

    fn ranges(&self) -> &[SearchRange] {
        if self.complete {
            &self.storage.ranges
        } else {
            &[]
        }
    }

    fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
        cached: &mut MatchBlockCache,
    ) -> Result<bool, TutError> {
        if reader.document_id() != self.document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        if self.complete {
            return Ok(false);
        }
        if self.cursor >= self.source_end || self.cursor >= self.visible.end {
            reader.validate()?;
            self.finish();
            return Ok(true);
        }

        let reusable = cached.ready().is_some_and(|block| {
            block.document_id == self.document_id
                && block.query_len == needle.len()
                && block.start <= self.needed_from
                && self.needed_from < block.next
        });
        if reusable {
            reader.validate()?;
        } else {
            cached.scan(
                reader,
                needle,
                self.document_id,
                self.source_end,
                self.cursor,
                self.previous,
            )?;
        }
        let block = cached
            .ready()
            .expect("reused and successfully scanned match blocks are ready");
        let push_result = block
            .overlapping(&self.visible)
            .try_for_each(|range| self.push(range));
        if push_result.is_ok() {
            self.cursor = block.next;
            self.needed_from = block.next;
            self.previous = block.last_or_previous();
        }
        push_result?;
        if self.cursor >= self.source_end || self.cursor >= self.visible.end {
            self.finish();
        }
        Ok(self.complete)
    }

    fn push(&mut self, range: SearchRange) -> Result<(), TutError> {
        match self.mode {
            #[cfg(test)]
            HighlightMode::Exact => self.push_exact(range),
            HighlightMode::Display { read, write } => {
                self.push_display(range, read, write);
                Ok(())
            }
        }
    }

    #[cfg(test)]
    fn push_exact(&mut self, range: SearchRange) -> Result<(), TutError> {
        if let Some(last) = self.storage.ranges.last_mut()
            && last.end() == range.start()
        {
            last.end = range.end();
            return Ok(());
        }
        reserve_highlight_range(&mut self.storage)?;
        self.storage.ranges.push(range);
        Ok(())
    }

    fn push_display(&mut self, range: SearchRange, mut read: usize, mut write: usize) {
        while let Some(target) = self.storage.ranges.get(read).copied() {
            if target.end() <= range.start() {
                read += 1;
                continue;
            }
            if target.start() >= range.end() {
                break;
            }
            read += 1;
            if write > 0 && self.storage.ranges[write - 1].end() == target.start() {
                self.storage.ranges[write - 1].end = target.end();
            } else {
                self.storage.ranges[write] = target;
                write += 1;
            }
        }
        self.mode = HighlightMode::Display { read, write };
    }

    fn finish(&mut self) {
        match self.mode {
            #[cfg(test)]
            HighlightMode::Exact => {}
            HighlightMode::Display { write, .. } => self.storage.ranges.truncate(write),
        }
        self.complete = true;
    }
}

#[cfg(test)]
fn reserve_highlight_range(storage: &mut SearchHighlightStorage) -> Result<(), TutError> {
    if reserve_display_target(storage)? {
        return Ok(());
    }
    Err(SearchError::Allocation.into())
}

#[cfg(test)]
fn reserve_display_target(storage: &mut SearchHighlightStorage) -> Result<bool, TutError> {
    let ranges = &mut storage.ranges;
    if ranges.len() >= MAX_DISPLAY_HIGHLIGHT_RANGES {
        return Ok(false);
    }
    if ranges.len() == ranges.capacity() {
        #[cfg(test)]
        {
            storage.reserve_attempts = storage.reserve_attempts.saturating_add(1);
            if storage.fail_reserve {
                storage.fail_reserve = false;
                return Err(SearchError::Allocation.into());
            }
        }
        let remaining = MAX_DISPLAY_HIGHLIGHT_RANGES - ranges.len();
        let additional = ranges.capacity().max(256).min(remaining);
        ranges
            .try_reserve_exact(additional)
            .map_err(|_| SearchError::Allocation)?;
    }
    Ok(true)
}

fn scan_window(
    reader: &mut DocumentReader<'_>,
    needle: &str,
    cursor: SourceOffset,
    source_end: SourceOffset,
    mut visit: impl FnMut(SearchRange) -> Result<(), TutError>,
) -> Result<SourceOffset, TutError> {
    let target_bytes = SOURCE_WINDOW_BYTES
        .checked_add(needle.len().saturating_sub(1))
        .and_then(NonZeroUsize::new)
        .ok_or(SearchError::Allocation)?;
    let window = reader.window(cursor, target_bytes)?;
    let text = window.as_str();
    let safe_relative = if window.end() == source_end {
        text.len()
    } else {
        let mut boundary = SOURCE_WINDOW_BYTES.min(text.len());
        while boundary < text.len() && !text.is_char_boundary(boundary) {
            boundary += 1;
        }
        boundary
    };
    let mut search_from = 0;
    let mut last_match_end = 0;

    while search_from < text.len() {
        let Some(relative) = text[search_from..].find(needle) else {
            break;
        };
        let start = search_from + relative;
        if start >= safe_relative {
            break;
        }
        let absolute_start = cursor
            .checked_add(start)
            .ok_or(SearchError::CoordinateOverflow)?;
        let absolute_end = absolute_start
            .checked_add(needle.len())
            .ok_or(SearchError::CoordinateOverflow)?;
        visit(
            SearchRange::new(absolute_start, absolute_end)
                .expect("nonempty queries produce nonempty ranges"),
        )?;
        last_match_end = start + needle.len();
        search_from = last_match_end;
    }

    let next = cursor
        .checked_add(safe_relative.max(last_match_end))
        .ok_or(SearchError::CoordinateOverflow)?;
    if next <= cursor || next > source_end {
        return Err(SearchError::NonIncreasingCursor { at: cursor.get() }.into());
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use crate::document::{Document, DocumentCache, MAX_FILE_BYTES};
    use tempfile::tempdir;
    use unicode_segmentation::UnicodeSegmentation;

    fn complete_at(
        text: &str,
        start: SourceOffset,
        needle: &str,
        anchor: SourceOffset,
    ) -> (Document, SearchIndex) {
        let document = Document::from_text_at(Path::new("search.txt"), text.to_owned(), start);
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut index = SearchIndex::new(&reader, needle, anchor).unwrap().unwrap();
        while !index.is_complete() {
            index.advance(&mut reader, needle).unwrap();
        }
        (document, index)
    }

    fn complete(text: &str, needle: &str) -> (Document, SearchIndex) {
        complete_at(text, SourceOffset::ZERO, needle, SourceOffset::ZERO)
    }

    fn matches(
        document: &Document,
        index: &SearchIndex,
        needle: &str,
        visible: Range<SourceOffset>,
    ) -> Vec<SearchRange> {
        let mut highlights = index.highlights(visible).unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut block = MatchBlockCache::default();
        while !highlights.is_complete() {
            highlights.advance(&mut reader, needle, &mut block).unwrap();
        }
        highlights.ranges().to_vec()
    }

    fn exact_matches(document: &Document, needle: &str) -> Vec<SearchRange> {
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let source_end = reader.source_end();
        let mut cursor = reader.source_start();
        let mut matches = Vec::new();
        while cursor < source_end {
            cursor = scan_window(&mut reader, needle, cursor, source_end, |range| {
                matches.push(range);
                Ok(())
            })
            .unwrap();
        }
        matches
    }

    fn display_highlight_oracle(
        document: &Document,
        index: &SearchIndex,
        needle: &str,
        visible: Range<SourceOffset>,
        targets: &[SearchRange],
    ) -> Vec<SearchRange> {
        let DisplayHighlightPreparation::Ready(mut highlights) = index
            .display_highlights(
                visible,
                targets.iter().copied(),
                SearchHighlightStorage::default(),
            )
            .unwrap()
        else {
            panic!("oracle targets should be highlightable");
        };
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut block = MatchBlockCache::default();
        while !highlights.is_complete() {
            highlights.advance(&mut reader, needle, &mut block).unwrap();
        }
        highlights.ranges().to_vec()
    }

    fn stream_display_highlights(
        document: &Document,
        index: &SearchIndex,
        needle: &str,
        visible: Range<SourceOffset>,
        targets: &[SearchRange],
    ) -> (Vec<SearchRange>, usize) {
        let key = display_key(visible, targets.len());
        let mut job = index.display_highlight_job(key, SearchHighlightStorage::default());
        let mut cache = DocumentCache::default();
        let mut block = MatchBlockCache::default();
        let mut visits = 0;
        loop {
            let phase = job.phase;
            let before_target = job.target_index;
            let before_match = job.match_index;
            let before_reserves = job.storage.reserve_attempts;
            cache.reset_metrics();
            let complete = {
                let mut reader = document.reader(&mut cache);
                job.advance(&mut reader, needle, &mut block, &mut |index| {
                    visits += 1;
                    targets[index]
                })
                .unwrap()
            };
            let metrics = job.last_step;
            assert_eq!(metrics.phase, Some(phase));
            assert!(metrics.comparisons <= DISPLAY_HIGHLIGHT_MERGE_BUDGET);
            assert!(metrics.output_attempts <= DISPLAY_HIGHLIGHT_MERGE_BUDGET);
            assert!(
                metrics.comparisons + metrics.output_attempts <= DISPLAY_HIGHLIGHT_MERGE_BUDGET
            );
            match phase {
                DisplayHighlightPhase::NeedBlock => {
                    assert_eq!((metrics.comparisons, metrics.output_attempts), (0, 0));
                    assert!(cache.metrics().window_calls() <= 1);
                    assert_eq!(job.storage.reserve_attempts, before_reserves);
                }
                DisplayHighlightPhase::Merge => {
                    assert_eq!(cache.metrics().window_calls(), 0);
                    assert_eq!(job.storage.reserve_attempts, before_reserves);
                    assert_eq!(
                        job.target_index - before_target + job.match_index - before_match,
                        metrics.comparisons
                    );
                }
                DisplayHighlightPhase::Grow => {
                    assert_eq!((metrics.comparisons, metrics.output_attempts), (0, 0));
                    assert_eq!(cache.metrics().window_calls(), 0);
                    assert_eq!(job.storage.reserve_attempts, before_reserves + 1);
                }
                DisplayHighlightPhase::Validate => {
                    assert_eq!((metrics.comparisons, metrics.output_attempts), (0, 0));
                    assert_eq!(cache.metrics().window_calls(), 0);
                }
            }
            if complete {
                return (job.storage.ranges.clone(), visits);
            }
        }
    }

    fn navigate(
        document: &Document,
        index: &SearchIndex,
        needle: &str,
        current: SearchRange,
        forward: bool,
    ) -> Option<SearchRange> {
        let mut navigation = index.navigation(current, forward).unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        loop {
            let advance = navigation.advance(&mut reader, needle).unwrap();
            if advance.completed() {
                return advance.selected();
            }
        }
    }

    fn navigate_with_block(
        document: &Document,
        index: &SearchIndex,
        needle: &str,
        current: SearchRange,
        forward: bool,
        block: MatchBlockCache,
    ) -> (Option<SearchRange>, MatchBlockCache, usize) {
        let mut navigation = index
            .navigation_with_block(current, forward, block)
            .unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        loop {
            let advance = navigation.advance(&mut reader, needle).unwrap();
            if advance.completed() {
                let scans = navigation.window_scans();
                return (advance.selected(), navigation.take_block(), scans);
            }
        }
    }

    fn settle_session(document: &Document, cache: &mut DocumentCache, session: &mut SearchSession) {
        while session.has_work() {
            let mut reader = document.reader(cache);
            session.advance(&mut reader).unwrap();
        }
    }

    fn complete_session(
        document: &Document,
        cache: &mut DocumentCache,
        query: &str,
    ) -> SearchSession {
        let mut session = {
            let reader = document.reader(cache);
            SearchSession::new(&reader, query.to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        settle_session(document, cache, &mut session);
        session
    }

    fn display_target(start: usize) -> SearchRange {
        SearchRange::new(
            SourceOffset::from_usize(start),
            SourceOffset::from_usize(start + 1),
        )
        .unwrap()
    }

    fn grapheme_targets(text: &str) -> Vec<SearchRange> {
        text.grapheme_indices(true)
            .filter(|(_, grapheme)| !grapheme.contains(['\r', '\n']))
            .map(|(start, grapheme)| {
                SearchRange::new(
                    SourceOffset::from_usize(start),
                    SourceOffset::from_usize(start + grapheme.len()),
                )
                .unwrap()
            })
            .collect()
    }

    fn display_key(visible: Range<SourceOffset>, target_count: usize) -> SearchHighlightKey {
        SearchHighlightKey::new(80, 24, visible.start, visible.end, target_count)
    }

    fn settle_highlights(
        document: &Document,
        cache: &mut DocumentCache,
        session: &mut SearchSession,
        key: SearchHighlightKey,
        mut target_at: impl FnMut(usize) -> SearchRange,
    ) -> Result<(), TutError> {
        while matches!(session.highlights, SearchHighlightState::Pending(_)) {
            let mut reader = document.reader(cache);
            session.advance_highlights(&mut reader, key, &mut target_at)?;
        }
        Ok(())
    }

    fn prepare_and_settle_highlights(
        document: &Document,
        cache: &mut DocumentCache,
        session: &mut SearchSession,
        visible: Range<SourceOffset>,
        target_count: usize,
        target_at: impl FnMut(usize) -> SearchRange,
    ) -> Result<SearchHighlightKey, TutError> {
        let key = display_key(visible, target_count);
        session.prepare_highlights(key);
        settle_highlights(document, cache, session, key, target_at)?;
        Ok(key)
    }

    fn request_session_navigation(
        document: &Document,
        cache: &mut DocumentCache,
        session: &mut SearchSession,
        forward: bool,
    ) -> bool {
        let mut reader = document.reader(cache);
        session.request_navigation(&mut reader, forward).unwrap()
    }

    #[test]
    fn sessions_make_equal_length_query_replacement_atomic() {
        let document = Document::from_text(Path::new("session.txt"), "cat dog dog cat".to_owned());
        let mut cache = DocumentCache::default();
        let mut cat = {
            let reader = document.reader(&mut cache);
            SearchSession::new(&reader, "cat".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        settle_session(&document, &mut cache, &mut cat);
        assert_eq!(cat.current_match().unwrap().start(), SourceOffset::ZERO);
        assert!(request_session_navigation(
            &document, &mut cache, &mut cat, true
        ));
        settle_session(&document, &mut cache, &mut cat);
        assert_eq!(cat.current_match().unwrap().start(), SourceOffset::new(12));
        assert!(cat.has_cached_block());

        let visible = document.source_start()..document.source_end();
        let target = SearchRange::new(visible.start, visible.end).unwrap();
        let key = prepare_and_settle_highlights(
            &document,
            &mut cache,
            &mut cat,
            visible.clone(),
            1,
            |_| target,
        )
        .unwrap();
        assert_eq!(cat.highlight_ranges(key), &[target]);

        let mut dog = {
            let reader = document.reader(&mut cache);
            SearchSession::new(&reader, "dog".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        assert_eq!(dog.query(), "dog");
        assert_eq!(dog.current_match(), None);
        assert_eq!(dog.highlight_ranges(display_key(visible, 1)), &[]);
        settle_session(&document, &mut cache, &mut dog);
        assert_eq!(dog.current_match().unwrap().start(), SourceOffset::new(4));
        assert!(request_session_navigation(
            &document, &mut cache, &mut dog, true
        ));
        settle_session(&document, &mut cache, &mut dog);
        assert_eq!(dog.current_match().unwrap().start(), SourceOffset::new(8));
    }

    #[test]
    fn session_constructor_enforces_query_boundaries() {
        let document = Document::from_text(Path::new("session.txt"), "body".to_owned());
        let mut cache = DocumentCache::default();
        let reader = document.reader(&mut cache);

        assert!(
            SearchSession::new(&reader, String::new(), SourceOffset::ZERO)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            SearchSession::new(
                &reader,
                "q".repeat(MAX_SEARCH_QUERY_BYTES + 1),
                SourceOffset::ZERO,
            ),
            Err(TutError::Search(SearchError::QueryTooLong {
                limit: MAX_SEARCH_QUERY_BYTES
            }))
        ));
    }

    #[test]
    fn sessions_reject_readers_from_another_document() {
        let first = Document::from_text(Path::new("first.txt"), "cat".to_owned());
        let second = Document::from_text(Path::new("second.txt"), "cat".to_owned());
        let mut first_cache = DocumentCache::default();
        let mut session = {
            let reader = first.reader(&mut first_cache);
            SearchSession::new(&reader, "cat".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        let mut second_cache = DocumentCache::default();
        let mut reader = second.reader(&mut second_cache);

        assert!(matches!(
            session.request_navigation(&mut reader, true),
            Err(TutError::Search(SearchError::SourceMismatch))
        ));
        assert!(matches!(
            session.advance(&mut reader),
            Err(TutError::Search(SearchError::SourceMismatch))
        ));
    }

    #[test]
    fn builds_a_global_nonoverlapping_index() {
        let (document, index) = complete("aaaaaa", "aa");
        assert!(index.is_complete());
        assert_eq!(
            exact_matches(&document, "aa"),
            vec![
                SearchRange::new(SourceOffset::new(0), SourceOffset::new(2)).unwrap(),
                SearchRange::new(SourceOffset::new(2), SourceOffset::new(4)).unwrap(),
                SearchRange::new(SourceOffset::new(4), SourceOffset::new(6)).unwrap(),
            ]
        );
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let (document, index) = complete("cat cat", "cat");
        let first = index.first_match.unwrap();
        let second = navigate(&document, &index, "cat", first, true).unwrap();
        assert_eq!(second.start(), SourceOffset::new(4));
        assert_eq!(
            navigate(&document, &index, "cat", second, true),
            Some(first)
        );
        assert_eq!(
            navigate(&document, &index, "cat", first, false),
            Some(second)
        );
    }

    #[test]
    fn search_work_rejects_another_document_with_the_same_range() {
        let (_document, index) = complete("cat cat", "cat");
        let current = index.first_match.unwrap();
        let mut navigation = index.navigation(current, true).unwrap();
        let other = Document::from_text(Path::new("other.txt"), "cat cat".to_owned());
        let mut cache = DocumentCache::default();
        let mut reader = other.reader(&mut cache);

        assert!(matches!(
            navigation.advance(&mut reader, "cat"),
            Err(TutError::Search(SearchError::SourceMismatch))
        ));
    }

    #[test]
    fn dense_navigation_reuses_one_compact_match_block() {
        let text = "a ".repeat(SOURCE_WINDOW_BYTES / 2);
        let (document, index) = complete(&text, "a");
        let mut current = index.first_match.unwrap();
        let mut block = MatchBlockCache::default();
        let mut forward_scans = 0;

        for match_index in 1..=1024 {
            let (selected, next_block, scans) =
                navigate_with_block(&document, &index, "a", current, true, block);
            current = selected.unwrap();
            block = next_block;
            forward_scans += scans;
            assert_eq!(current.start(), SourceOffset::from_usize(match_index * 2));
        }
        assert_eq!(forward_scans, 1);

        let mut backward_scans = 0;
        for match_index in (0..1024).rev() {
            let (selected, next_block, scans) =
                navigate_with_block(&document, &index, "a", current, false, block);
            current = selected.unwrap();
            block = next_block;
            backward_scans += scans;
            assert_eq!(current.start(), SourceOffset::from_usize(match_index * 2));
        }
        assert_eq!(backward_scans, 0);
    }

    #[test]
    fn cached_navigation_preserves_nonoverlapping_matches() {
        let text = "a".repeat(4096);
        let (document, index) = complete(&text, "aa");
        let mut current = index.first_match.unwrap();
        let mut block = MatchBlockCache::default();
        let mut scans = 0;

        for match_index in 1..=512 {
            let (selected, next_block, window_scans) =
                navigate_with_block(&document, &index, "aa", current, true, block);
            current = selected.unwrap();
            block = next_block;
            scans += window_scans;
            assert_eq!(current.start(), SourceOffset::from_usize(match_index * 2));
        }
        assert_eq!(scans, 1);
    }

    #[test]
    fn cached_navigation_crosses_windows_with_utf8_queries() {
        let needle = "é猫";
        let mut text = needle.to_owned();
        text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES - 2));
        let crossing = text.len();
        text.push_str(needle);
        text.push_str("--");
        let following = text.len();
        text.push_str(needle);
        let (document, index) = complete(&text, needle);
        let first = index.first_match.unwrap();

        let (second, block, first_scans) = navigate_with_block(
            &document,
            &index,
            needle,
            first,
            true,
            MatchBlockCache::default(),
        );
        let second = second.unwrap();
        assert_eq!(second.start(), SourceOffset::from_usize(crossing));
        assert_eq!(first_scans, 1);

        let (third, _, second_scans) =
            navigate_with_block(&document, &index, needle, second, true, block);
        assert_eq!(third.unwrap().start(), SourceOffset::from_usize(following));
        assert_eq!(second_scans, 1);
    }

    #[test]
    fn consecutive_dense_blocks_reuse_one_starts_allocation() {
        let document = Document::from_text(
            Path::new("dense-blocks.txt"),
            "a".repeat(SOURCE_WINDOW_BYTES * 2),
        );
        let mut document_cache = DocumentCache::default();
        let mut reader = document.reader(&mut document_cache);
        let document_id = reader.document_id();
        let source_end = reader.source_end();
        let mut cache = MatchBlockCache::default();

        cache
            .scan(
                &mut reader,
                "a",
                document_id,
                source_end,
                SourceOffset::ZERO,
                None,
            )
            .unwrap();
        let first = cache.ready().unwrap();
        assert_eq!(first.storage.starts.len(), SOURCE_WINDOW_BYTES);
        let allocation = first.storage.starts.as_ptr();
        let capacity = first.storage.starts.capacity();
        let reserve_attempts = first.storage.reserve_attempts;
        let second_start = first.next;
        let previous = first.last_or_previous();
        assert!(reserve_attempts > 0);

        cache
            .scan(
                &mut reader,
                "a",
                document_id,
                source_end,
                second_start,
                previous,
            )
            .unwrap();
        let second = cache.ready().unwrap();
        assert_eq!(second.storage.starts.len(), SOURCE_WINDOW_BYTES);
        assert_eq!(second.storage.starts.as_ptr(), allocation);
        assert_eq!(second.storage.starts.capacity(), capacity);
        assert_eq!(second.storage.reserve_attempts, reserve_attempts);
    }

    #[test]
    fn match_block_scan_failures_leave_only_cleared_spare_storage() {
        let document = Document::from_text(Path::new("failed-block.txt"), "aaaa".to_owned());
        let mut document_cache = DocumentCache::default();
        let mut reader = document.reader(&mut document_cache);
        let document_id = reader.document_id();
        let source_end = reader.source_end();
        let mut cache = MatchBlockCache::default();

        cache
            .scan(
                &mut reader,
                "a",
                document_id,
                source_end,
                SourceOffset::ZERO,
                None,
            )
            .unwrap();
        let ready = cache.ready().unwrap();
        let allocation = ready.storage.starts.as_ptr();
        let capacity = ready.storage.starts.capacity();
        let reserve_attempts = ready.storage.reserve_attempts;

        assert!(matches!(
            cache.scan(
                &mut reader,
                "a",
                document_id,
                SourceOffset::new(1),
                SourceOffset::ZERO,
                None,
            ),
            Err(TutError::Search(SearchError::NonIncreasingCursor { at: 0 }))
        ));
        let MatchBlockCache::Spare(storage) = &cache else {
            panic!("failed scans must not publish partial match blocks");
        };
        assert!(storage.starts.is_empty());
        assert_eq!(storage.starts.as_ptr(), allocation);
        assert_eq!(storage.starts.capacity(), capacity);
        assert_eq!(storage.reserve_attempts, reserve_attempts);
    }

    #[test]
    fn empty_queries_have_no_index_and_multibyte_queries_use_byte_ranges() {
        let document = Document::from_text(Path::new("search.txt"), "text".to_owned());
        let mut cache = DocumentCache::default();
        let reader = document.reader(&mut cache);
        assert!(
            SearchIndex::new(&reader, "", SourceOffset::ZERO)
                .unwrap()
                .is_none()
        );

        let (document, index) = complete("é-é", "é");
        let first = index.first_match.unwrap();
        assert_eq!(first.end().get() - first.start().get(), 2);
        assert_eq!(
            navigate(&document, &index, "é", first, true)
                .unwrap()
                .start(),
            SourceOffset::new(3)
        );
    }

    #[test]
    fn initial_checkpoint_capacity_follows_the_immutable_source_range() {
        let interval = INITIAL_CHECKPOINT_INTERVAL_BYTES;
        let limits = SearchIndexLimits::DEFAULT;
        let start = SourceOffset::new(3);
        let reservation = |source_len| {
            initial_checkpoint_reservation(
                start,
                SourceOffset::new(start.get() + source_len),
                limits,
            )
        };

        assert_eq!(reservation(0), 1);
        assert_eq!(reservation(interval - 1), 1);
        assert_eq!(reservation(interval), 2);
        assert_eq!(reservation(2 * interval - 1), 2);
        assert_eq!(reservation(MAX_FILE_BYTES), 513);
        assert_eq!(
            initial_checkpoint_reservation(
                SourceOffset::ZERO,
                SourceOffset::new(100),
                SearchIndexLimits::new(1, 4).unwrap(),
            ),
            4
        );
    }

    #[test]
    fn maximum_sources_complete_without_growing_the_checkpoint_allocation() {
        let source_len = usize::try_from(MAX_FILE_BYTES).unwrap();
        let document = Document::from_text(Path::new("maximum-search.txt"), "x".repeat(source_len));
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut index = SearchIndex::new(&reader, "z", SourceOffset::ZERO)
            .unwrap()
            .unwrap();

        while !index.is_complete() {
            index.advance(&mut reader, "z").unwrap();
        }

        assert_eq!(index.checkpoints.len(), 513);
        assert_eq!(index.checkpoint_reserve_attempts, 1);
    }

    #[test]
    fn search_layer_rejects_queries_beyond_its_memory_bound() {
        let document = Document::from_text(Path::new("search.txt"), "text".to_owned());
        let mut cache = DocumentCache::default();
        let reader = document.reader(&mut cache);
        let query = "q".repeat(MAX_SEARCH_QUERY_BYTES + 1);

        assert!(matches!(
            SearchIndex::new(&reader, &query, SourceOffset::ZERO),
            Err(TutError::Search(SearchError::QueryTooLong {
                limit: MAX_SEARCH_QUERY_BYTES
            }))
        ));
    }

    #[test]
    fn initial_selection_includes_matches_crossing_the_anchor() {
        let (_, index) = complete_at(
            "abcd--abcd",
            SourceOffset::ZERO,
            "abcd",
            SourceOffset::new(3),
        );
        assert_eq!(index.initial_selection.unwrap().start(), SourceOffset::ZERO);

        let (_, index) = complete_at(
            "abcd--abcd",
            SourceOffset::ZERO,
            "abcd",
            SourceOffset::new(4),
        );
        assert_eq!(
            index.initial_selection.unwrap().start(),
            SourceOffset::new(6)
        );
    }

    #[test]
    fn viewport_highlights_include_matches_crossing_its_start() {
        let (document, index) = complete("abcd--abcd", "abcd");
        let visible = matches(
            &document,
            &index,
            "abcd",
            SourceOffset::new(3)..SourceOffset::new(7),
        );
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn viewport_highlights_merge_adjacent_matches() {
        let (document, index) = complete("aaaaaa", "aa");
        assert_eq!(
            matches(
                &document,
                &index,
                "aa",
                SourceOffset::ZERO..SourceOffset::new(6)
            ),
            vec![SearchRange::new(SourceOffset::ZERO, SourceOffset::new(6)).unwrap()]
        );
    }

    #[test]
    fn cached_highlights_preserve_global_greedy_matches() {
        let (document, index) = complete("aaaaaa", "aa");
        let mut cache = DocumentCache::default();
        let mut block = MatchBlockCache::default();

        let mut first = index
            .highlights(SourceOffset::new(1)..SourceOffset::new(3))
            .unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(first.advance(&mut reader, "aa", &mut block).unwrap());
        }
        assert_eq!(
            first.ranges(),
            &[SearchRange::new(SourceOffset::ZERO, SourceOffset::new(4)).unwrap()]
        );

        let mut second = index
            .highlights(SourceOffset::new(3)..SourceOffset::new(5))
            .unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(second.advance(&mut reader, "aa", &mut block).unwrap());
        }
        assert_eq!(
            second.ranges(),
            &[SearchRange::new(SourceOffset::new(2), SourceOffset::new(6)).unwrap()]
        );
        assert_eq!(cache.metrics().window_calls(), 1);
    }

    #[test]
    fn cached_highlights_reuse_one_window_and_target_storage_across_contiguous_viewports() {
        const ROW_BYTES: usize = 160;
        const VIEWPORTS: usize = SOURCE_WINDOW_BYTES / ROW_BYTES;

        let directory = tempdir().unwrap();
        let path = directory.path().join("rows.txt");
        let mut text = String::with_capacity((VIEWPORTS + 1) * ROW_BYTES);
        for row in 0..=VIEWPORTS {
            text.push(if row % 7 == 0 { 'z' } else { 'x' });
            text.extend(std::iter::repeat_n('x', ROW_BYTES - 2));
            text.push('\n');
        }
        fs::write(&path, text).unwrap();
        let document = crate::document::load(path).unwrap();
        let mut cache = DocumentCache::default();
        let mut session = {
            let reader = document.reader(&mut cache);
            SearchSession::new(&reader, "z".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        settle_session(&document, &mut cache, &mut session);
        cache.reset_metrics();

        let mut target_allocation = None;
        let mut target_capacity = None;
        for row in 0..VIEWPORTS {
            let start = SourceOffset::from_usize(row * ROW_BYTES);
            let end = start.checked_add(ROW_BYTES).unwrap();
            let visible = start..end;
            let target = SearchRange::new(start, end).unwrap();
            let key = display_key(visible, 1);
            session.prepare_highlights(key);
            assert!(matches!(
                session.highlights,
                SearchHighlightState::Pending(_)
            ));
            assert!(session.highlight_ranges(key).is_empty());
            session.prepare_highlights(key);
            settle_highlights(&document, &mut cache, &mut session, key, |_| target).unwrap();
            let ready = session.highlights.ready().unwrap();
            let allocation = ready.storage.ranges.as_ptr();
            let capacity = ready.storage.ranges.capacity();
            assert_eq!(ready.storage.reserve_attempts, 1);
            assert_eq!(*target_allocation.get_or_insert(allocation), allocation);
            assert_eq!(*target_capacity.get_or_insert(capacity), capacity);
            let expected = if row % 7 == 0 { &[target][..] } else { &[] };
            assert_eq!(session.highlight_ranges(key), expected);
        }

        assert_eq!(VIEWPORTS, 409);
        assert_eq!(cache.metrics().window_calls(), 1);
    }

    #[test]
    fn cached_highlights_keep_utf8_boundary_matches_and_selection() {
        let needle = "é猫";
        let crossing = SOURCE_WINDOW_BYTES - "é".len();
        let mut text = "x".repeat(crossing);
        text.push_str(needle);
        let (document, index) = complete(&text, needle);
        let start = SourceOffset::from_usize(crossing);
        let after_first_character = start.checked_add("é".len()).unwrap();
        let end = start.checked_add(needle.len()).unwrap();
        let expected = SearchRange::new(start, end).unwrap();
        let mut cache = DocumentCache::default();
        let mut block = MatchBlockCache::default();

        let mut suffix = index.highlights(after_first_character..end).unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(suffix.advance(&mut reader, needle, &mut block).unwrap());
        }
        assert_eq!(suffix.ranges(), &[expected]);
        block.ready_mut().unwrap().selected = Some(0);
        cache.reset_metrics();

        let mut prefix = index.highlights(start..after_first_character).unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(prefix.advance(&mut reader, needle, &mut block).unwrap());
        }
        assert_eq!(prefix.ranges(), &[expected]);
        assert_eq!(block.ready().unwrap().selected, Some(0));
        assert_eq!(cache.metrics().window_calls(), 0);
    }

    #[test]
    fn cached_highlights_validate_tracked_files_before_reuse() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing.txt");
        fs::write(&path, "cat cat").unwrap();
        let document = crate::document::load(path.clone()).unwrap();
        let mut cache = DocumentCache::default();
        let index = {
            let mut reader = document.reader(&mut cache);
            let mut index = SearchIndex::new(&reader, "cat", SourceOffset::ZERO)
                .unwrap()
                .unwrap();
            while !index.is_complete() {
                index.advance(&mut reader, "cat").unwrap();
            }
            index
        };

        let visible = document.source_start()..document.source_end();
        let mut block = MatchBlockCache::default();
        let mut first = index.highlights(visible.clone()).unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(first.advance(&mut reader, "cat", &mut block).unwrap());
        }
        let ready = block.ready().unwrap();
        let allocation = ready.storage.starts.as_ptr();
        let capacity = ready.storage.starts.capacity();
        fs::write(path, "changed contents").unwrap();
        cache.reset_metrics();

        let mut stale = index.highlights(visible).unwrap();
        let result = {
            let mut reader = document.reader(&mut cache);
            stale.advance(&mut reader, "cat", &mut block)
        };
        assert!(matches!(result, Err(TutError::Load(_))));
        let ready = block.ready().unwrap();
        assert_eq!(ready.storage.starts.as_ptr(), allocation);
        assert_eq!(ready.storage.starts.capacity(), capacity);
        assert!(stale.ranges().is_empty());
        assert_eq!(cache.metrics().window_calls(), 0);
    }

    #[test]
    fn sessions_return_one_match_block_between_highlights_and_navigation() {
        let document = Document::from_text(Path::new("session.txt"), "cat cat".to_owned());
        let mut cache = DocumentCache::default();
        let mut session = {
            let reader = document.reader(&mut cache);
            SearchSession::new(&reader, "cat".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        settle_session(&document, &mut cache, &mut session);

        let whole = document.source_start()..document.source_end();
        let whole_target = SearchRange::new(whole.start, whole.end).unwrap();
        let whole_key =
            prepare_and_settle_highlights(&document, &mut cache, &mut session, whole, 1, |_| {
                whole_target
            })
            .unwrap();
        let allocation = session.match_block.ready().unwrap().storage.starts.as_ptr();
        cache.reset_metrics();

        assert!(request_session_navigation(
            &document,
            &mut cache,
            &mut session,
            true
        ));
        settle_session(&document, &mut cache, &mut session);
        assert_eq!(
            session.current_match().unwrap().start(),
            SourceOffset::new(4)
        );
        assert_eq!(
            session.match_block.ready().unwrap().storage.starts.as_ptr(),
            allocation
        );
        assert_eq!(session.highlight_ranges(whole_key), &[whole_target]);

        let second = SourceOffset::new(4)..SourceOffset::new(7);
        let second_target = SearchRange::new(second.start, second.end).unwrap();
        let second_key =
            prepare_and_settle_highlights(&document, &mut cache, &mut session, second, 1, |_| {
                second_target
            })
            .unwrap();
        assert_eq!(session.highlight_ranges(second_key), &[second_target]);
        assert_eq!(
            session.match_block.ready().unwrap().storage.starts.as_ptr(),
            allocation
        );

        assert!(request_session_navigation(
            &document,
            &mut cache,
            &mut session,
            false
        ));
        settle_session(&document, &mut cache, &mut session);
        assert_eq!(session.current_match().unwrap().start(), SourceOffset::ZERO);
        assert_eq!(
            session.match_block.ready().unwrap().storage.starts.as_ptr(),
            allocation
        );
        assert_eq!(cache.metrics().window_calls(), 0);
    }

    #[test]
    fn highlights_and_cross_window_navigation_move_one_starts_allocation() {
        let first_start = SOURCE_WINDOW_BYTES - 3;
        let second_start = SOURCE_WINDOW_BYTES + 5;
        let mut text = "x".repeat(first_start);
        text.push_str("cat");
        text.push_str(&"x".repeat(second_start - text.len()));
        text.push_str("cat");
        let document = Document::from_text(Path::new("cross-window.txt"), text);
        let mut document_cache = DocumentCache::default();
        let mut session = {
            let reader = document.reader(&mut document_cache);
            SearchSession::new(
                &reader,
                "cat".to_owned(),
                SourceOffset::from_usize(first_start),
            )
            .unwrap()
            .unwrap()
        };
        settle_session(&document, &mut document_cache, &mut session);
        assert_eq!(
            session.current_match().unwrap().start(),
            SourceOffset::from_usize(first_start)
        );

        let first_visible = SourceOffset::from_usize(first_start)
            ..SourceOffset::from_usize(first_start + "cat".len());
        let first_target = SearchRange::new(first_visible.start, first_visible.end).unwrap();
        let first_key = prepare_and_settle_highlights(
            &document,
            &mut document_cache,
            &mut session,
            first_visible,
            1,
            |_| first_target,
        )
        .unwrap();
        assert_eq!(session.highlight_ranges(first_key), &[first_target]);
        let first = session.match_block.ready().unwrap();
        let allocation = first.storage.starts.as_ptr();
        let capacity = first.storage.starts.capacity();
        let reserve_attempts = first.storage.reserve_attempts;

        document_cache.reset_metrics();
        assert!(request_session_navigation(
            &document,
            &mut document_cache,
            &mut session,
            true
        ));
        settle_session(&document, &mut document_cache, &mut session);
        assert_eq!(
            session.current_match().unwrap().start(),
            SourceOffset::from_usize(second_start)
        );
        let navigated = session.match_block.ready().unwrap();
        assert_eq!(navigated.storage.starts.as_ptr(), allocation);
        assert_eq!(navigated.storage.starts.capacity(), capacity);
        assert_eq!(navigated.storage.reserve_attempts, reserve_attempts);
        assert_eq!(navigated.selected, Some(0));
        assert_eq!(document_cache.metrics().window_calls(), 1);

        let second_visible = SourceOffset::from_usize(second_start)
            ..SourceOffset::from_usize(second_start + "cat".len());
        let second_target = SearchRange::new(second_visible.start, second_visible.end).unwrap();
        document_cache.reset_metrics();
        let second_key = prepare_and_settle_highlights(
            &document,
            &mut document_cache,
            &mut session,
            second_visible,
            1,
            |_| second_target,
        )
        .unwrap();
        assert_eq!(session.highlight_ranges(second_key), &[second_target]);
        let highlighted = session.match_block.ready().unwrap();
        assert_eq!(highlighted.storage.starts.as_ptr(), allocation);
        assert_eq!(highlighted.storage.starts.capacity(), capacity);
        assert_eq!(highlighted.storage.reserve_attempts, reserve_attempts);
        assert_eq!(highlighted.selected, Some(0));
        assert_eq!(document_cache.metrics().window_calls(), 0);
    }

    #[test]
    fn cancel_motion_returns_cross_window_match_block_storage() {
        let second_start = SOURCE_WINDOW_BYTES * 2 + 7;
        let mut text = "cat".to_owned();
        text.push_str(&"x".repeat(second_start - text.len()));
        text.push_str("cat");
        let document = Document::from_text(Path::new("cancel-cross-window.txt"), text);
        let mut document_cache = DocumentCache::default();
        let mut session = {
            let reader = document.reader(&mut document_cache);
            SearchSession::new(&reader, "cat".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        settle_session(&document, &mut document_cache, &mut session);

        let first_visible = SourceOffset::ZERO..SourceOffset::new(3);
        let first_target = SearchRange::new(first_visible.start, first_visible.end).unwrap();
        prepare_and_settle_highlights(
            &document,
            &mut document_cache,
            &mut session,
            first_visible,
            1,
            |_| first_target,
        )
        .unwrap();
        let first = session.match_block.ready().unwrap();
        let allocation = first.storage.starts.as_ptr();
        let capacity = first.storage.starts.capacity();
        let reserve_attempts = first.storage.reserve_attempts;
        assert_ne!(capacity, 0);

        document_cache.reset_metrics();
        assert!(request_session_navigation(
            &document,
            &mut document_cache,
            &mut session,
            true
        ));
        {
            let mut reader = document.reader(&mut document_cache);
            let step = session.advance(&mut reader).unwrap();
            assert!(!step.changed());
        }
        let active = session.navigation.as_ref().unwrap().block.ready().unwrap();
        assert_eq!(active.start, SourceOffset::from_usize(SOURCE_WINDOW_BYTES));
        assert_eq!(
            active.next,
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2)
        );
        assert!(active.storage.starts.is_empty());
        assert_eq!(active.storage.starts.as_ptr(), allocation);
        assert_eq!(active.storage.starts.capacity(), capacity);
        assert_eq!(active.storage.reserve_attempts, reserve_attempts);
        assert_eq!(document_cache.metrics().window_calls(), 1);

        assert!(session.cancel_motion());
        assert!(session.navigation.is_none());
        let returned = session.match_block.ready().unwrap();
        assert_eq!(returned.storage.starts.as_ptr(), allocation);
        assert_eq!(returned.storage.starts.capacity(), capacity);
        assert_eq!(returned.storage.reserve_attempts, reserve_attempts);

        document_cache.reset_metrics();
        assert!(request_session_navigation(
            &document,
            &mut document_cache,
            &mut session,
            true
        ));
        settle_session(&document, &mut document_cache, &mut session);
        assert_eq!(
            session.current_match().unwrap().start(),
            SourceOffset::from_usize(second_start)
        );
        let resumed = session.match_block.ready().unwrap();
        assert_eq!(resumed.storage.starts.as_ptr(), allocation);
        assert_eq!(resumed.storage.starts.capacity(), capacity);
        assert_eq!(resumed.storage.reserve_attempts, reserve_attempts);
        assert_eq!(document_cache.metrics().window_calls(), 1);
    }

    #[test]
    fn incompatible_blocks_become_spare_without_skipping_freshness() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("incompatible-block.txt");
        fs::write(&path, "cat").unwrap();
        let document = crate::document::load(path.clone()).unwrap();
        let mut document_cache = DocumentCache::default();
        let index = {
            let mut reader = document.reader(&mut document_cache);
            let mut index = SearchIndex::new(&reader, "cat", SourceOffset::ZERO)
                .unwrap()
                .unwrap();
            while !index.is_complete() {
                index.advance(&mut reader, "cat").unwrap();
            }
            index
        };
        let current = index.first_match.unwrap();
        let mut block = MatchBlockCache::default();
        {
            let mut reader = document.reader(&mut document_cache);
            block
                .scan(
                    &mut reader,
                    "c",
                    index.document_id,
                    index.source_end,
                    index.source_start,
                    None,
                )
                .unwrap();
        }
        let allocation = block.ready().unwrap().storage.starts.as_ptr();
        let mut navigation = index.navigation_with_block(current, true, block).unwrap();
        let MatchBlockCache::Spare(storage) = &navigation.block else {
            panic!("query-incompatible match data must become spare storage");
        };
        assert_eq!(storage.starts.as_ptr(), allocation);

        fs::write(path, "changed contents").unwrap();
        document_cache.reset_metrics();
        let result = {
            let mut reader = document.reader(&mut document_cache);
            navigation.advance(&mut reader, "cat")
        };
        assert!(matches!(result, Err(TutError::Load(_))));
        let MatchBlockCache::Spare(storage) = &navigation.block else {
            panic!("failed freshness checks must not publish match data");
        };
        assert_eq!(storage.starts.as_ptr(), allocation);
        assert_eq!(document_cache.metrics().window_calls(), 0);
    }

    #[test]
    fn viewport_highlights_advance_one_source_window_at_a_time() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2 + 17);
        let source_end = SourceOffset::from_usize(text.len());
        let (document, index) = complete(&text, "absent");
        let mut highlights = index.highlights(SourceOffset::ZERO..source_end).unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut block = MatchBlockCache::default();

        assert!(
            !highlights
                .advance(&mut reader, "absent", &mut block)
                .unwrap()
        );
        assert_eq!(
            highlights.cursor,
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES)
        );
        assert!(
            !highlights
                .advance(&mut reader, "absent", &mut block)
                .unwrap()
        );
        assert_eq!(
            highlights.cursor,
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2)
        );
        assert!(
            highlights
                .advance(&mut reader, "absent", &mut block)
                .unwrap()
        );
        assert!(highlights.is_complete());
    }

    #[test]
    fn streamed_display_highlights_match_the_oracle_at_merge_boundaries() {
        const COUNTS: [usize; 4] = [1, 1023, 1024, 1025];
        const QUERIES: [&str; 7] = [
            "A",
            "猫",
            "👩\u{200d}❤️\u{200d}💋\u{200d}👨",
            "e\u{301}",
            "\t",
            "\0",
            "needle",
        ];
        let pattern = "A猫👩\u{200d}❤️\u{200d}💋\u{200d}👨e\u{301}\t\0\r\nneedleneedle";

        for query in QUERIES {
            let text = format!("{query}{pattern}").repeat(180);
            let targets = grapheme_targets(&text);
            assert!(targets.len() >= 1025);
            let (document, index) = complete(&text, query);
            for count in COUNTS {
                let selected = &targets[..count];
                let visible = SourceOffset::ZERO..selected.last().unwrap().end();
                let oracle =
                    display_highlight_oracle(&document, &index, query, visible.clone(), selected);
                let (streamed, _) =
                    stream_display_highlights(&document, &index, query, visible, selected);
                assert_eq!(streamed, oracle, "query {query:?}, target count {count}");
            }
        }
    }

    #[test]
    fn streamed_display_highlights_keep_cross_window_queries() {
        let crossing = SOURCE_WINDOW_BYTES - 3;
        let mut text = "x".repeat(crossing);
        text.push_str("needle");
        text.push_str(&"y".repeat(2048));
        let (document, index) = complete(&text, "needle");
        let visible = SourceOffset::from_usize(crossing - 8)
            ..SourceOffset::from_usize(crossing + "needle".len() + 8);
        let targets = (crossing - 8..crossing + "needle".len() + 8)
            .map(display_target)
            .collect::<Vec<_>>();
        let oracle =
            display_highlight_oracle(&document, &index, "needle", visible.clone(), &targets);
        let (streamed, _) =
            stream_display_highlights(&document, &index, "needle", visible, &targets);
        assert_eq!(streamed, oracle);
    }

    #[test]
    fn exact_cap_highlights_are_streamed_and_over_cap_is_constant_work() {
        let text = "x\0".repeat(131_072);
        let targets = (0..MAX_DISPLAY_HIGHLIGHT_RANGES)
            .map(display_target)
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), MAX_DISPLAY_HIGHLIGHT_RANGES);
        let (document, index) = complete(&text, "x");
        let visible = document.source_start()..document.source_end();
        let oracle = display_highlight_oracle(&document, &index, "x", visible.clone(), &targets);
        let (streamed, visits) =
            stream_display_highlights(&document, &index, "x", visible.clone(), &targets);
        assert_eq!(streamed, oracle);
        assert_eq!(streamed.len(), MAX_DISPLAY_HIGHLIGHT_RANGES / 2);
        assert!(visits <= MAX_DISPLAY_HIGHLIGHT_RANGES * 2);

        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let exact_key = display_key(visible.clone(), MAX_DISPLAY_HIGHLIGHT_RANGES);
        session.prepare_highlights(exact_key);
        assert_eq!(session.highlight_reserve_attempts(), 0);
        assert_eq!(session.pending_highlight_ranges(), &[]);
        assert_eq!(
            session.pending_highlight_progress(),
            Some((DisplayHighlightPhase::NeedBlock, 0, 0, None))
        );
        session.invalidate_highlights();

        let over_key = display_key(visible, MAX_DISPLAY_HIGHLIGHT_RANGES + 1);
        session.prepare_highlights(over_key);
        assert!(matches!(
            session.highlights,
            SearchHighlightState::Disabled { .. }
        ));
        assert_eq!(session.highlight_reserve_attempts(), 0);
    }

    #[test]
    fn pending_display_ranges_publish_only_after_validation() {
        let document = Document::from_text(Path::new("atomic.txt"), "x x".to_owned());
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let visible = document.source_start()..document.source_end();
        let targets = [display_target(0), display_target(1), display_target(2)];
        let key = display_key(visible, targets.len());
        session.prepare_highlights(key);

        while let Some((phase, ..)) = session.pending_highlight_progress() {
            assert!(session.highlight_ranges(key).is_empty());
            let mut reader = document.reader(&mut cache);
            let published = session
                .advance_highlights(&mut reader, key, |index| targets[index])
                .unwrap();
            assert_eq!(published, phase == DisplayHighlightPhase::Validate);
        }
        assert_eq!(
            session.highlight_ranges(key),
            &[display_target(0), display_target(2)]
        );
    }

    #[test]
    fn display_key_changes_cancel_partial_work_and_reuse_storage() {
        let document = Document::from_text(Path::new("key-change.txt"), "x x".to_owned());
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let visible = document.source_start()..document.source_end();
        let targets = [display_target(0), display_target(1), display_target(2)];
        let first = display_key(visible.clone(), targets.len());
        session.prepare_highlights(first);
        while session.highlight_reserve_attempts() == 0 {
            let mut reader = document.reader(&mut cache);
            session
                .advance_highlights(&mut reader, first, |index| targets[index])
                .unwrap();
        }
        let SearchHighlightState::Pending(job) = &session.highlights else {
            panic!("the first key should remain pending");
        };
        let allocation = job.storage.ranges.as_ptr();
        let capacity = job.storage.ranges.capacity();
        assert_ne!(capacity, 0);

        let replacement =
            SearchHighlightKey::new(81, 24, visible.start, visible.end, targets.len());
        session.prepare_highlights(replacement);
        let SearchHighlightState::Pending(job) = &session.highlights else {
            panic!("the replacement key should restart highlighting");
        };
        assert_eq!(job.storage.ranges.as_ptr(), allocation);
        assert_eq!(job.storage.ranges.capacity(), capacity);
        assert!(job.storage.ranges.is_empty());
        assert_eq!((job.target_index, job.match_index), (0, 0));
        assert_eq!(job.phase, DisplayHighlightPhase::NeedBlock);
    }

    #[test]
    fn navigation_reclaims_an_unfinished_highlight_block_and_storage() {
        let document = Document::from_text(Path::new("reclaim.txt"), "x x".to_owned());
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let visible = document.source_start()..document.source_end();
        let targets = [display_target(0), display_target(1), display_target(2)];
        let key = display_key(visible, targets.len());
        session.prepare_highlights(key);
        while session.highlight_reserve_attempts() == 0 {
            let mut reader = document.reader(&mut cache);
            session
                .advance_highlights(&mut reader, key, |index| targets[index])
                .unwrap();
        }
        let block_allocation = session.match_block.ready().unwrap().storage.starts.as_ptr();
        let SearchHighlightState::Pending(job) = &session.highlights else {
            panic!("highlighting should remain unfinished");
        };
        let range_allocation = job.storage.ranges.as_ptr();
        let range_capacity = job.storage.ranges.capacity();

        assert!(request_session_navigation(
            &document,
            &mut cache,
            &mut session,
            true
        ));
        settle_session(&document, &mut cache, &mut session);
        assert_eq!(
            session.match_block.ready().unwrap().storage.starts.as_ptr(),
            block_allocation
        );
        session.prepare_highlights(key);
        let SearchHighlightState::Pending(job) = &session.highlights else {
            panic!("highlighting should restart after navigation");
        };
        assert_eq!(job.storage.ranges.as_ptr(), range_allocation);
        assert_eq!(job.storage.ranges.capacity(), range_capacity);
        assert!(job.storage.ranges.is_empty());
    }

    #[test]
    fn file_mutation_drops_partial_ranges_at_the_final_barrier() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("highlight-mutation.txt");
        fs::write(&path, "x x").unwrap();
        let document = crate::document::load(path.clone()).unwrap();
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let visible = document.source_start()..document.source_end();
        let targets = [display_target(0), display_target(1), display_target(2)];
        let key = display_key(visible, targets.len());
        session.prepare_highlights(key);
        while !matches!(
            session.pending_highlight_progress(),
            Some((DisplayHighlightPhase::Validate, ..))
        ) {
            let mut reader = document.reader(&mut cache);
            session
                .advance_highlights(&mut reader, key, |index| targets[index])
                .unwrap();
        }
        assert!(!session.pending_highlight_ranges().is_empty());
        assert!(session.highlight_ranges(key).is_empty());
        fs::write(path, "changed").unwrap();

        let result = {
            let mut reader = document.reader(&mut cache);
            session.advance_highlights(&mut reader, key, |index| targets[index])
        };
        assert!(matches!(result, Err(TutError::Load(_))));
        assert!(matches!(
            session.highlights,
            SearchHighlightState::Absent(_)
        ));
        assert!(session.highlight_ranges(key).is_empty());
    }

    #[test]
    fn display_highlights_use_visible_ranges_instead_of_dense_match_storage() {
        let match_count = SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES / size_of::<SearchRange>() + 1;
        let text = "ab".repeat(match_count);
        let source_end = SourceOffset::from_usize(text.len());
        let target = SearchRange::new(SourceOffset::ZERO, source_end).unwrap();
        let (document, index) = complete(&text, "a");
        let DisplayHighlightPreparation::Ready(mut highlights) = index
            .display_highlights(
                SourceOffset::ZERO..source_end,
                [target],
                SearchHighlightStorage::default(),
            )
            .unwrap()
        else {
            panic!("visible targets should prepare highlights");
        };
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut block = MatchBlockCache::default();

        while !highlights.is_complete() {
            highlights.advance(&mut reader, "a", &mut block).unwrap();
        }

        assert_eq!(highlights.ranges(), &[target]);
    }

    #[test]
    fn excessive_display_targets_disable_optional_highlights() {
        let target_count = MAX_DISPLAY_HIGHLIGHT_RANGES + 1;
        let text = "x".repeat(target_count);
        let source_end = SourceOffset::from_usize(text.len());
        let (_, index) = complete(&text, "z");
        let seed = SearchRange::new(SourceOffset::ZERO, SourceOffset::new(1)).unwrap();
        let DisplayHighlightPreparation::Ready(seed) = index
            .display_highlights(
                SourceOffset::ZERO..source_end,
                [seed],
                SearchHighlightStorage::default(),
            )
            .unwrap()
        else {
            panic!("the seed target should prepare highlights");
        };
        let storage = seed.into_storage();
        let targets = (0..target_count).filter(|_| true).map(display_target);

        let DisplayHighlightPreparation::Disabled(storage) = index
            .display_highlights(SourceOffset::ZERO..source_end, targets, storage)
            .unwrap()
        else {
            panic!("excessive targets should disable optional highlights");
        };
        assert!(storage.ranges.is_empty());
        assert!(storage.ranges.capacity() >= MAX_DISPLAY_HIGHLIGHT_RANGES);
        assert!(storage.reserve_attempts > 0);
    }

    #[test]
    fn sessions_cache_exact_overbudget_highlights_without_visiting_targets() {
        let target_count = MAX_DISPLAY_HIGHLIGHT_RANGES + 1;
        let document = Document::from_text(Path::new("overbudget.txt"), "x".repeat(target_count));
        let visible = document.source_start()..document.source_end();
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let key = display_key(visible.clone(), target_count);
        session.prepare_highlights(key);
        let SearchHighlightState::Disabled {
            key: disabled,
            storage,
        } = &session.highlights
        else {
            panic!("the overbudget viewport should be cached as disabled");
        };
        assert_eq!(*disabled, key);
        assert!(storage.ranges.is_empty());
        assert_eq!(storage.ranges.capacity(), 0);
        assert_eq!(storage.reserve_attempts, 0);
        session.prepare_highlights(key);
        assert!(session.highlight_ranges(key).is_empty());
    }

    #[test]
    fn disabled_highlights_retry_after_visible_changes_and_invalidation() {
        let target_count = MAX_DISPLAY_HIGHLIGHT_RANGES + 1;
        let document = Document::from_text(Path::new("overbudget.txt"), "x".repeat(target_count));
        let whole = document.source_start()..document.source_end();
        let small = SourceOffset::ZERO..SourceOffset::new(1);
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let whole_key = display_key(whole, target_count);
        session.prepare_highlights(whole_key);

        let small_key =
            prepare_and_settle_highlights(&document, &mut cache, &mut session, small, 1, |_| {
                display_target(0)
            })
            .unwrap();
        assert_eq!(session.highlight_ranges(small_key), &[display_target(0)]);
        assert!(matches!(session.highlights, SearchHighlightState::Ready(_)));

        session.prepare_highlights(whole_key);
        assert!(matches!(
            session.highlights,
            SearchHighlightState::Disabled { .. }
        ));
        session.invalidate_highlights();
        let retry_key = display_key(whole_key.visible(), 1);
        session.prepare_highlights(retry_key);
        settle_highlights(&document, &mut cache, &mut session, retry_key, |_| {
            display_target(0)
        })
        .unwrap();
        assert!(matches!(session.highlights, SearchHighlightState::Ready(_)));
    }

    #[test]
    fn incomplete_indexes_never_cache_highlight_budget_rejections() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES + 1);
        let document = Document::from_text(Path::new("incomplete.txt"), text);
        let visible = document.source_start()..document.source_end();
        let mut cache = DocumentCache::default();
        let mut session = {
            let reader = document.reader(&mut cache);
            SearchSession::new(&reader, "x".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        assert!(!session.index_complete());
        let disabled_key = display_key(visible.clone(), MAX_DISPLAY_HIGHLIGHT_RANGES + 1);
        session.prepare_highlights(disabled_key);
        assert!(matches!(
            session.highlights,
            SearchHighlightState::Absent(_)
        ));

        settle_session(&document, &mut cache, &mut session);
        let ready_key = display_key(visible, 1);
        session.prepare_highlights(ready_key);
        settle_highlights(&document, &mut cache, &mut session, ready_key, |_| {
            display_target(0)
        })
        .unwrap();
        assert!(matches!(session.highlights, SearchHighlightState::Ready(_)));
    }

    #[test]
    fn allocation_errors_never_disable_future_highlight_preparation() {
        let document = Document::from_text(Path::new("allocation.txt"), "x".to_owned());
        let visible = document.source_start()..document.source_end();
        let mut cache = DocumentCache::default();
        let mut session = complete_session(&document, &mut cache, "x");
        let SearchHighlightState::Absent(storage) = &mut session.highlights else {
            panic!("unprepared sessions should retain absent highlight storage");
        };
        storage.fail_reserve = true;
        let key = display_key(visible, 1);
        session.prepare_highlights(key);
        let result = loop {
            let mut reader = document.reader(&mut cache);
            match session.advance_highlights(&mut reader, key, |_| display_target(0)) {
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert!(matches!(result, TutError::Search(SearchError::Allocation)));
        assert!(matches!(
            session.highlights,
            SearchHighlightState::Absent(_)
        ));
        session.prepare_highlights(key);
        settle_highlights(&document, &mut cache, &mut session, key, |_| {
            display_target(0)
        })
        .unwrap();
        assert!(matches!(session.highlights, SearchHighlightState::Ready(_)));
    }

    #[test]
    fn matches_keep_absolute_offsets_above_u32() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 23);
        let (document, index) = complete_at("cat cat", start, "cat", start);
        let first = index.first_match.unwrap();
        let second = navigate(&document, &index, "cat", first, true).unwrap();

        assert_eq!(first.start(), start);
        assert_eq!(first.end(), start.checked_add(3).unwrap());
        assert_eq!(second.start(), start.checked_add(4).unwrap());
        assert_eq!(
            navigate(&document, &index, "cat", first, false),
            Some(second)
        );
    }

    #[test]
    fn matches_cross_source_window_boundaries() {
        let mut text = "x".repeat(SOURCE_WINDOW_BYTES - 2);
        text.push_str("needle tail needle");
        let (document, index) = complete(&text, "needle");
        let found = matches(
            &document,
            &index,
            "needle",
            SourceOffset::ZERO..SourceOffset::from_usize(text.len()),
        );

        assert_eq!(
            found[0].start(),
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES - 2)
        );
        assert_eq!(
            found[1].start(),
            found[0].end().checked_add(" tail ".len()).unwrap()
        );
    }

    #[test]
    fn maximum_length_queries_cross_source_windows() {
        let needle = "q".repeat(MAX_SEARCH_QUERY_BYTES);
        let mut text = "x".repeat(SOURCE_WINDOW_BYTES - 3);
        text.push_str(&needle);
        text.push('z');
        let (document, index) = complete(&text, &needle);
        let found = matches(
            &document,
            &index,
            &needle,
            SourceOffset::ZERO..SourceOffset::from_usize(text.len()),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].start(),
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES - 3)
        );
    }

    #[test]
    fn matches_at_and_overlapping_the_safe_boundary_are_found_once() {
        let mut exact = "x".repeat(SOURCE_WINDOW_BYTES);
        exact.push_str("needle");
        let (document, index) = complete(&exact, "needle");
        let found = matches(
            &document,
            &index,
            "needle",
            SourceOffset::ZERO..SourceOffset::from_usize(exact.len()),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].start(),
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES)
        );

        let mut crossing = "x".repeat(SOURCE_WINDOW_BYTES - 1);
        crossing.push_str("aaaaa");
        let (document, index) = complete(&crossing, "aaa");
        let found = matches(
            &document,
            &index,
            "aaa",
            SourceOffset::ZERO..SourceOffset::from_usize(crossing.len()),
        );
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].start(),
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES - 1)
        );
    }

    #[test]
    fn incomplete_indexes_do_not_create_viewport_highlights() {
        let mut text = "cat".to_owned();
        text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES * 2));
        text.push_str("cat");
        let document = Document::from_text(Path::new("search.txt"), text);
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut index = SearchIndex::new(&reader, "cat", SourceOffset::ZERO)
            .unwrap()
            .unwrap();
        index.advance(&mut reader, "cat").unwrap();

        assert!(
            index
                .highlights(SourceOffset::ZERO..document.source_end())
                .is_none()
        );
    }

    #[test]
    fn each_advance_scans_one_bounded_source_window() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2 + 17);
        let document = Document::from_text(Path::new("search.txt"), text);
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut index = SearchIndex::new(&reader, "absent", SourceOffset::ZERO)
            .unwrap()
            .unwrap();

        let first = index.advance(&mut reader, "absent").unwrap();
        assert!(!first.completed());
        assert_eq!(
            index.scanned_to,
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES)
        );
        let second = index.advance(&mut reader, "absent").unwrap();
        assert!(!second.completed());
        assert_eq!(
            index.scanned_to,
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES * 2)
        );
        assert!(index.advance(&mut reader, "absent").unwrap().completed());
    }

    #[test]
    fn checkpoint_memory_compacts_without_changing_navigation() {
        let mut text = String::new();
        for _ in 0..16 {
            text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES));
            text.push_str("cat");
        }
        let document = Document::from_text(Path::new("search.txt"), text);
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let limits = SearchIndexLimits::new(1, 4).unwrap();
        let mut index =
            SearchIndex::with_limits(&reader, "cat", SourceOffset::ZERO, limits).unwrap();
        while !index.is_complete() {
            index.advance(&mut reader, "cat").unwrap();
        }

        assert!(index.checkpoints.len() <= 4);
        assert!(index.checkpoint_interval_bytes > 1);
        let found = matches(
            &document,
            &index,
            "cat",
            document.source_start()..document.source_end(),
        );
        assert_eq!(found.len(), 16);
        let first = index.first_match.unwrap();
        assert_eq!(
            navigate(&document, &index, "cat", first, false).unwrap(),
            index.last_match.unwrap()
        );
        let last = index.last_match.unwrap();
        assert_eq!(
            navigate(&document, &index, "cat", last, false)
                .unwrap()
                .start(),
            SourceOffset::from_usize(
                14 * (SOURCE_WINDOW_BYTES + "cat".len()) + SOURCE_WINDOW_BYTES
            )
        );
    }
}
