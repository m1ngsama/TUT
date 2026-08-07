use std::{num::NonZeroUsize, ops::Range};

use crate::{
    document::{DocumentReader, SOURCE_WINDOW_BYTES},
    error::{SearchError, TutError},
    source::SourceOffset,
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
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<Option<Self>, TutError> {
        if needle.is_empty() {
            return Ok(None);
        }

        let source_start = reader.source_start();
        let source_end = reader.source_end();
        let source_len = usize::try_from(source_end.get() - source_start.get())
            .expect("document spans fit the process address space");
        let query_len = needle.len();
        let storage = required_storage_bytes(source_len);
        let mut bits = Vec::new();
        bits.try_reserve_exact(storage)
            .map_err(|_| SearchError::Allocation)?;
        bits.resize(storage, 0);

        let mut index = Self {
            bits,
            source_start,
            source_end,
            source_len,
            query_len,
        };
        let target_bytes = SOURCE_WINDOW_BYTES
            .checked_add(query_len.saturating_sub(1))
            .and_then(NonZeroUsize::new)
            .ok_or(SearchError::Allocation)?;
        let mut cursor = source_start;

        while cursor < source_end {
            let next = {
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
                let window_relative = usize::try_from(cursor.get() - source_start.get())
                    .expect("document spans fit the process address space");
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
                    index.set_start(window_relative + start);
                    last_match_end = start + query_len;
                    search_from = last_match_end;
                }

                cursor
                    .checked_add(safe_relative.max(last_match_end))
                    .expect("bounded search windows fit source coordinates")
            };
            debug_assert!(next > cursor);
            cursor = next;
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
            .expect("document source offsets fit usize")
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
    use std::path::Path;

    use super::*;
    use crate::document::{Document, DocumentCache};

    fn build_at(text: &str, start: SourceOffset, needle: &str) -> Option<MatchIndex> {
        let document = Document::from_text_at(Path::new("search.txt"), text.to_owned(), start);
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        MatchIndex::build(&mut reader, needle).unwrap()
    }

    fn build(text: &str, needle: &str) -> Option<MatchIndex> {
        build_at(text, SourceOffset::ZERO, needle)
    }

    #[test]
    fn builds_a_global_nonoverlapping_index() {
        let index = build("aaaaaa", "aa").unwrap();
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
        let index = build("cat cat", "cat").unwrap();
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
        assert!(build("text", "").is_none());
        let index = build("é-é", "é").unwrap();
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
        let index = build("abcd--abcd", "abcd").unwrap();
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
        let index = build_at("cat cat", start, "cat").unwrap();
        let first = index.first_intersecting_or_wrap(start).unwrap();
        let second = index.next_after(first).unwrap();

        assert_eq!(first.start(), start);
        assert_eq!(first.end(), start.checked_add(3).unwrap());
        assert_eq!(second.start(), start.checked_add(4).unwrap());
        assert_eq!(index.previous_before(first), Some(second));
    }

    #[test]
    fn matches_cross_source_window_boundaries() {
        let mut text = "x".repeat(SOURCE_WINDOW_BYTES - 2);
        text.push_str("needle tail needle");
        let index = build(&text, "needle").unwrap();
        let first = index
            .first_intersecting_or_wrap(SourceOffset::ZERO)
            .unwrap();
        let second = index.next_after(first).unwrap();

        assert_eq!(
            first.start(),
            SourceOffset::from_usize(SOURCE_WINDOW_BYTES - 2)
        );
        assert_eq!(
            second.start(),
            first.end().checked_add(" tail ".len()).unwrap()
        );
    }
}
