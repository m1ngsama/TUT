use std::ops::Range;

use crate::{error::SearchError, layout::NormalizedOffset};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SearchRange {
    start: NormalizedOffset,
    end: NormalizedOffset,
}

impl SearchRange {
    pub(super) const fn new(start: NormalizedOffset, end: NormalizedOffset) -> Option<Self> {
        if start.get() < end.get() {
            Some(Self { start, end })
        } else {
            None
        }
    }

    pub(super) const fn start(self) -> NormalizedOffset {
        self.start
    }

    pub(super) const fn end(self) -> NormalizedOffset {
        self.end
    }
}

#[derive(Debug)]
pub(super) struct MatchIndex {
    bits: Vec<u8>,
    source_len: NormalizedOffset,
    query_len: u32,
}

impl MatchIndex {
    pub(super) fn build(haystack: &str, needle: &str) -> Result<Option<Self>, SearchError> {
        if needle.is_empty() {
            return Ok(None);
        }

        let source_len = NormalizedOffset::try_from_usize(haystack.len()).map_err(|_| {
            SearchError::TextTooLong {
                bytes: haystack.len(),
            }
        })?;
        let query_len = u32::try_from(needle.len()).map_err(|_| SearchError::TextTooLong {
            bytes: needle.len(),
        })?;
        let storage = required_storage_bytes(source_len);
        let mut bits = Vec::new();
        bits.try_reserve_exact(storage)
            .map_err(|_| SearchError::Allocation)?;
        bits.resize(storage, 0);

        let mut index = Self {
            bits,
            source_len,
            query_len,
        };
        let mut cursor = 0usize;
        while cursor < haystack.len() {
            let Some(relative) = haystack[cursor..].find(needle) else {
                break;
            };
            let start = cursor + relative;
            index.set_start(u32::try_from(start).expect("source length was validated"));
            cursor = start + needle.len();
        }

        Ok(Some(index))
    }

    pub(super) fn first_intersecting_or_wrap(
        &self,
        visible_start: NormalizedOffset,
    ) -> Option<SearchRange> {
        let earliest = visible_start
            .get()
            .saturating_sub(self.query_len.saturating_sub(1));
        self.next_start_at_or_after(earliest)
            .map(|start| self.range_at(start))
            .filter(|range| range.end() > visible_start)
            .or_else(|| {
                self.next_start_at_or_after(0)
                    .map(|start| self.range_at(start))
            })
    }

    pub(super) fn next_after(&self, current: SearchRange) -> Option<SearchRange> {
        self.next_start_at_or_after(current.end().get())
            .or_else(|| self.next_start_at_or_after(0))
            .map(|start| self.range_at(start))
    }

    pub(super) fn previous_before(&self, current: SearchRange) -> Option<SearchRange> {
        self.previous_start_before(current.start().get())
            .or_else(|| self.previous_start_before(self.source_len.get()))
            .map(|start| self.range_at(start))
    }

