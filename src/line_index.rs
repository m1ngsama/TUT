use std::{error::Error, fmt, mem::size_of};

use crate::source::{SourceOffset, SourceText};

const LINE_INDEX_MEMORY_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const INITIAL_CHECKPOINT_INTERVAL_BYTES: u64 = 64 * 1024;
const INITIAL_CHECKPOINT_RESERVATION: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
struct PhysicalLine(u64);

impl PhysicalLine {
    const ZERO: Self = Self(0);

    const fn get(self) -> u64 {
        self.0
    }

    fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineCheckpoint {
    scan_at: SourceOffset,
    line: PhysicalLine,
    line_start: SourceOffset,
    pending_cr: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineIndexLimits {
    initial_interval_bytes: u64,
    max_checkpoints: usize,
}

impl LineIndexLimits {
    const DEFAULT: Self = Self {
        initial_interval_bytes: INITIAL_CHECKPOINT_INTERVAL_BYTES,
        max_checkpoints: LINE_INDEX_MEMORY_BUDGET_BYTES / size_of::<LineCheckpoint>(),
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
pub(super) enum LineIndexError {
    InvalidSourceRange,
    NonContiguous {
        expected: SourceOffset,
        actual: SourceOffset,
    },
    WindowBeyondSource,
    EmptyWindow,
    AlreadyFinished,
    Incomplete {
        expected: SourceOffset,
        actual: SourceOffset,
    },
    CoordinateOverflow,
    Allocation,
}

impl fmt::Display for LineIndexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceRange => formatter.write_str("invalid source range"),
            Self::NonContiguous { expected, actual } => write!(
                formatter,
                "non-contiguous source window: expected {}, got {}",
                expected.get(),
                actual.get()
            ),
            Self::WindowBeyondSource => formatter.write_str("source window exceeds document"),
            Self::EmptyWindow => formatter.write_str("empty source window before document end"),
            Self::AlreadyFinished => formatter.write_str("line index is already complete"),
            Self::Incomplete { expected, actual } => write!(
                formatter,
                "incomplete line index: expected {}, reached {}",
                expected.get(),
                actual.get()
            ),
            Self::CoordinateOverflow => formatter.write_str("line coordinate overflow"),
            Self::Allocation => formatter.write_str("could not allocate line checkpoints"),
        }
    }
}

impl Error for LineIndexError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LinePosition {
    current: u64,
    total: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LineScan {
    scan_at: SourceOffset,
    base_line: u64,
    line_start: SourceOffset,
    pending_cr: bool,
    total_lines: Option<u64>,
}

impl LineScan {
    pub(super) const fn start(self) -> SourceOffset {
        self.scan_at
    }

    pub(super) const fn line_start(self) -> SourceOffset {
        self.line_start
    }

    pub(super) const fn pending_cr(self) -> bool {
        self.pending_cr
    }

    pub(super) fn finish(self, additional_lines: u64) -> Option<LinePosition> {
        let current = self
            .base_line
            .checked_add(additional_lines)?
            .checked_add(1)?;
        Some(LinePosition {
            current,
            total: self.total_lines,
        })
    }
}

impl LinePosition {
    pub(super) const fn current(self) -> u64 {
        self.current
    }

    pub(super) const fn total(self) -> Option<u64> {
        self.total
    }
}

#[derive(Debug)]
pub(super) struct LineIndex {
    source_start: SourceOffset,
    source_end: SourceOffset,
    scanned_to: SourceOffset,
    last_line: PhysicalLine,
    last_line_start: SourceOffset,
    checkpoints: Vec<LineCheckpoint>,
    checkpoint_interval_bytes: u64,
    max_checkpoints: usize,
    pending_cr: bool,
    finished: bool,
}

impl LineIndex {
    pub(super) fn new(
        source_start: SourceOffset,
        source_end: SourceOffset,
    ) -> Result<Self, LineIndexError> {
        Self::with_limits(source_start, source_end, LineIndexLimits::DEFAULT)
    }

