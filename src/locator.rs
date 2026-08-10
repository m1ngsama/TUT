use std::{collections::VecDeque, num::NonZeroUsize};

use crate::{
    document::{DocumentId, DocumentReader},
    error::TutError,
    layout::{
        BodyHeight, ContentWidth, ProjectedRowsResult, ProjectedScanAdvance, ProjectedScanMeter,
        ViewportLayout, VisualRowScanner,
    },
    source::SourceOffset,
};

const INITIAL_ROW_NEIGHBORHOOD_CAPACITY: usize = 64;
const ROW_NEIGHBORHOOD_CAPACITY: usize = 4096;

pub(super) fn source_row_bound(
    source_start: SourceOffset,
    source_end: SourceOffset,
) -> NonZeroUsize {
    debug_assert!(source_start <= source_end);
    let source_bytes = source_end.get().saturating_sub(source_start.get());
    let bound = usize::try_from(source_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    NonZeroUsize::new(bound).expect("source row bounds are nonzero")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowDelta {
    Backward(usize),
    Forward(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LocatedViewport {
    pub(super) anchor: SourceOffset,
    pub(super) at_end: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RowEdge {
    start: SourceOffset,
    next: Option<SourceOffset>,
}

#[derive(Debug, Default)]
pub(super) struct RowNeighborhood {
    key: Option<(DocumentId, ContentWidth)>,
    edges: VecDeque<RowEdge>,
}

impl RowNeighborhood {
    pub(super) fn clear(&mut self) {
        self.key = None;
        self.edges.clear();
    }

    pub(super) fn locate_target(
        &self,
        key: (DocumentId, ContentWidth),
        source_start: SourceOffset,
        source_end: SourceOffset,
        target: SourceOffset,
        delta: RowDelta,
        height: BodyHeight,
    ) -> Option<LocatedViewport> {
        if self.key != Some(key) || target < source_start || target > source_end {
            return None;
        }
        let target_index = match self.edges.binary_search_by_key(&target, |edge| edge.start) {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let target_edge = self.edges.get(target_index)?;
        if target_edge.next.is_some_and(|next| target >= next) {
            return None;
        }
        let candidate_index = match delta {
            RowDelta::Backward(amount) => {
                if amount <= target_index {
                    target_index - amount
                } else if self.edges.front()?.start == source_start {
                    0
                } else {
                    return None;
                }
            }
            RowDelta::Forward(amount) => {
                let mut index = target_index;
                for _ in 0..amount {
                    let Some(next) = self.edges.get(index)?.next else {
                        break;
                    };
                    index += 1;
                    if self.edges.get(index)?.start != next {
                        return None;
                    }
                }
                index
            }
        };
        self.clamp(candidate_index, source_start, usize::from(height.get()))
    }

    fn clamp(
        &self,
        candidate_index: usize,
        source_start: SourceOffset,
        height: usize,
    ) -> Option<LocatedViewport> {
        let mut index = candidate_index;
        for visible in 1..=height {
            let edge = self.edges.get(index)?;
            let Some(next) = edge.next else {
                let first = (index + 1).saturating_sub(height);
                if first == 0 && self.edges.front()?.start != source_start {
                    return None;
                }
                return Some(LocatedViewport {
                    anchor: self.edges.get(first)?.start,
                    at_end: true,
                });
            };
            if visible == height {
                return Some(LocatedViewport {
                    anchor: self.edges.get(candidate_index)?.start,
                    at_end: false,
                });
            }
            index += 1;
            if self.edges.get(index)?.start != next {
                return None;
            }
        }
        None
    }

    fn observe(
        &mut self,
        key: (DocumentId, ContentWidth),
        start: SourceOffset,
        next: Option<SourceOffset>,
    ) -> Result<(), TutError> {
        debug_assert!(next.is_none_or(|next| next > start));
        if self.key != Some(key) {
            self.clear();
            self.key = Some(key);
        }
        let edge = RowEdge { start, next };
        if let Ok(index) = self.edges.binary_search_by_key(&start, |edge| edge.start) {
            if self.edges[index] == edge {
                return Ok(());
            }
            self.edges.clear();
        } else if self
            .edges
            .back()
            .is_some_and(|last| last.next == Some(start))
        {
            return self.push_back(edge);
        } else if next
            .is_some_and(|next| self.edges.front().is_some_and(|first| first.start == next))
        {
            return self.push_front(edge);
        } else {
            self.edges.clear();
        }
        self.push_back(edge)
    }

    fn reserve(&mut self) -> Result<(), TutError> {
        if self.edges.len() < self.edges.capacity() || self.edges.len() == ROW_NEIGHBORHOOD_CAPACITY
        {
            return Ok(());
        }
        let target = if self.edges.capacity() == 0 {
            INITIAL_ROW_NEIGHBORHOOD_CAPACITY
        } else {
            self.edges
                .capacity()
                .saturating_mul(2)
                .min(ROW_NEIGHBORHOOD_CAPACITY)
        };
        self.edges
            .try_reserve_exact(target - self.edges.len())
            .map_err(|_| TutError::Allocation("visual row neighborhood"))
    }

    fn push_back(&mut self, edge: RowEdge) -> Result<(), TutError> {
        if self.edges.len() == ROW_NEIGHBORHOOD_CAPACITY {
            self.edges.pop_front();
        }
        self.reserve()?;
        self.edges.push_back(edge);
        Ok(())
    }

    fn push_front(&mut self, edge: RowEdge) -> Result<(), TutError> {
        if self.edges.len() == ROW_NEIGHBORHOOD_CAPACITY {
            self.edges.pop_back();
        }
        self.reserve()?;
        self.edges.push_front(edge);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefixResume {
    SelectBackward { amount: usize },
    FinishAtEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Locate {
        cursor: SourceOffset,
    },
    MoveBackward {
        amount: usize,
    },
    MoveForward {
        cursor: SourceOffset,
        remaining: usize,
    },
    Clamp {
        candidate: SourceOffset,
        cursor: SourceOffset,
        visible: usize,
    },
    Prepend {
        required: usize,
        resume: PrefixResume,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrefixScan {
    line_start: SourceOffset,
    line_end: SourceOffset,
    cursor: SourceOffset,
}

struct PendingRowScan {
    start: SourceOffset,
    scanner: VisualRowScanner,
}

pub(super) struct ViewportLocator {
    target: SourceOffset,
    delta: RowDelta,
    height: usize,
    phase: Phase,
    history: VecDeque<SourceOffset>,
    prefix: Option<PrefixScan>,
    prefix_rows: VecDeque<SourceOffset>,
    row_scan: Option<PendingRowScan>,
    capacity: usize,
}

impl ViewportLocator {
    pub(super) fn new(
        target: SourceOffset,
        delta: RowDelta,
        height: BodyHeight,
        row_bound: NonZeroUsize,
    ) -> Result<Self, TutError> {
        let height = usize::from(height.get());
        let backward_rows = match delta {
            RowDelta::Backward(amount) => amount.saturating_add(1),
            RowDelta::Forward(_) => 1,
        };
        let capacity = height.max(backward_rows).min(row_bound.get());
        let mut history = VecDeque::new();
        let mut prefix_rows = VecDeque::new();
        history
            .try_reserve_exact(capacity)
            .map_err(|_| TutError::Allocation("viewport locator row starts"))?;
        prefix_rows
            .try_reserve_exact(capacity)
            .map_err(|_| TutError::Allocation("viewport locator row starts"))?;
        Ok(Self {
            target,
            delta,
            height,
            phase: Phase::Start,
            history,
            prefix: None,
            prefix_rows,
            row_scan: None,
            capacity,
        })
    }

    pub(super) fn from_row_start(
        target: SourceOffset,
        delta: RowDelta,
        height: BodyHeight,
        row_bound: NonZeroUsize,
    ) -> Result<Self, TutError> {
        let mut locator = Self::new(target, delta, height, row_bound)?;
        locator.push_history(target);
        locator.begin_movement(target);
        Ok(locator)
    }

    pub(super) fn advance(
        &mut self,
        layout: &ViewportLayout,
        reader: &mut DocumentReader<'_>,
        neighborhood: &mut RowNeighborhood,
    ) -> Result<Option<LocatedViewport>, TutError> {
        let row_key = layout.row_cache_key();
        let mut meter = ProjectedScanMeter::standard();
        loop {
            if meter.exhausted() {
                return Ok(None);
            }
            match self.phase {
                Phase::Start => {
                    let cursor = reader.line_start_at_or_before(self.target)?;
                    self.phase = Phase::Locate { cursor };
                }
                Phase::Locate { cursor } => {
                    let Some(row) = self.advance_row(layout, reader, &mut meter, cursor)? else {
                        return Ok(None);
                    };
                    let next = row.next;
                    neighborhood.observe(row_key, cursor, next)?;
                    self.push_history(cursor);
                    if let Some(next) = next.filter(|next| *next <= self.target) {
                        self.phase = Phase::Locate { cursor: next };
                    } else {
                        self.begin_movement(cursor);
                    }
                }
                Phase::MoveBackward { amount } => {
                    if let Some(candidate) = self.select_backward(amount, reader.source_start()) {
                        self.begin_clamp(candidate);
                    } else {
                        self.phase = Phase::Prepend {
                            required: amount.saturating_add(1).min(self.capacity),
                            resume: PrefixResume::SelectBackward { amount },
                        };
                    }
                }
                Phase::MoveForward { cursor, remaining } => {
                    if remaining == 0 {
                        self.begin_clamp(cursor);
                        continue;
                    }
                    let Some(row) = self.advance_row(layout, reader, &mut meter, cursor)? else {
                        return Ok(None);
                    };
                    let next = row.next;
                    neighborhood.observe(row_key, cursor, next)?;
                    if let Some(next) = next {
                        self.push_history(next);
                        self.phase = Phase::MoveForward {
                            cursor: next,
                            remaining: remaining - 1,
                        };
                    } else {
                        self.begin_clamp(cursor);
                    }
                }
                Phase::Clamp {
                    candidate,
                    cursor,
                    visible,
                } => {
                    let Some(row) = self.advance_row(layout, reader, &mut meter, cursor)? else {
                        return Ok(None);
                    };
                    let next = row.next;
                    neighborhood.observe(row_key, cursor, next)?;
                    let visible = visible + 1;
                    match next {
                        None => {
                            if let Some(located) = self.finish_at_end(reader.source_start()) {
                                return Ok(Some(located));
                            }
                            self.phase = Phase::Prepend {
                                required: self.height,
                                resume: PrefixResume::FinishAtEnd,
                            };
                        }
                        Some(_) if visible == self.height => {
                            return Ok(Some(LocatedViewport {
                                anchor: candidate,
                                at_end: false,
                            }));
                        }
                        Some(next) => {
                            self.push_history(next);
                            self.phase = Phase::Clamp {
                                candidate,
                                cursor: next,
                                visible,
                            };
                        }
                    }
                }
                Phase::Prepend { required, resume } => {
                    if self.history.len() >= required
                        || self.history.front().copied() == Some(reader.source_start())
                    {
                        if let Some(located) =
                            self.resume_after_prefix(resume, reader.source_start())
                        {
                            return Ok(Some(located));
                        }
                        continue;
                    }
                    if self.prefix.is_none() {
                        let line_end = *self
                            .history
                            .front()
                            .expect("viewport locators retain located row starts");
                        let probe = reader
                            .previous_char_start(line_end)?
                            .expect("non-initial physical lines have preceding characters");
                        let line_start = reader.line_start_at_or_before(probe)?;
                        self.prefix = Some(PrefixScan {
                            line_start,
                            line_end,
                            cursor: line_start,
                        });
                        self.prefix_rows.clear();
                    }

                    let prefix = self.prefix.expect("prefix scans were initialized");
                    let remaining = required - self.history.len();
                    let Some(row) = self.advance_row(layout, reader, &mut meter, prefix.cursor)?
                    else {
                        return Ok(None);
                    };
                    let next = row.next;
                    neighborhood.observe(row_key, prefix.cursor, next)?;
                    if self.prefix_rows.len() == remaining {
                        self.prefix_rows.pop_front();
                    }
                    self.prefix_rows.push_back(prefix.cursor);
                    if let Some(next) = next.filter(|next| *next < prefix.line_end) {
                        self.prefix = Some(PrefixScan {
                            cursor: next,
                            ..prefix
                        });
                        continue;
                    }

                    while let Some(row) = self.prefix_rows.pop_back() {
                        self.history.push_front(row);
                    }
                    self.prefix = None;
                    if (self.history.len() >= required
                        || prefix.line_start == reader.source_start())
                        && let Some(located) =
                            self.resume_after_prefix(resume, reader.source_start())
                    {
                        return Ok(Some(located));
                    }
                }
            }
        }
    }

    fn advance_row(
        &mut self,
        layout: &ViewportLayout,
        reader: &mut DocumentReader<'_>,
        meter: &mut ProjectedScanMeter,
        start: SourceOffset,
    ) -> Result<Option<ProjectedRowsResult>, TutError> {
        let scanner = if let Some(pending) = self.row_scan.take() {
            debug_assert_eq!(pending.start, start);
            pending.scanner
        } else {
            layout.start_row_scan(reader, start)?
        };
        match scanner.advance(reader, meter)? {
            ProjectedScanAdvance::Pending(scanner) => {
                self.row_scan = Some(PendingRowScan { start, scanner });
                Ok(None)
            }
            ProjectedScanAdvance::Complete { result, sink } => {
                let _ = sink;
                debug_assert_eq!(result.rows, 1);
                Ok(Some(result))
            }
        }
    }

    fn push_history(&mut self, row: SourceOffset) {
        if self.history.len() == self.capacity {
            self.history.pop_front();
        }
        self.history.push_back(row);
    }

    fn begin_movement(&mut self, target_row: SourceOffset) {
        self.phase = match self.delta {
            RowDelta::Backward(amount) if amount > 0 => Phase::MoveBackward { amount },
            RowDelta::Forward(amount) if amount > 0 => Phase::MoveForward {
                cursor: target_row,
                remaining: amount,
            },
            RowDelta::Backward(_) | RowDelta::Forward(_) => {
                self.begin_clamp(target_row);
                return;
            }
        };
    }

    fn select_backward(
        &mut self,
        amount: usize,
        source_start: SourceOffset,
    ) -> Option<SourceOffset> {
        if self.history.len() > amount {
            let index = self.history.len() - amount - 1;
            let candidate = self.history[index];
            self.history.truncate(index + 1);
            return Some(candidate);
        }
        if self.history.front().copied() == Some(source_start) {
            self.history.truncate(1);
            return self.history.front().copied();
        }
        None
    }

    fn begin_clamp(&mut self, candidate: SourceOffset) {
        self.phase = Phase::Clamp {
            candidate,
            cursor: candidate,
            visible: 0,
        };
    }

    fn finish_at_end(&self, source_start: SourceOffset) -> Option<LocatedViewport> {
        if self.history.len() < self.height && self.history.front().copied() != Some(source_start) {
            return None;
        }
        let index = self.history.len().saturating_sub(self.height);
        Some(LocatedViewport {
            anchor: self.history[index],
            at_end: true,
        })
    }

    fn resume_after_prefix(
        &mut self,
        resume: PrefixResume,
        source_start: SourceOffset,
    ) -> Option<LocatedViewport> {
        match resume {
            PrefixResume::SelectBackward { amount } => {
                let candidate = self
                    .select_backward(amount, source_start)
                    .expect("completed prefix scans satisfy backward selection");
                self.begin_clamp(candidate);
                None
            }
            PrefixResume::FinishAtEnd => Some(
                self.finish_at_end(source_start)
                    .expect("completed prefix scans satisfy end clamping"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::*;
    use crate::{
        document::{Document, DocumentCache, SOURCE_WINDOW_BYTES, load},
        layout::rebuild_viewport_layout,
    };

    fn offset(value: u64) -> SourceOffset {
        SourceOffset::new(value)
    }

    fn row_key(width: u16) -> (DocumentId, ContentWidth) {
        let document = Document::from_text(Path::new("rows.txt"), String::new());
        let mut cache = DocumentCache::default();
        let reader = document.reader(&mut cache);
        (reader.document_id(), ContentWidth::new(width).unwrap())
    }

    fn viewport_layout(
        document: &Document,
        cache: &mut DocumentCache,
        width: u16,
        height: u16,
    ) -> ViewportLayout {
        let reader = document.reader(cache);
        let mut layout = None;
        rebuild_viewport_layout(
            &mut layout,
            &reader,
            ContentWidth::new(width).unwrap(),
            BodyHeight::new(height).unwrap(),
        );
        layout.unwrap()
    }

    fn row_bound(document: &Document) -> NonZeroUsize {
        source_row_bound(document.source_start(), document.source_end())
    }

    fn advance(
        locator: &mut ViewportLocator,
        layout: &ViewportLayout,
        document: &Document,
        cache: &mut DocumentCache,
        rows: &mut RowNeighborhood,
    ) -> Result<Option<LocatedViewport>, TutError> {
        let mut reader = document.reader(cache);
        locator.advance(layout, &mut reader, rows)
    }

    #[test]
    fn row_neighborhood_moves_in_both_directions_and_clamps_at_end() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        for start in 0..5 {
            rows.observe(key, offset(start), Some(offset(start + 1)))
                .unwrap();
        }
        rows.observe(key, offset(5), None).unwrap();
        let height = BodyHeight::new(3).unwrap();

        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(6),
                offset(0),
                RowDelta::Forward(2),
                height,
            ),
            Some(LocatedViewport {
                anchor: offset(2),
                at_end: false,
            })
        );
        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(6),
                offset(4),
                RowDelta::Backward(2),
                height,
            ),
            Some(LocatedViewport {
                anchor: offset(2),
                at_end: false,
            })
        );
        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(6),
                offset(0),
                RowDelta::Forward(20),
                height,
            ),
            Some(LocatedViewport {
                anchor: offset(3),
                at_end: true,
            })
        );
    }

    #[test]
    fn row_neighborhood_locates_half_open_targets_and_bounded_eof() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        rows.observe(key, offset(3), Some(offset(7))).unwrap();
        rows.observe(key, offset(7), Some(offset(12))).unwrap();
        rows.observe(key, offset(12), None).unwrap();
        let height = BodyHeight::new(1).unwrap();

        for (target, anchor, at_end) in [
            (3, 3, false),
            (6, 3, false),
            (7, 7, false),
            (11, 7, false),
            (12, 12, true),
            (15, 12, true),
        ] {
            assert_eq!(
                rows.locate_target(
                    key,
                    offset(3),
                    offset(15),
                    offset(target),
                    RowDelta::Forward(0),
                    height,
                ),
                Some(LocatedViewport {
                    anchor: offset(anchor),
                    at_end,
                }),
                "target={target}"
            );
        }
        for target in [2, 16] {
            assert_eq!(
                rows.locate_target(
                    key,
                    offset(3),
                    offset(15),
                    offset(target),
                    RowDelta::Forward(0),
                    height,
                ),
                None,
                "target={target}"
            );
        }
    }

    #[test]
    fn row_neighborhood_rejects_unknown_boundaries_and_moves_from_containing_rows() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        rows.observe(key, offset(0), Some(offset(4))).unwrap();
        rows.observe(key, offset(4), Some(offset(8))).unwrap();
        rows.observe(key, offset(8), Some(offset(12))).unwrap();

        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(15),
                offset(11),
                RowDelta::Forward(0),
                BodyHeight::new(1).unwrap(),
            ),
            Some(LocatedViewport {
                anchor: offset(8),
                at_end: false,
            })
        );
        for target in [12, 13] {
            assert_eq!(
                rows.locate_target(
                    key,
                    offset(0),
                    offset(15),
                    offset(target),
                    RowDelta::Forward(0),
                    BodyHeight::new(1).unwrap(),
                ),
                None,
                "target={target}"
            );
        }

        rows.observe(key, offset(12), None).unwrap();
        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(15),
                offset(6),
                RowDelta::Backward(1),
                BodyHeight::new(2).unwrap(),
            ),
            Some(LocatedViewport {
                anchor: offset(0),
                at_end: false,
            })
        );
        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(15),
                offset(6),
                RowDelta::Forward(1),
                BodyHeight::new(2).unwrap(),
            ),
            Some(LocatedViewport {
                anchor: offset(8),
                at_end: true,
            })
        );
        assert_eq!(
            rows.locate_target(
                key,
                offset(0),
                offset(15),
                offset(15),
                RowDelta::Forward(0),
                BodyHeight::new(3).unwrap(),
            ),
            Some(LocatedViewport {
                anchor: offset(4),
                at_end: true,
            })
        );

        let mut empty = RowNeighborhood::default();
        empty.observe(key, offset(3), None).unwrap();
        assert_eq!(
            empty.locate_target(
                key,
                offset(3),
                offset(3),
                offset(3),
                RowDelta::Forward(0),
                BodyHeight::new(5).unwrap(),
            ),
            Some(LocatedViewport {
                anchor: offset(3),
                at_end: true,
            })
        );
    }

    #[test]
    fn row_neighborhood_starts_with_a_small_allocation() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);

        rows.observe(key, offset(0), Some(offset(1))).unwrap();

        assert_eq!(rows.edges.len(), 1);
        assert!(rows.edges.capacity() >= INITIAL_ROW_NEIGHBORHOOD_CAPACITY);
        assert!(rows.edges.capacity() < ROW_NEIGHBORHOOD_CAPACITY);
    }

    #[test]
    fn row_neighborhood_growth_preserves_continuity_and_locations() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        rows.observe(key, offset(0), Some(offset(1))).unwrap();
        let initial_capacity = rows.edges.capacity();

        for start in 1..=initial_capacity {
            rows.observe(
                key,
                SourceOffset::from_usize(start),
                Some(SourceOffset::from_usize(start + 1)),
            )
            .unwrap();
        }

        assert_eq!(rows.edges.len(), initial_capacity + 1);
        assert!(rows.edges.capacity() > initial_capacity);
        assert!(rows.edges.iter().enumerate().all(|(start, edge)| edge.start
            == SourceOffset::from_usize(start)
            && edge.next == Some(SourceOffset::from_usize(start + 1))));
        let target = initial_capacity / 2;
        assert_eq!(
            rows.locate_target(
                key,
                SourceOffset::ZERO,
                SourceOffset::from_usize(initial_capacity + 1),
                SourceOffset::from_usize(target),
                RowDelta::Forward(1),
                BodyHeight::new(1).unwrap(),
            ),
            Some(LocatedViewport {
                anchor: SourceOffset::from_usize(target + 1),
                at_end: false,
            })
        );
    }

    #[test]
    fn row_neighborhood_has_a_fixed_logical_bound() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        for start in 0..ROW_NEIGHBORHOOD_CAPACITY {
            rows.observe(
                key,
                SourceOffset::from_usize(start),
                Some(SourceOffset::from_usize(start + 1)),
            )
            .unwrap();
            assert!(rows.edges.len() <= ROW_NEIGHBORHOOD_CAPACITY);
        }
        rows.observe(
            key,
            SourceOffset::from_usize(ROW_NEIGHBORHOOD_CAPACITY),
            None,
        )
        .unwrap();

        assert_eq!(rows.edges.len(), ROW_NEIGHBORHOOD_CAPACITY);
        assert_eq!(rows.edges.front().unwrap().start, SourceOffset::new(1));
        assert_eq!(
            rows.edges.back().unwrap().start,
            SourceOffset::from_usize(ROW_NEIGHBORHOOD_CAPACITY)
        );

        rows.observe(key, SourceOffset::ZERO, Some(SourceOffset::new(1)))
            .unwrap();
        assert_eq!(rows.edges.len(), ROW_NEIGHBORHOOD_CAPACITY);
        assert_eq!(rows.edges.front().unwrap().start, SourceOffset::ZERO);
        assert_eq!(
            rows.edges.back().unwrap().start,
            SourceOffset::from_usize(ROW_NEIGHBORHOOD_CAPACITY - 1)
        );
    }

    #[test]
    fn row_neighborhood_rejects_another_layout_key() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        rows.observe(key, offset(0), Some(offset(1))).unwrap();
        rows.observe(key, offset(1), None).unwrap();

        let other_width = (key.0, ContentWidth::new(17).unwrap());
        assert_eq!(
            rows.locate_target(
                other_width,
                offset(0),
                offset(1),
                offset(0),
                RowDelta::Forward(1),
                BodyHeight::new(1).unwrap(),
            ),
            None
        );

        let other_document = row_key(16);
        assert_eq!(
            rows.locate_target(
                other_document,
                offset(0),
                offset(1),
                offset(0),
                RowDelta::Forward(1),
                BodyHeight::new(1).unwrap(),
            ),
            None
        );
    }

    #[test]
    fn source_row_bounds_cover_empty_shifted_and_maximum_ranges() {
        assert_eq!(source_row_bound(offset(3), offset(3)).get(), 1);
        assert_eq!(source_row_bound(offset(3), offset(4)).get(), 2);
        assert_eq!(
            source_row_bound(SourceOffset::ZERO, SourceOffset::new(u64::MAX)).get(),
            usize::MAX
        );
    }

    #[test]
    fn viewport_locator_reservations_obey_source_bounds_and_match_the_layout_oracle() {
        for text in ["", "x", "a\n", "a\r\nβ\r終", "abcdefghi\n\nlast"] {
            let document = Document::from_text(Path::new("bounded-locator.txt"), text.to_owned());
            let bound = row_bound(&document);
            let targets: Vec<_> = text
                .char_indices()
                .map(|(index, _)| SourceOffset::from_usize(index))
                .chain(std::iter::once(SourceOffset::from_usize(text.len())))
                .collect();

            for height_value in [1, u16::MAX] {
                let height = BodyHeight::new(height_value).unwrap();
                let mut cache = DocumentCache::default();
                let layout = viewport_layout(&document, &mut cache, 4, height_value);

                for target in targets.iter().copied() {
                    for (downward, delta) in [
                        (false, RowDelta::Backward(0)),
                        (false, RowDelta::Backward(7)),
                        (true, RowDelta::Forward(0)),
                        (true, RowDelta::Forward(7)),
                    ] {
                        let amount = match delta {
                            RowDelta::Backward(amount) | RowDelta::Forward(amount) => amount,
                        };
                        let expected = {
                            let mut reader = document.reader(&mut cache);
                            let anchor = layout
                                .move_row_start(&mut reader, target, downward, amount)
                                .unwrap();
                            LocatedViewport {
                                anchor,
                                at_end: layout.is_last_viewport(&mut reader, anchor).unwrap(),
                            }
                        };
                        let mut locator =
                            ViewportLocator::new(target, delta, height, bound).unwrap();
                        let backward_rows = match delta {
                            RowDelta::Backward(amount) => amount.saturating_add(1),
                            RowDelta::Forward(_) => 1,
                        };
                        let expected_capacity = usize::from(height_value)
                            .max(backward_rows)
                            .min(bound.get());
                        assert_eq!(locator.capacity, expected_capacity);
                        assert!(locator.history.capacity() >= expected_capacity);
                        assert!(locator.prefix_rows.capacity() >= expected_capacity);

                        let mut rows = RowNeighborhood::default();
                        let mut actual = None;
                        for _ in 0..100 {
                            actual =
                                advance(&mut locator, &layout, &document, &mut cache, &mut rows)
                                    .unwrap();
                            if actual.is_some() {
                                break;
                            }
                        }
                        assert_eq!(
                            actual,
                            Some(expected),
                            "text={text:?}, target={target:?}, delta={delta:?}, height={height_value}"
                        );
                    }
                }
            }
        }

        let document = Document::from_text(Path::new("bounded-locator.txt"), "a\nb".to_owned());
        let bound = row_bound(&document);
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, u16::MAX, u16::MAX);
        let mut locator = ViewportLocator::from_row_start(
            SourceOffset::new(2),
            RowDelta::Backward(usize::MAX),
            BodyHeight::new(u16::MAX).unwrap(),
            bound,
        )
        .unwrap();
        assert_eq!(locator.capacity, 4);
        let mut rows = RowNeighborhood::default();
        assert_eq!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
            Some(LocatedViewport {
                anchor: SourceOffset::ZERO,
                at_end: true,
            })
        );
    }

    #[test]
    fn viewport_locator_preempts_long_rows_without_committing_partial_edges() {
        let document = Document::from_text(Path::new("long-row.txt"), "x".repeat(2_048));
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, u16::MAX, 1);
        let mut locator = ViewportLocator::from_row_start(
            SourceOffset::ZERO,
            RowDelta::Forward(0),
            BodyHeight::new(1).unwrap(),
            row_bound(&document),
        )
        .unwrap();
        let mut rows = RowNeighborhood::default();

        for _ in 0..2 {
            cache.reset_metrics();
            assert_eq!(
                advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
                None
            );
            assert_eq!(cache.metrics().grapheme_emissions(), 1_024);
            assert!(locator.row_scan.is_some());
            assert_eq!(locator.history, VecDeque::from([SourceOffset::ZERO]));
            assert!(rows.edges.is_empty());
            assert_eq!(
                locator.phase,
                Phase::Clamp {
                    candidate: SourceOffset::ZERO,
                    cursor: SourceOffset::ZERO,
                    visible: 0,
                }
            );
        }

        cache.reset_metrics();
        assert_eq!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
            Some(LocatedViewport {
                anchor: SourceOffset::ZERO,
                at_end: true,
            })
        );
        assert_eq!(cache.metrics().grapheme_emissions(), 0);
        assert!(locator.row_scan.is_none());
        assert_eq!(
            rows.edges.back().copied(),
            Some(RowEdge {
                start: SourceOffset::ZERO,
                next: None,
            })
        );
    }

    #[test]
    fn viewport_locator_preempts_after_one_oversized_byte_budget_emission() {
        let mut cluster = String::from("a");
        cluster.extend(std::iter::repeat_n(
            '\u{301}',
            SOURCE_WINDOW_BYTES / '\u{301}'.len_utf8() + 1,
        ));
        assert!(cluster.len() > SOURCE_WINDOW_BYTES);
        let document = Document::from_text(Path::new("wide-clusters.txt"), cluster.repeat(3));
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, 4, 1);
        let mut locator = ViewportLocator::from_row_start(
            SourceOffset::ZERO,
            RowDelta::Forward(0),
            BodyHeight::new(1).unwrap(),
            row_bound(&document),
        )
        .unwrap();
        let mut rows = RowNeighborhood::default();

        for _ in 0..3 {
            cache.reset_metrics();
            assert_eq!(
                advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
                None
            );
            assert_eq!(cache.metrics().grapheme_emissions(), 1);
            assert!(locator.row_scan.is_some());
            assert!(rows.edges.is_empty());
        }

        assert_eq!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
            Some(LocatedViewport {
                anchor: SourceOffset::ZERO,
                at_end: true,
            })
        );
    }

    #[test]
    fn viewport_locator_shares_one_atom_budget_across_short_rows() {
        let document = Document::from_text(Path::new("short-rows.txt"), "x\n".repeat(800));
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, 80, 1);
        let mut locator = ViewportLocator::from_row_start(
            SourceOffset::ZERO,
            RowDelta::Forward(700),
            BodyHeight::new(1).unwrap(),
            row_bound(&document),
        )
        .unwrap();
        let mut rows = RowNeighborhood::default();

        cache.reset_metrics();
        assert_eq!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
            None
        );
        assert_eq!(cache.metrics().grapheme_emissions(), 1_024);
        assert!(locator.row_scan.is_none());
        assert_eq!(rows.edges.len(), 512);
        assert!(matches!(
            locator.phase,
            Phase::MoveForward {
                cursor,
                remaining: 188,
            } if cursor == SourceOffset::from_usize(1_024)
        ));

        for _ in 0..4 {
            if let Some(located) =
                advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap()
            {
                assert_eq!(
                    located,
                    LocatedViewport {
                        anchor: SourceOffset::from_usize(1_400),
                        at_end: false,
                    }
                );
                return;
            }
        }
        panic!("short-row locator did not complete within its bounded turns");
    }

    #[test]
    fn locate_and_prepend_commit_state_only_after_complete_rows() {
        let long = "x".repeat(2_048);
        let document = Document::from_text(Path::new("phase-atomicity.txt"), long.clone());
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, u16::MAX, 1);
        let mut locator = ViewportLocator::new(
            SourceOffset::from_usize(1_500),
            RowDelta::Forward(0),
            BodyHeight::new(1).unwrap(),
            row_bound(&document),
        )
        .unwrap();
        let mut rows = RowNeighborhood::default();

        for _ in 0..2 {
            assert_eq!(
                advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
                None
            );
            assert!(locator.history.is_empty());
            assert!(rows.edges.is_empty());
            assert_eq!(
                locator.phase,
                Phase::Locate {
                    cursor: SourceOffset::ZERO,
                }
            );
        }

        let text = format!("{long}\nlast");
        let document = Document::from_text(Path::new("prepend-atomicity.txt"), text);
        let last_start = SourceOffset::from_usize(2_049);
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, u16::MAX, 2);
        let mut locator = ViewportLocator::from_row_start(
            last_start,
            RowDelta::Forward(0),
            BodyHeight::new(2).unwrap(),
            row_bound(&document),
        )
        .unwrap();
        locator.phase = Phase::Prepend {
            required: 2,
            resume: PrefixResume::FinishAtEnd,
        };
        let mut rows = RowNeighborhood::default();

        for _ in 0..2 {
            assert_eq!(
                advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
                None
            );
            assert_eq!(locator.history, VecDeque::from([last_start]));
            assert!(locator.prefix.is_some());
            assert!(locator.prefix_rows.is_empty());
            assert!(rows.edges.is_empty());
        }
        assert_eq!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
            Some(LocatedViewport {
                anchor: SourceOffset::ZERO,
                at_end: true,
            })
        );
        assert_eq!(
            locator.history,
            VecDeque::from([SourceOffset::ZERO, last_start])
        );
        assert!(locator.prefix.is_none());
    }

    #[test]
    fn pending_row_scans_reject_file_changes_before_committing_state() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changing-row.txt");
        fs::write(&path, "x".repeat(2_048)).unwrap();
        let document = load(path.clone()).unwrap();
        let mut cache = DocumentCache::default();
        let layout = viewport_layout(&document, &mut cache, u16::MAX, 1);
        let mut locator = ViewportLocator::from_row_start(
            SourceOffset::ZERO,
            RowDelta::Forward(0),
            BodyHeight::new(1).unwrap(),
            row_bound(&document),
        )
        .unwrap();
        let mut rows = RowNeighborhood::default();

        assert_eq!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows).unwrap(),
            None
        );
        assert!(locator.row_scan.is_some());
        let history = locator.history.clone();
        fs::write(path, "y".repeat(2_048)).unwrap();

        assert!(matches!(
            advance(&mut locator, &layout, &document, &mut cache, &mut rows),
            Err(TutError::Load(_))
        ));
        assert!(locator.row_scan.is_none());
        assert_eq!(locator.history, history);
        assert!(rows.edges.is_empty());
    }
}
