use std::ops::Range;

use crate::{
    error::SearchError,
    source::{SourceOffset, SourceText},
};

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

#[derive(Debug)]
pub(super) struct MatchIndex {
    bits: Vec<u8>,
    source_start: SourceOffset,
    source_end: SourceOffset,
    source_len: usize,
    query_len: usize,
}

impl MatchIndex {
    pub(super) fn build(
        haystack: SourceText<'_>,
        needle: &str,
    ) -> Result<Option<Self>, SearchError> {
        if needle.is_empty() {
            return Ok(None);
        }

        let source_len = haystack.len_bytes();
        let query_len = needle.len();
        let storage = required_storage_bytes(source_len);
        let mut bits = Vec::new();
        bits.try_reserve_exact(storage)
            .map_err(|_| SearchError::Allocation)?;
        bits.resize(storage, 0);

        let mut index = Self {
            bits,
            source_start: haystack.start(),
            source_end: haystack.end(),
            source_len,
            query_len,
        };
        let mut cursor = 0usize;
        while cursor < haystack.len_bytes() {
            let Some(relative) = haystack.as_str()[cursor..].find(needle) else {
                break;
            };
            let start = cursor + relative;
            index.set_start(start);
            cursor = start + needle.len();
        }

        Ok(Some(index))
    }

    pub(super) fn first_intersecting_or_wrap(
        &self,
        visible_start: SourceOffset,
    ) -> Option<SearchRange> {
        let visible = self.relative_clamped(visible_start);
        let earliest = visible.saturating_sub(self.query_len.saturating_sub(1));
        self.next_start_at_or_after(earliest)
            .map(|start| self.range_at(start))
            .filter(|range| range.end() > visible_start)
            .or_else(|| {
                self.next_start_at_or_after(0)
                    .map(|start| self.range_at(start))
            })
    }

    pub(super) fn next_after(&self, current: SearchRange) -> Option<SearchRange> {
        self.next_start_at_or_after(self.relative_clamped(current.end()))
            .or_else(|| self.next_start_at_or_after(0))
            .map(|start| self.range_at(start))
    }

    pub(super) fn previous_before(&self, current: SearchRange) -> Option<SearchRange> {
        self.previous_start_before(self.relative_clamped(current.start()))
            .or_else(|| self.previous_start_before(self.source_len))
            .map(|start| self.range_at(start))
    }

    pub(super) fn intersecting(&self, visible: Range<SourceOffset>) -> IntersectingMatches<'_> {
        let earliest = self
            .relative_clamped(visible.start)
            .saturating_sub(self.query_len.saturating_sub(1));
        let before = self.relative_clamped(visible.end);
        IntersectingMatches {
            index: self,
            visible,
            next: self.next_start_at_or_after_before(earliest, before),
        }
    }

    fn range_at(&self, start: usize) -> SearchRange {
        let start = self
            .source_start
            .checked_add(start)
            .expect("indexed match starts inside the source span");
        let end = start
            .checked_add(self.query_len)
            .expect("indexed match ends inside the source span");
        SearchRange::new(start, end).expect("indexed queries are nonempty")
    }

    fn relative_clamped(&self, offset: SourceOffset) -> usize {
        if offset <= self.source_start {
            return 0;
        }
        if offset >= self.source_end {
            return self.source_len;
        }
        usize::try_from(offset.get() - self.source_start.get())
            .expect("in-memory source offsets fit usize")
    }

    fn set_start(&mut self, offset: usize) {
        self.bits[offset / 8] |= 1_u8 << (offset % 8);
    }

    fn next_start_at_or_after_before(&self, from: usize, before: usize) -> Option<usize> {
        let before = before.min(self.source_len);
        if from >= before {
            return None;
        }

        let mut byte_index = from / 8;
        let last_byte_index = (before - 1) / 8;
        let mut byte = self.bits[byte_index] & (u8::MAX << (from % 8));

        loop {
            if byte_index == last_byte_index {
                let last_bit = (before - 1) % 8;
                byte &= u8::MAX >> (7 - last_bit);
            }
            if byte != 0 {
                return Some(byte_index * 8 + byte.trailing_zeros() as usize);
            }
            if byte_index == last_byte_index {
                return None;
            }
            byte_index += 1;
            byte = self.bits[byte_index];
        }
    }

    fn next_start_at_or_after(&self, from: usize) -> Option<usize> {
        if from >= self.source_len {
            return None;
        }
        let mut byte_index = from / 8;
        let mut byte = self.bits[byte_index] & (u8::MAX << (from % 8));

        loop {
            if byte != 0 {
                let offset = byte_index * 8 + byte.trailing_zeros() as usize;
                return (offset < self.source_len).then_some(offset);
            }
            byte_index += 1;
            byte = *self.bits.get(byte_index)?;
        }
    }

    fn previous_start_before(&self, before: usize) -> Option<usize> {
        if before == 0 || self.source_len == 0 {
            return None;
        }

        let last = before.min(self.source_len) - 1;
        let mut byte_index = last / 8;
        let last_bit = last % 8;
        let mut byte = self.bits[byte_index] & (u8::MAX >> (7 - last_bit));

        loop {
            if byte != 0 {
                return Some(byte_index * 8 + byte.ilog2() as usize);
            }
            if byte_index == 0 {
                return None;
            }
            byte_index -= 1;
            byte = self.bits[byte_index];
        }
    }
}