    fn with_limits(
        source_start: SourceOffset,
        source_end: SourceOffset,
        limits: LineIndexLimits,
    ) -> Result<Self, LineIndexError> {
        if source_start > source_end {
            return Err(LineIndexError::InvalidSourceRange);
        }

        let mut checkpoints = Vec::new();
        checkpoints
            .try_reserve_exact(INITIAL_CHECKPOINT_RESERVATION.min(limits.max_checkpoints))
            .map_err(|_| LineIndexError::Allocation)?;
        checkpoints.push(LineCheckpoint {
            scan_at: source_start,
            line: PhysicalLine::ZERO,
            line_start: source_start,
            pending_cr: false,
        });

        Ok(Self {
            source_start,
            source_end,
            scanned_to: source_start,
            last_line: PhysicalLine::ZERO,
            last_line_start: source_start,
            checkpoints,
            checkpoint_interval_bytes: limits.initial_interval_bytes,
            max_checkpoints: limits.max_checkpoints,
            pending_cr: false,
            finished: false,
        })
    }

    pub(super) fn extend(&mut self, source: SourceText<'_>) -> Result<(), LineIndexError> {
        if self.finished {
            return Err(LineIndexError::AlreadyFinished);
        }
        if source.start() != self.scanned_to {
            return Err(LineIndexError::NonContiguous {
                expected: self.scanned_to,
                actual: source.start(),
            });
        }
        if source.end() > self.source_end {
            return Err(LineIndexError::WindowBeyondSource);
        }
        if source.start() == source.end() {
            return Err(LineIndexError::EmptyWindow);
        }

        let bytes = source.as_str().as_bytes();
        let mut index = 0;
        if self.pending_cr {
            self.pending_cr = false;
            if bytes[0] == b'\n' {
                self.record_line_start(
                    source
                        .start()
                        .checked_add(1)
                        .ok_or(LineIndexError::CoordinateOverflow)?,
                )?;
                index = 1;
            } else {
                self.record_line_start(source.start())?;
            }
        }

        while index < bytes.len() {
            match bytes[index] {
                b'\n' => {
                    self.record_relative_line_start(source.start(), index + 1)?;
                    index += 1;
                }
                b'\r' if index + 1 < bytes.len() => {
                    let line_end = index + usize::from(bytes[index + 1] == b'\n') + 1;
                    self.record_relative_line_start(source.start(), line_end)?;
                    index = line_end;
                }
                b'\r' => {
                    self.pending_cr = true;
                    index += 1;
                }
                _ => index += 1,
            }
        }

        self.scanned_to = source.end();
        self.record_checkpoint()?;
        Ok(())
    }

    pub(super) fn finish(&mut self) -> Result<(), LineIndexError> {
        if self.finished {
            return Err(LineIndexError::AlreadyFinished);
        }
        if self.scanned_to != self.source_end {
            return Err(LineIndexError::Incomplete {
                expected: self.source_end,
                actual: self.scanned_to,
            });
        }
        if self.pending_cr {
            self.record_line_start(self.source_end)?;
            self.pending_cr = false;
        }
        self.finished = true;
        Ok(())
    }

    pub(super) const fn is_complete(&self) -> bool {
        self.finished
    }

    pub(super) const fn scanned_to(&self) -> SourceOffset {
        self.scanned_to
    }

    pub(super) fn covers(&self, offset: SourceOffset) -> bool {
        self.scan_from(offset).is_some()
    }

    pub(super) fn scan_from(&self, offset: SourceOffset) -> Option<LineScan> {
        if offset < self.source_start
            || offset > self.source_end
            || (!self.finished
                && (offset > self.scanned_to || (offset == self.scanned_to && self.pending_cr)))
        {
            return None;
        }
        let checkpoint = self.checkpoint_at_or_before(offset);
        Some(LineScan {
            scan_at: checkpoint.scan_at,
            base_line: checkpoint.line.get(),
            line_start: checkpoint.line_start,
            pending_cr: checkpoint.pending_cr,
            total_lines: if self.finished {
                self.last_line.get().checked_add(1)
            } else {
                None
            },
        })
    }

