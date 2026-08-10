use std::collections::VecDeque;

use crate::{
    document::{DocumentReader, SOURCE_WINDOW_BYTES},
    error::TutError,
    layout::{BodyHeight, ViewportLayout},
    source::SourceOffset,
};

const ROW_BUDGET: usize = 1024;
const BYTE_BUDGET: u64 = SOURCE_WINDOW_BYTES as u64;

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

    pub(super) fn advance(
        &mut self,
        layout: &ViewportLayout,
        reader: &mut DocumentReader<'_>,
    ) -> Result<Option<LocatedViewport>, TutError> {
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