    pub(super) fn intersecting(&self, visible: Range<NormalizedOffset>) -> IntersectingMatches<'_> {
        let earliest = visible
            .start
            .get()
            .saturating_sub(self.query_len.saturating_sub(1));
        let before = visible.end.get();
        IntersectingMatches {
            index: self,
            visible,
            next: self.next_start_at_or_after_before(earliest, before),
        }
    }

    fn range_at(&self, start: u32) -> SearchRange {
        SearchRange::new(
            NormalizedOffset::new(start),
            NormalizedOffset::new(start + self.query_len),
        )
        .expect("indexed queries are nonempty")
    }

    fn set_start(&mut self, offset: u32) {
        self.bits[(offset / 8) as usize] |= 1_u8 << (offset % 8);
    }

    fn next_start_at_or_after_before(&self, from: u32, before: u32) -> Option<u32> {
        let before = before.min(self.source_len.get());
        if from >= before {
            return None;
        }

        let mut byte_index = (from / 8) as usize;
        let last_byte_index = ((before - 1) / 8) as usize;
        let mut byte = self.bits[byte_index] & (u8::MAX << (from % 8));

        loop {
            if byte_index == last_byte_index {
                let last_bit = (before - 1) % 8;
                byte &= u8::MAX >> (7 - last_bit);
            }
            if byte != 0 {
                let byte_offset =
                    u32::try_from(byte_index).expect("search index is bounded by u32");
                return Some(byte_offset * 8 + byte.trailing_zeros());
            }
            if byte_index == last_byte_index {
                return None;
            }
            byte_index += 1;
            byte = self.bits[byte_index];
        }
    }

    fn next_start_at_or_after(&self, from: u32) -> Option<u32> {
        if from >= self.source_len.get() {
            return None;
        }
        let mut byte_index = (from / 8) as usize;
        let mut byte = self.bits[byte_index] & (u8::MAX << (from % 8));

        loop {
            if byte != 0 {
                let byte_offset =
                    u32::try_from(byte_index).expect("search index is bounded by u32");
                let offset = byte_offset * 8 + byte.trailing_zeros();
                return (offset < self.source_len.get()).then_some(offset);
            }
            byte_index += 1;
            byte = *self.bits.get(byte_index)?;
        }
    }

    fn previous_start_before(&self, before: u32) -> Option<u32> {
        if before == 0 || self.source_len.get() == 0 {
            return None;
        }

        let last = before.min(self.source_len.get()) - 1;
        let mut byte_index = (last / 8) as usize;
        let last_bit = last % 8;
        let mut byte = self.bits[byte_index] & (u8::MAX >> (7 - last_bit));

        loop {
            if byte != 0 {
                let highest = byte.ilog2();
                let byte_offset =
                    u32::try_from(byte_index).expect("search index is bounded by u32");
                return Some(byte_offset * 8 + highest);
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
    visible: Range<NormalizedOffset>,
    next: Option<u32>,
}

impl Iterator for IntersectingMatches<'_> {
    type Item = SearchRange;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let start = self.next?;
            if start >= self.visible.end.get() {
                self.next = None;
                return None;
            }

            let range = self.index.range_at(start);
            self.next = self.index.next_start_at_or_after_before(
                start + self.index.query_len,
                self.visible.end.get(),
            );
            if range.end() <= self.visible.start {
                continue;
            }
            return Some(range);
        }
    }
}

const fn required_storage_bytes(source_len: NormalizedOffset) -> usize {
    source_len.get().div_ceil(8) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_global_nonoverlapping_index() {
        let index = MatchIndex::build("aaaaaa", "aa").unwrap().unwrap();
        let matches: Vec<_> = index
            .intersecting(NormalizedOffset::ZERO..NormalizedOffset::new(6))
            .collect();
        assert_eq!(
            matches,
            vec![
                SearchRange::new(NormalizedOffset::new(0), NormalizedOffset::new(2)).unwrap(),
                SearchRange::new(NormalizedOffset::new(2), NormalizedOffset::new(4)).unwrap(),
                SearchRange::new(NormalizedOffset::new(4), NormalizedOffset::new(6)).unwrap(),
            ]
        );
    }

    #[test]
    fn navigation_wraps_in_both_directions() {
        let index = MatchIndex::build("cat cat", "cat").unwrap().unwrap();
        let first = index
            .first_intersecting_or_wrap(NormalizedOffset::ZERO)
            .unwrap();
        let second = index.next_after(first).unwrap();
        assert_eq!(second.start(), NormalizedOffset::new(4));
        assert_eq!(index.next_after(second), Some(first));
        assert_eq!(index.previous_before(first), Some(second));
    }

    #[test]
    fn empty_queries_have_no_index_and_multibyte_queries_use_byte_ranges() {
        assert!(MatchIndex::build("text", "").unwrap().is_none());
        let index = MatchIndex::build("é-é", "é").unwrap().unwrap();
        let first = index
            .first_intersecting_or_wrap(NormalizedOffset::ZERO)
            .unwrap();
        assert_eq!(first.end().get() - first.start().get(), 2);
        assert_eq!(
            index.next_after(first).unwrap().start(),
            NormalizedOffset::new(3)
        );
    }

    #[test]
    fn viewport_selection_includes_matches_crossing_its_start() {
        let index = MatchIndex::build("abcd--abcd", "abcd").unwrap().unwrap();
        assert_eq!(
            index
                .first_intersecting_or_wrap(NormalizedOffset::new(3))
                .unwrap()
                .start(),
            NormalizedOffset::ZERO
        );
        assert_eq!(
            index
                .first_intersecting_or_wrap(NormalizedOffset::new(4))
                .unwrap()
                .start(),
            NormalizedOffset::new(6)
        );
        let visible: Vec<_> = index
            .intersecting(NormalizedOffset::new(3)..NormalizedOffset::new(7))
            .collect();
        assert_eq!(visible.len(), 2);
    }
}