pub(super) struct IntersectingMatches<'a> {
    index: &'a MatchIndex,
    visible: Range<SourceOffset>,
    next: Option<usize>,
}

impl Iterator for IntersectingMatches<'_> {
    type Item = SearchRange;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let start = self.next?;
            let range = self.index.range_at(start);
            if range.start() >= self.visible.end {
                self.next = None;
                return None;
            }

            self.next = self.index.next_start_at_or_after_before(
                start + self.index.query_len,
                self.index.relative_clamped(self.visible.end),
            );
            if range.end() <= self.visible.start {
                continue;
            }
            return Some(range);
        }
    }
}

const fn required_storage_bytes(source_len: usize) -> usize {
    source_len.div_ceil(8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_global_nonoverlapping_index() {
        let index = MatchIndex::build(SourceText::new("aaaaaa"), "aa")
            .unwrap()
            .unwrap();
        let matches: Vec<_> = index
            .intersecting(SourceOffset::ZERO..SourceOffset::new(6))
            .collect();
        assert_eq!(
            matches,
            vec![
                SearchRange::new(SourceOffset::new(0), SourceOffset::new(2)).unwrap(),
                SearchRange::new(SourceOffset::new(2), SourceOffset::new(4)).unwrap(),
                SearchRange::new(SourceOffset::new(4), SourceOffset::new(6)).unwrap(),
            ]
        );
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let index = MatchIndex::build(SourceText::new("cat cat"), "cat")
            .unwrap()
            .unwrap();
        let first = index
            .first_intersecting_or_wrap(SourceOffset::ZERO)
            .unwrap();
        let second = index.next_after(first).unwrap();
        assert_eq!(second.start(), SourceOffset::new(4));
        assert_eq!(index.next_after(second), Some(first));
        assert_eq!(index.previous_before(first), Some(second));
    }

    #[test]
    fn empty_queries_have_no_index_and_multibyte_queries_use_byte_ranges() {
        assert!(
            MatchIndex::build(SourceText::new("text"), "")
                .unwrap()
                .is_none()
        );
        let index = MatchIndex::build(SourceText::new("é-é"), "é")
            .unwrap()
            .unwrap();
        let first = index
            .first_intersecting_or_wrap(SourceOffset::ZERO)
            .unwrap();
        assert_eq!(first.end().get() - first.start().get(), 2);
        assert_eq!(
            index.next_after(first).unwrap().start(),
            SourceOffset::new(3)
        );
    }

    #[test]
    fn viewport_selection_includes_matches_crossing_its_start() {
        let index = MatchIndex::build(SourceText::new("abcd--abcd"), "abcd")
            .unwrap()
            .unwrap();
        assert_eq!(
            index
                .first_intersecting_or_wrap(SourceOffset::new(3))
                .unwrap()
                .start(),
            SourceOffset::ZERO
        );
        assert_eq!(
            index
                .first_intersecting_or_wrap(SourceOffset::new(4))
                .unwrap()
                .start(),
            SourceOffset::new(6)
        );
        let visible: Vec<_> = index
            .intersecting(SourceOffset::new(3)..SourceOffset::new(7))
            .collect();
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn matches_keep_absolute_offsets_above_u32() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 23);
        let source = SourceText::with_start("cat cat", start).unwrap();
        let index = MatchIndex::build(source, "cat").unwrap().unwrap();
        let first = index.first_intersecting_or_wrap(start).unwrap();
        let second = index.next_after(first).unwrap();

        assert_eq!(first.start(), start);
        assert_eq!(first.end(), start.checked_add(3).unwrap());
        assert_eq!(second.start(), start.checked_add(4).unwrap());
        assert_eq!(index.previous_before(first), Some(second));
    }
}
