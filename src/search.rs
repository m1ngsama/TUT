use std::{mem::size_of, num::NonZeroUsize, ops::Range};

use crate::{
    document::{DocumentId, DocumentReader, SOURCE_WINDOW_BYTES},
    error::{SearchError, TutError},
    source::SourceOffset,
};

const SEARCH_INDEX_MEMORY_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const INITIAL_CHECKPOINT_INTERVAL_BYTES: u64 = SOURCE_WINDOW_BYTES as u64;
const INITIAL_CHECKPOINT_RESERVATION: usize = 1024;
pub(super) const MAX_SEARCH_QUERY_BYTES: usize = 4096;
const MATCH_BLOCK_START_LIMIT: usize = SOURCE_WINDOW_BYTES + MAX_SEARCH_QUERY_BYTES;

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
    block: Option<MatchBlock>,
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
    query_len: usize,
    start: SourceOffset,
    next: SourceOffset,
    previous: Option<SearchRange>,
    starts: Vec<u32>,
    selected: Option<usize>,
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
    match_block: Option<MatchBlock>,
    pending_navigation: i64,
    highlights: Option<SearchHighlights>,
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
            match_block: None,
            pending_navigation: 0,
            highlights: None,
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
        self.is_searching()
            || matches!(&self.highlights, Some(highlights) if !highlights.is_complete())
    }

    pub(super) fn highlight_ranges(&self, visible: &Range<SourceOffset>) -> &[SearchRange] {
        self.highlights
            .as_ref()
            .filter(|highlights| highlights.covers(visible))
            .map_or(&[], SearchHighlights::ranges)
    }

    pub(super) fn prepare_highlights(
        &mut self,
        visible: Range<SourceOffset>,
        targets: impl IntoIterator<Item = SearchRange>,
    ) -> Result<(), TutError> {
        if self
            .highlights
            .as_ref()
            .is_some_and(|highlights| highlights.covers(&visible))
        {
            return Ok(());
        }
        let storage = self.highlights.take().map_or_else(
            SearchHighlightStorage::default,
            SearchHighlights::into_storage,
        );
        self.highlights = if self.index.is_complete() && self.index.has_matches() {
            self.index.display_highlights(visible, targets, storage)?
        } else {
            None
        };
        Ok(())
    }

    pub(super) fn invalidate_highlights(&mut self) {
        self.highlights = None;
    }

    pub(super) fn cancel_motion(&mut self) -> bool {
        let changed =
            self.navigation.is_some() || self.pending_navigation != 0 || self.jump_pending;
        if let Some(mut navigation) = self.navigation.take()
            && let Some(block) = navigation.take_block()
        {
            self.match_block = Some(block);
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
                    let Some(highlights) = self.highlights.as_mut() else {
                        return Ok(SearchStep {
                            changed: false,
                            jump: None,
                        });
                    };
                    return Ok(SearchStep {
                        changed: highlights.advance(
                            reader,
                            self.query.as_str(),
                            &mut self.match_block,
                        )?,
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
                self.navigation =
                    self.index
                        .navigation_with_block(current, forward, self.match_block.take());
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
                .and_then(SearchNavigation::take_block);
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
        self.match_block.is_some()
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
        self.navigation_with_block(current, forward, None)
    }

    fn navigation_with_block(
        &self,
        current: SearchRange,
        forward: bool,
        block: Option<MatchBlock>,
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
        let mut block = block;
        if block.as_mut().is_some_and(|block| {
            let contains = block.locate(current).is_some();
            !(contains || forward && block.previous == Some(current))
        }) {
            block = None;
        }
        let cursor = if forward {
            block.as_ref().map_or(current.end(), |block| block.start)
        } else {
            block.as_ref().map_or_else(
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
            block.as_ref().map_or_else(
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

    fn display_highlights(
        &self,
        visible: Range<SourceOffset>,
        targets: impl IntoIterator<Item = SearchRange>,
        mut storage: SearchHighlightStorage,
    ) -> Result<Option<SearchHighlights>, TutError> {
        if !self.highlightable(&visible) {
            return Ok(None);
        }
        storage.ranges.clear();
        for target in targets {
            debug_assert!(target.start() < visible.end && target.end() > visible.start);
            debug_assert!(
                storage
                    .ranges
                    .last()
                    .is_none_or(|last: &SearchRange| { last.end() <= target.start() })
            );
            if !reserve_display_target(&mut storage)? {
                return Ok(None);
            }
            storage.ranges.push(target);
        }
        Ok(self.make_highlights(
            visible,
            storage,
            HighlightMode::Display { read: 0, write: 0 },
        ))
    }

    fn highlightable(&self, visible: &Range<SourceOffset>) -> bool {
        self.complete
            && visible.start < visible.end
            && visible.start < self.source_end
            && visible.end > self.source_start
    }

    fn make_highlights(
        &self,
        visible: Range<SourceOffset>,
        storage: SearchHighlightStorage,
        mode: HighlightMode,
    ) -> Option<SearchHighlights> {
        if !self.highlightable(&visible) {
            return None;
        }
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
        Some(highlights)
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

impl MatchBlock {
    fn scan(
        reader: &mut DocumentReader<'_>,
        needle: &str,
        document_id: DocumentId,
        source_end: SourceOffset,
        start: SourceOffset,
        previous: Option<SearchRange>,
    ) -> Result<Self, TutError> {
        if reader.document_id() != document_id {
            return Err(SearchError::SourceMismatch.into());
        }
        let mut block = Self {
            query_len: needle.len(),
            start,
            next: start,
            previous,
            starts: Vec::new(),
            selected: None,
        };
        block.next = scan_window(reader, needle, start, source_end, |range| {
            block.push(range.start())
        })?;
        Ok(block)
    }

    fn push(&mut self, start: SourceOffset) -> Result<(), TutError> {
        if self.starts.len() >= MATCH_BLOCK_START_LIMIT {
            return Err(SearchError::Allocation.into());
        }
        let relative = start
            .get()
            .checked_sub(self.start.get())
            .and_then(|relative| u32::try_from(relative).ok())
            .ok_or(SearchError::CoordinateOverflow)?;
        if self.starts.len() == self.starts.capacity() {
            let remaining = MATCH_BLOCK_START_LIMIT - self.starts.len();
            let additional = self.starts.capacity().max(256).min(remaining);
            self.starts
                .try_reserve_exact(additional)
                .map_err(|_| SearchError::Allocation)?;
        }
        self.starts.push(relative);
        Ok(())
    }

    fn range(&self, index: usize) -> SearchRange {
        let relative = usize::try_from(self.starts[index])
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

    fn overlapping(&self, visible: &Range<SourceOffset>) -> impl Iterator<Item = SearchRange> + '_ {
        debug_assert_ne!(self.query_len, 0);
        let overlap = u64::try_from(self.query_len - 1).expect("query lengths fit u64");
        let earliest = visible.start.get().saturating_sub(overlap);
        let lower = earliest.saturating_sub(self.start.get());
        let upper = visible.end.get().saturating_sub(self.start.get());
        let first = self
            .starts
            .partition_point(|start| u64::from(*start) < lower);
        let last = self
            .starts
            .partition_point(|start| u64::from(*start) < upper);

        (first..last).map(|index| self.range(index))
    }

    fn locate(&mut self, range: SearchRange) -> Option<usize> {
        if let Some(index) = self.selected
            && self.range(index) == range
        {
            return Some(index);
        }
        let relative = range.start().get().checked_sub(self.start.get())?;
        let relative = u32::try_from(relative).ok()?;
        let index = self.starts.binary_search(&relative).ok()?;
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
        (!self.starts.is_empty()).then(|| self.select(0))
    }

    fn last_or_previous(&self) -> Option<SearchRange> {
        self.starts
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
        (index < self.starts.len()).then(|| self.select(index))
    }

    fn predecessor(&mut self, current: SearchRange) -> Option<SearchRange> {
        let index = self.locate(current)?;
        index.checked_sub(1).map(|index| self.select(index))
    }

    fn last_before(&mut self, before: SourceOffset) -> Option<SearchRange> {
        let relative = before.get().saturating_sub(self.start.get());
        let index = self
            .starts
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

    fn take_block(&mut self) -> Option<MatchBlock> {
        self.block.take()
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
        let reused_block = self.block.is_some();
        if reused_block {
            reader.validate()?;
        }
        if let Some(mut block) = self.take_block() {
            if let Some(selected) = block.successor(self.current) {
                self.block = Some(block);
                return Ok(SearchAdvance {
                    selected: Some(selected),
                    completed: true,
                });
            }
            self.cursor = block.next;
            self.previous = block.last_or_previous();
            if self.cursor >= self.source_end {
                block.clear_selection();
                self.block = Some(block);
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

        let mut block = MatchBlock::scan(
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
        self.cursor = block.next;
        self.previous = block.last_or_previous();
        let selected = block.first();
        let completed = selected.is_some() || self.cursor == self.source_end;
        if completed && selected.is_none() {
            block.clear_selection();
        }
        self.block = Some(block);
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
        let reused_block = self.block.is_some();
        if reused_block {
            reader.validate()?;
        }
        if let Some(mut block) = self.take_block() {
            if block.locate(self.current).is_some() {
                let selected = block.predecessor(self.current).or(block.previous);
                if selected.is_none() || selected == block.previous {
                    block.clear_selection();
                }
                self.block = Some(block);
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

        let mut block = MatchBlock::scan(
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
        self.cursor = block.next;
        self.previous = block.last_or_previous();
        if self.cursor < self.current.start() {
            self.block = Some(block);
            return Ok(SearchAdvance {
                selected: None,
                completed: false,
            });
        }

        let selected = block.last_before(self.current.start()).or(block.previous);
        if selected.is_none() || selected == block.previous {
            block.clear_selection();
        }
        self.block = Some(block);
        Ok(SearchAdvance {
            selected: selected.or(self.wrap),
            completed: true,
        })
    }
}

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
}

#[derive(Debug, Clone, Copy)]
enum HighlightMode {
    #[cfg(test)]
    Exact,
    Display {
        read: usize,
        write: usize,
    },
}

impl SearchHighlights {
    fn into_storage(self) -> SearchHighlightStorage {
        self.storage
    }

    const fn is_complete(&self) -> bool {
        self.complete
    }

    fn covers(&self, visible: &Range<SourceOffset>) -> bool {
        self.visible == *visible
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
        cached: &mut Option<MatchBlock>,
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

        let block = cached.take().filter(|block| {
            block.query_len == needle.len()
                && block.start <= self.needed_from
                && self.needed_from < block.next
        });
        let block = if let Some(block) = block {
            if let Err(error) = reader.validate() {
                *cached = Some(block);
                return Err(error.into());
            }
            block
        } else {
            MatchBlock::scan(
                reader,
                needle,
                self.document_id,
                self.source_end,
                self.cursor,
                self.previous,
            )?
        };
        let push_result = block
            .overlapping(&self.visible)
            .try_for_each(|range| self.push(range));
        if push_result.is_ok() {
            self.cursor = block.next;
            self.needed_from = block.next;
            self.previous = block.last_or_previous();
        }
        *cached = Some(block);
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

fn reserve_display_target(storage: &mut SearchHighlightStorage) -> Result<bool, TutError> {
    let ranges = &mut storage.ranges;
    let max_ranges = SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES / size_of::<SearchRange>();
    if ranges.len() >= max_ranges {
        return Ok(false);
    }
    if ranges.len() == ranges.capacity() {
        #[cfg(test)]
        {
            storage.reserve_attempts = storage.reserve_attempts.saturating_add(1);
        }
        let remaining = max_ranges - ranges.len();
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
        let mut block = None;
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
        block: Option<MatchBlock>,
    ) -> (Option<SearchRange>, Option<MatchBlock>, usize) {
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
        cat.prepare_highlights(visible.clone(), [target]).unwrap();
        settle_session(&document, &mut cache, &mut cat);
        assert_eq!(cat.highlight_ranges(&visible), &[target]);

        let mut dog = {
            let reader = document.reader(&mut cache);
            SearchSession::new(&reader, "dog".to_owned(), SourceOffset::ZERO)
                .unwrap()
                .unwrap()
        };
        assert_eq!(dog.query(), "dog");
        assert_eq!(dog.current_match(), None);
        assert_eq!(dog.highlight_ranges(&visible), &[]);
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
        let mut block = None;
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
        let mut block = None;
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

        let (second, block, first_scans) =
            navigate_with_block(&document, &index, needle, first, true, None);
        let second = second.unwrap();
        assert_eq!(second.start(), SourceOffset::from_usize(crossing));
        assert_eq!(first_scans, 1);

        let (third, _, second_scans) =
            navigate_with_block(&document, &index, needle, second, true, block);
        assert_eq!(third.unwrap().start(), SourceOffset::from_usize(following));
        assert_eq!(second_scans, 1);
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
        let mut block = None;

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
            session
                .prepare_highlights(visible.clone(), [target])
                .unwrap();
            let pending = session.highlights.as_ref().unwrap();
            assert!(!pending.is_complete());
            let allocation = pending.storage.ranges.as_ptr();
            let capacity = pending.storage.ranges.capacity();
            assert_eq!(pending.storage.reserve_attempts, 1);
            assert_eq!(*target_allocation.get_or_insert(allocation), allocation);
            assert_eq!(*target_capacity.get_or_insert(capacity), capacity);

            session
                .prepare_highlights(visible.clone(), std::iter::once_with(|| unreachable!()))
                .unwrap();
            let unchanged = session.highlights.as_ref().unwrap();
            assert_eq!(unchanged.storage.ranges.as_ptr(), allocation);
            assert_eq!(unchanged.storage.ranges.capacity(), capacity);
            assert_eq!(unchanged.storage.reserve_attempts, 1);
            assert!(!unchanged.is_complete());

            settle_session(&document, &mut cache, &mut session);
            let expected = if row % 7 == 0 { &[target][..] } else { &[] };
            assert_eq!(session.highlight_ranges(&visible), expected);
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
        let mut block = None;

        let mut suffix = index.highlights(after_first_character..end).unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(suffix.advance(&mut reader, needle, &mut block).unwrap());
        }
        assert_eq!(suffix.ranges(), &[expected]);
        block.as_mut().unwrap().selected = Some(0);
        cache.reset_metrics();

        let mut prefix = index.highlights(start..after_first_character).unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(prefix.advance(&mut reader, needle, &mut block).unwrap());
        }
        assert_eq!(prefix.ranges(), &[expected]);
        assert_eq!(block.as_ref().unwrap().selected, Some(0));
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
        let mut block = None;
        let mut first = index.highlights(visible.clone()).unwrap();
        {
            let mut reader = document.reader(&mut cache);
            assert!(first.advance(&mut reader, "cat", &mut block).unwrap());
        }
        assert!(block.is_some());
        fs::write(path, "changed contents").unwrap();
        cache.reset_metrics();

        let mut stale = index.highlights(visible).unwrap();
        let result = {
            let mut reader = document.reader(&mut cache);
            stale.advance(&mut reader, "cat", &mut block)
        };
        assert!(matches!(result, Err(TutError::Load(_))));
        assert!(block.is_some());
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
        session.prepare_highlights(whole, [whole_target]).unwrap();
        settle_session(&document, &mut cache, &mut session);
        let allocation = session.match_block.as_ref().unwrap().starts.as_ptr();
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
            session.match_block.as_ref().unwrap().starts.as_ptr(),
            allocation
        );

        let second = SourceOffset::new(4)..SourceOffset::new(7);
        let second_target = SearchRange::new(second.start, second.end).unwrap();
        session
            .prepare_highlights(second.clone(), [second_target])
            .unwrap();
        settle_session(&document, &mut cache, &mut session);
        assert_eq!(session.highlight_ranges(&second), &[second_target]);
        assert_eq!(
            session.match_block.as_ref().unwrap().starts.as_ptr(),
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
            session.match_block.as_ref().unwrap().starts.as_ptr(),
            allocation
        );
        assert_eq!(cache.metrics().window_calls(), 0);
    }

    #[test]
    fn viewport_highlights_advance_one_source_window_at_a_time() {
        let text = "x".repeat(SOURCE_WINDOW_BYTES * 2 + 17);
        let source_end = SourceOffset::from_usize(text.len());
        let (document, index) = complete(&text, "absent");
        let mut highlights = index.highlights(SourceOffset::ZERO..source_end).unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut block = None;

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
    fn display_highlights_use_visible_ranges_instead_of_dense_match_storage() {
        let match_count = SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES / size_of::<SearchRange>() + 1;
        let text = "ab".repeat(match_count);
        let source_end = SourceOffset::from_usize(text.len());
        let target = SearchRange::new(SourceOffset::ZERO, source_end).unwrap();
        let (document, index) = complete(&text, "a");
        let mut highlights = index
            .display_highlights(
                SourceOffset::ZERO..source_end,
                [target],
                SearchHighlightStorage::default(),
            )
            .unwrap()
            .unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut block = None;

        while !highlights.is_complete() {
            highlights.advance(&mut reader, "a", &mut block).unwrap();
        }

        assert_eq!(highlights.ranges(), &[target]);
    }

    #[test]
    fn excessive_display_targets_disable_optional_highlights() {
        let target_count = SEARCH_HIGHLIGHT_MEMORY_BUDGET_BYTES / size_of::<SearchRange>() + 1;
        let text = "x".repeat(target_count);
        let source_end = SourceOffset::from_usize(text.len());
        let (_, index) = complete(&text, "z");
        let seed = SearchRange::new(SourceOffset::ZERO, SourceOffset::new(1)).unwrap();
        let storage = index
            .display_highlights(
                SourceOffset::ZERO..source_end,
                [seed],
                SearchHighlightStorage::default(),
            )
            .unwrap()
            .unwrap()
            .into_storage();
        let targets = (0..target_count).map(|start| {
            SearchRange::new(
                SourceOffset::from_usize(start),
                SourceOffset::from_usize(start + 1),
            )
            .unwrap()
        });

        assert!(
            index
                .display_highlights(SourceOffset::ZERO..source_end, targets, storage,)
                .unwrap()
                .is_none()
        );
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