    #[cfg(test)]
    pub(super) fn position(
        &self,
        source: SourceText<'_>,
        offset: SourceOffset,
    ) -> Option<LinePosition> {
        if !self.finished || source.start() != self.source_start || source.end() != self.source_end
        {
            return None;
        }

        let relative = source.relative_offset(offset)?;
        if !source.as_str().is_char_boundary(relative) {
            return None;
        }
        let checkpoint = self.checkpoint_at_or_before(offset);
        let scan_end = if relative == source.len_bytes() {
            offset
        } else {
            let character = source.as_str()[relative..].chars().next()?;
            offset.checked_add(character.len_utf8())?
        };
        let text = source.slice(checkpoint.scan_at..scan_end)?;
        let current = checkpoint.line.get().checked_add(count_line_starts(
            text,
            checkpoint.scan_at,
            offset,
            checkpoint.pending_cr,
        )?)?;
        let current = current.checked_add(1)?;
        let total = self.last_line.get().checked_add(1);
        Some(LinePosition { current, total })
    }

    fn record_relative_line_start(
        &mut self,
        base: SourceOffset,
        relative: usize,
    ) -> Result<(), LineIndexError> {
        self.record_line_start(
            base.checked_add(relative)
                .ok_or(LineIndexError::CoordinateOverflow)?,
        )
    }

    fn record_line_start(&mut self, start: SourceOffset) -> Result<(), LineIndexError> {
        let line = self
            .last_line
            .next()
            .ok_or(LineIndexError::CoordinateOverflow)?;
        self.last_line = line;
        self.last_line_start = start;
        Ok(())
    }

    fn record_checkpoint(&mut self) -> Result<(), LineIndexError> {
        let last = self
            .checkpoints
            .last()
            .expect("line indexes retain their source-start checkpoint");
        if self.scanned_to.get() - last.scan_at.get() < self.checkpoint_interval_bytes {
            return Ok(());
        }
        while self.checkpoints.len() >= self.max_checkpoints {
            self.compact()?;
            let last = self
                .checkpoints
                .last()
                .expect("line indexes retain their source-start checkpoint");
            if self.scanned_to.get() - last.scan_at.get() < self.checkpoint_interval_bytes {
                return Ok(());
            }
        }
        self.reserve_checkpoint()?;
        self.checkpoints.push(LineCheckpoint {
            scan_at: self.scanned_to,
            line: self.last_line,
            line_start: self.last_line_start,
            pending_cr: self.pending_cr,
        });
        Ok(())
    }

    fn compact(&mut self) -> Result<(), LineIndexError> {
        self.checkpoint_interval_bytes = self
            .checkpoint_interval_bytes
            .checked_mul(2)
            .ok_or(LineIndexError::CoordinateOverflow)?;
        let mut index = 0;
        self.checkpoints.retain(|_| {
            let keep = index % 2 == 0;
            index += 1;
            keep
        });
        Ok(())
    }

    fn reserve_checkpoint(&mut self) -> Result<(), LineIndexError> {
        if self.checkpoints.len() < self.checkpoints.capacity() {
            return Ok(());
        }
        let remaining = self.max_checkpoints - self.checkpoints.len();
        let additional = self.checkpoints.capacity().max(1).min(remaining);
        self.checkpoints
            .try_reserve_exact(additional)
            .map_err(|_| LineIndexError::Allocation)
    }

