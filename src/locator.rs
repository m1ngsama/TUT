use std::collections::VecDeque;

use crate::{
    document::{DocumentId, DocumentReader, SOURCE_WINDOW_BYTES},
    error::TutError,
    layout::{BodyHeight, ContentWidth, ViewportLayout},
    source::SourceOffset,
};

const ROW_BUDGET: usize = 1024;
const BYTE_BUDGET: u64 = SOURCE_WINDOW_BYTES as u64;
const ROW_NEIGHBORHOOD_CAPACITY: usize = 4096;

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
        if self.edges.capacity() < ROW_NEIGHBORHOOD_CAPACITY {
            self.edges
                .try_reserve_exact(ROW_NEIGHBORHOOD_CAPACITY - self.edges.len())
                .map_err(|_| TutError::Allocation("visual row neighborhood"))?;
        }
        Ok(())
    }

    fn push_back(&mut self, edge: RowEdge) -> Result<(), TutError> {
        self.reserve()?;
        if self.edges.len() == ROW_NEIGHBORHOOD_CAPACITY {
            self.edges.pop_front();
        }
        self.edges.push_back(edge);
        Ok(())
    }

    fn push_front(&mut self, edge: RowEdge) -> Result<(), TutError> {
        self.reserve()?;
        if self.edges.len() == ROW_NEIGHBORHOOD_CAPACITY {
            self.edges.pop_back();
        }
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

struct Budget {
    rows: usize,
    bytes: u64,
}

impl Budget {
    const fn new() -> Self {
        Self { rows: 0, bytes: 0 }
    }

    fn charge(&mut self, start: SourceOffset, end: SourceOffset) {
        self.rows += 1;
        self.bytes = self
            .bytes
            .saturating_add(end.get().saturating_sub(start.get()));
    }

    const fn exhausted(&self) -> bool {
        self.rows >= ROW_BUDGET || self.bytes >= BYTE_BUDGET
    }
}

pub(super) struct ViewportLocator {
    target: SourceOffset,
    delta: RowDelta,
    height: usize,
    phase: Phase,
    history: VecDeque<SourceOffset>,
    prefix: Option<PrefixScan>,
    prefix_rows: VecDeque<SourceOffset>,
    capacity: usize,
}

impl ViewportLocator {
    pub(super) fn new(
        target: SourceOffset,
        delta: RowDelta,
        height: BodyHeight,
    ) -> Result<Self, TutError> {
        let height = usize::from(height.get());
        let backward_rows = match delta {
            RowDelta::Backward(amount) => amount
                .checked_add(1)
                .ok_or(TutError::Allocation("viewport locator row starts"))?,
            RowDelta::Forward(_) => 1,
        };
        let capacity = height.max(backward_rows);
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
            capacity,
        })
    }

    pub(super) fn from_row_start(
        target: SourceOffset,
        delta: RowDelta,
        height: BodyHeight,
    ) -> Result<Self, TutError> {
        let mut locator = Self::new(target, delta, height)?;
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
        let mut budget = Budget::new();
        loop {
            if budget.exhausted() {
                return Ok(None);
            }
            match self.phase {
                Phase::Start => {
                    let cursor = reader.line_start_at_or_before(self.target)?;
                    self.phase = Phase::Locate { cursor };
                }
                Phase::Locate { cursor } => {
                    self.push_history(cursor);
                    let next = layout.next_row_start(reader, cursor)?;
                    neighborhood.observe(row_key, cursor, next)?;
                    budget.charge(cursor, next.unwrap_or_else(|| reader.source_end()));
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
                            required: amount + 1,
                            resume: PrefixResume::SelectBackward { amount },
                        };
                    }
                }
                Phase::MoveForward { cursor, remaining } => {
                    if remaining == 0 {
                        self.begin_clamp(cursor);
                        continue;
                    }
                    let next = layout.next_row_start(reader, cursor)?;
                    neighborhood.observe(row_key, cursor, next)?;
                    budget.charge(cursor, next.unwrap_or_else(|| reader.source_end()));
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
                    let next = layout.next_row_start(reader, cursor)?;
                    neighborhood.observe(row_key, cursor, next)?;
                    budget.charge(cursor, next.unwrap_or_else(|| reader.source_end()));
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
                    if self.prefix_rows.len() == remaining {
                        self.prefix_rows.pop_front();
                    }
                    self.prefix_rows.push_back(prefix.cursor);
                    let next = layout.next_row_start(reader, prefix.cursor)?;
                    neighborhood.observe(row_key, prefix.cursor, next)?;
                    let scan_end = next
                        .unwrap_or_else(|| reader.source_end())
                        .min(prefix.line_end);
                    budget.charge(prefix.cursor, scan_end);
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
    use std::path::Path;

    use super::*;
    use crate::document::{Document, DocumentCache};

    fn offset(value: u64) -> SourceOffset {
        SourceOffset::new(value)
    }

    fn row_key(width: u16) -> (DocumentId, ContentWidth) {
        let document = Document::from_text(Path::new("rows.txt"), String::new());
        let mut cache = DocumentCache::default();
        let reader = document.reader(&mut cache);
        (reader.document_id(), ContentWidth::new(width).unwrap())
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
    fn row_neighborhood_has_a_fixed_memory_bound() {
        let mut rows = RowNeighborhood::default();
        let key = row_key(16);
        for start in 0..ROW_NEIGHBORHOOD_CAPACITY {
            rows.observe(
                key,
                SourceOffset::from_usize(start),
                Some(SourceOffset::from_usize(start + 1)),
            )
            .unwrap();
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
}