    fn checkpoint_at_or_before(&self, offset: SourceOffset) -> LineCheckpoint {
        let insertion = self
            .checkpoints
            .partition_point(|checkpoint| checkpoint.scan_at <= offset);
        self.checkpoints[insertion.saturating_sub(1)]
    }
}

#[cfg(test)]
fn count_line_starts(
    text: &str,
    base: SourceOffset,
    through: SourceOffset,
    pending_cr: bool,
) -> Option<u64> {
    let bytes = text.as_bytes();
    let mut count = 0_u64;
    let mut index = 0;
    if pending_cr {
        if bytes.first() == Some(&b'\n') {
            if base.checked_add(1)? <= through {
                count = count.checked_add(1)?;
            }
            index = 1;
        } else if base <= through {
            count = count.checked_add(1)?;
        }
    }
    while index < bytes.len() {
        let line_end = match bytes[index] {
            b'\n' => index + 1,
            b'\r' if index + 1 < bytes.len() && bytes[index + 1] == b'\n' => index + 2,
            b'\r' => index + 1,
            _ => {
                index += 1;
                continue;
            }
        };
        if base.checked_add(line_end)? > through {
            break;
        }
        count = count.checked_add(1)?;
        index = line_end;
    }
    Some(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_in_chunks(source: SourceText<'_>, chunk_bytes: usize) -> LineIndex {
        let mut index = LineIndex::new(source.start(), source.end()).unwrap();
        let mut relative = 0;
        while relative < source.len_bytes() {
            let end = (relative + chunk_bytes).min(source.len_bytes());
            let start_offset = source.start().checked_add(relative).unwrap();
            let end_offset = source.start().checked_add(end).unwrap();
            let text = source.slice(start_offset..end_offset).unwrap();
            index
                .extend(SourceText::with_start(text, start_offset).unwrap())
                .unwrap();
            relative = end;
        }
        index.finish().unwrap();
        index
    }

    #[test]
    fn line_endings_are_stable_across_every_byte_split() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 31);
        let source = SourceText::with_start("a\r\nb\rc\n\n", start).unwrap();
        let expected = [(0, 1), (3, 2), (5, 3), (7, 4), (8, 5)];

        for chunk_bytes in 1..=source.len_bytes() {
            let index = build_in_chunks(source, chunk_bytes);
            for (relative, line) in expected {
                let offset = start.checked_add(relative).unwrap();
                assert_eq!(index.position(source, offset).unwrap().current(), line);
                assert_eq!(index.position(source, offset).unwrap().total(), Some(5));
            }
        }
    }

    #[test]
    fn byte_checkpoints_preserve_crlf_state_at_every_split() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 31);
        let source = SourceText::with_start("a\r\nb\rc\n\n", start).unwrap();
        let expected = [(0, 1), (3, 2), (5, 3), (7, 4), (8, 5)];

        for chunk_bytes in 1..=source.len_bytes() {
            let limits = LineIndexLimits::new(1, 64).unwrap();
            let mut index = LineIndex::with_limits(source.start(), source.end(), limits).unwrap();
            let mut relative = 0;
            while relative < source.len_bytes() {
                let end = (relative + chunk_bytes).min(source.len_bytes());
                let chunk_start = source.start().checked_add(relative).unwrap();
                let chunk_end = source.start().checked_add(end).unwrap();
                index
                    .extend(
                        SourceText::with_start(
                            source.slice(chunk_start..chunk_end).unwrap(),
                            chunk_start,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                relative = end;
            }
            index.finish().unwrap();

            for (relative, line) in expected {
                let offset = start.checked_add(relative).unwrap();
                assert_eq!(index.position(source, offset).unwrap().current(), line);
            }
        }
    }

    #[test]
    fn checkpoint_memory_compacts_without_losing_positions() {
        let text = "\n".repeat(128);
        let source = SourceText::new(&text);
        let limits = LineIndexLimits::new(1, 4).unwrap();
        let mut index = LineIndex::with_limits(source.start(), source.end(), limits).unwrap();
        for relative in 0..source.len_bytes() {
            let start = source.start().checked_add(relative).unwrap();
            let end = start.checked_add(1).unwrap();
            index
                .extend(SourceText::with_start(source.slice(start..end).unwrap(), start).unwrap())
                .unwrap();
        }
        index.finish().unwrap();

        assert!(index.checkpoints.len() <= 4);
        assert!(index.checkpoint_interval_bytes > 1);
        assert_eq!(
            index.position(source, SourceOffset::new(97)).unwrap(),
            LinePosition {
                current: 98,
                total: Some(129),
            }
        );
    }

    #[test]
    fn builder_rejects_discontinuous_and_incomplete_input() {
        let mut index = LineIndex::new(SourceOffset::new(10), SourceOffset::new(20)).unwrap();
        assert!(matches!(
            index.extend(SourceText::with_start("x", SourceOffset::new(11)).unwrap()),
            Err(LineIndexError::NonContiguous { .. })
        ));
        assert!(matches!(
            index.finish(),
            Err(LineIndexError::Incomplete { .. })
        ));
    }

    #[test]
    fn empty_source_has_one_physical_line() {
        let source = SourceText::with_start("", SourceOffset::new(3)).unwrap();
        let mut index = LineIndex::new(source.start(), source.end()).unwrap();
        index.finish().unwrap();

        assert_eq!(
            index.position(source, source.start()).unwrap(),
            LinePosition {
                current: 1,
                total: Some(1),
            }
        );
    }

    #[test]
    fn partial_indexes_report_unknown_totals_and_reject_unscanned_offsets() {
        let source = SourceText::new("a\r\nb\nc");
        let mut index = LineIndex::new(source.start(), source.end()).unwrap();
        let first_end = SourceOffset::new(2);
        index
            .extend(
                SourceText::with_start(
                    source.slice(SourceOffset::ZERO..first_end).unwrap(),
                    SourceOffset::ZERO,
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(
            index.scan_from(SourceOffset::ZERO).unwrap().finish(0),
            Some(LinePosition {
                current: 1,
                total: None,
            })
        );
        assert!(index.scan_from(first_end).is_none());
        assert!(index.scan_from(SourceOffset::new(3)).is_none());

        let rest =
            SourceText::with_start(source.slice(first_end..source.end()).unwrap(), first_end)
                .unwrap();
        index.extend(rest).unwrap();
        index.finish().unwrap();
        assert_eq!(
            index.position(source, source.end()).unwrap(),
            LinePosition {
                current: 3,
                total: Some(3),
            }
        );
    }

    #[test]
    fn byte_checkpoints_bound_replay_inside_a_long_physical_line() {
        let text = "x".repeat(4096);
        let source = SourceText::new(&text);
        let limits = LineIndexLimits::new(64, 128).unwrap();
        let mut index = LineIndex::with_limits(source.start(), source.end(), limits).unwrap();
        let mut start = 0;
        while start < source.len_bytes() {
            let end = (start + 17).min(source.len_bytes());
            let start_offset = SourceOffset::new(start as u64);
            let end_offset = SourceOffset::new(end as u64);
            index
                .extend(
                    SourceText::with_start(
                        source.slice(start_offset..end_offset).unwrap(),
                        start_offset,
                    )
                    .unwrap(),
                )
                .unwrap();
            start = end;
        }
        index.finish().unwrap();

        let target = SourceOffset::new(4000);
        let scan = index.scan_from(target).unwrap();
        assert!(target.get() - scan.start().get() < 81);
        assert_eq!(scan.line_start(), SourceOffset::ZERO);
        assert_eq!(
            index.position(source, target).unwrap(),
            LinePosition {
                current: 1,
                total: Some(1),
            }
        );
    }

    #[test]
    fn compacted_checkpoints_preserve_pending_cr_state() {
        let mut text = "x\r\n".repeat(128);
        text.push_str("tail");
        let source = SourceText::new(&text);
        let reference = build_in_chunks(source, source.len_bytes());
        let limits = LineIndexLimits::new(1, 4).unwrap();
        let mut compacted = LineIndex::with_limits(source.start(), source.end(), limits).unwrap();

        for relative in 0..source.len_bytes() {
            let start = SourceOffset::new(relative as u64);
            let end = start.checked_add(1).unwrap();
            compacted
                .extend(SourceText::with_start(source.slice(start..end).unwrap(), start).unwrap())
                .unwrap();
        }
        compacted.finish().unwrap();

        assert!(compacted.checkpoint_interval_bytes > 1);
        for relative in source
            .as_str()
            .char_indices()
            .map(|(relative, _)| relative)
            .chain(std::iter::once(source.len_bytes()))
        {
            let offset = SourceOffset::new(relative as u64);
            assert_eq!(
                compacted.position(source, offset),
                reference.position(source, offset)
            );
        }
    }
}
