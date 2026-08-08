use std::{mem::size_of, num::NonZeroUsize, ops::Range};

use crate::{
    document::{DocumentReader, SOURCE_WINDOW_BYTES},
    error::{SearchError, TutError},
    source::SourceOffset,
};

const SEARCH_INDEX_MEMORY_BUDGET_BYTES: usize = 4 * 1024 * 1024;
const INITIAL_CHECKPOINT_INTERVAL_BYTES: u64 = SOURCE_WINDOW_BYTES as u64;
const INITIAL_CHECKPOINT_RESERVATION: usize = 1024;
pub(super) const MAX_SEARCH_QUERY_BYTES: usize = 4096;

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
pub(super) struct SearchAdvance {
    selected: Option<SearchRange>,
    completed: bool,
}

#[derive(Debug)]
pub(super) struct SearchNavigation {
    cursor: SourceOffset,
    source_end: SourceOffset,
    direction: NavigationDirection,
}

#[derive(Debug)]
enum NavigationDirection {
    Forward {
        after: SourceOffset,
        wrap: Option<SearchRange>,
    },
    Backward {
        before: SourceOffset,
        previous: Option<SearchRange>,
        wrap: Option<SearchRange>,
    },
}

impl SearchAdvance {
    pub(super) const fn selected(self) -> Option<SearchRange> {
        self.selected
    }

    pub(super) const fn completed(self) -> bool {
        self.completed
    }
}

#[derive(Debug)]
pub(super) struct SearchIndex {
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
}

impl SearchIndex {
    pub(super) fn new(
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
        checkpoints
            .try_reserve_exact(INITIAL_CHECKPOINT_RESERVATION.min(limits.max_checkpoints))
            .map_err(|_| SearchError::Allocation)?;
        checkpoints.push(SearchCheckpoint {
            scan_at: source_start,
            previous_match: None,
        });

        Ok(Self {
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
        })
    }

    pub(super) const fn is_complete(&self) -> bool {
        self.complete
    }

    pub(super) const fn has_matches(&self) -> bool {
        self.first_match.is_some()
    }

    pub(super) fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<SearchAdvance, TutError> {
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

    pub(super) fn navigation(
        &self,
        current: SearchRange,
        forward: bool,
    ) -> Option<SearchNavigation> {
        if !self.complete {
            return None;
        }
        if forward {
            let insertion = self.checkpoints.partition_point(|checkpoint| {
                checkpoint
                    .previous_match
                    .is_none_or(|range| range.start() < current.end())
            });
            let checkpoint = self.checkpoints[insertion.saturating_sub(1)];
            return Some(SearchNavigation {
                cursor: checkpoint.scan_at,
                source_end: self.source_end,
                direction: NavigationDirection::Forward {
                    after: current.end(),
                    wrap: self.first_match,
                },
            });
        }

        let checkpoint = self.checkpoint_at_or_before(current.start());
        Some(SearchNavigation {
            cursor: checkpoint.scan_at,
            source_end: self.source_end,
            direction: NavigationDirection::Backward {
                before: current.start(),
                previous: checkpoint
                    .previous_match
                    .filter(|range| range.start() < current.start()),
                wrap: self.last_match,
            },
        })
    }

    pub(super) fn replay(&self, visible: Range<SourceOffset>) -> Option<SearchReplay> {
        if visible.start >= visible.end || visible.start >= self.scanned_to {
            return None;
        }
        let earliest = visible
            .start
            .checked_sub(self.query_len.saturating_sub(1))
            .unwrap_or(self.source_start)
            .max(self.source_start);
        let checkpoint = self.checkpoint_at_or_before(earliest);
        Some(SearchReplay {
            visible,
            cursor: checkpoint.scan_at,
            stop: self.scanned_to,
            pending: Vec::new(),
            pending_index: 0,
            complete: false,
        })
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

impl SearchNavigation {
    pub(super) fn advance(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<SearchAdvance, TutError> {
        match &mut self.direction {
            NavigationDirection::Forward { after, wrap } => {
                if self.cursor >= self.source_end {
                    return Ok(SearchAdvance {
                        selected: *wrap,
                        completed: true,
                    });
                }
                let mut selected = None;
                self.cursor = scan_window(reader, needle, self.cursor, self.source_end, |range| {
                    if selected.is_none() && range.start() >= *after {
                        selected = Some(range);
                    }
                    Ok(())
                })?;
                let completed = selected.is_some() || self.cursor == self.source_end;
                Ok(SearchAdvance {
                    selected: if completed {
                        selected.or(*wrap)
                    } else {
                        selected
                    },
                    completed,
                })
            }
            NavigationDirection::Backward {
                before,
                previous,
                wrap,
            } => {
                if self.cursor >= *before {
                    return Ok(SearchAdvance {
                        selected: (*previous).or(*wrap),
                        completed: true,
                    });
                }
                self.cursor = scan_window(reader, needle, self.cursor, self.source_end, |range| {
                    if range.start() < *before {
                        *previous = Some(range);
                    }
                    Ok(())
                })?;
                let completed = self.cursor >= *before;
                Ok(SearchAdvance {
                    selected: completed.then_some((*previous).or(*wrap)).flatten(),
                    completed,
                })
            }
        }
    }
}

pub(super) struct SearchReplay {
    visible: Range<SourceOffset>,
    cursor: SourceOffset,
    stop: SourceOffset,
    pending: Vec<SearchRange>,
    pending_index: usize,
    complete: bool,
}

impl SearchReplay {
    pub(super) fn peek(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<Option<SearchRange>, TutError> {
        self.refill(reader, needle)?;
        Ok(self.pending.get(self.pending_index).copied())
    }

    pub(super) fn next(
        &mut self,
        reader: &mut DocumentReader<'_>,
        needle: &str,
    ) -> Result<Option<SearchRange>, TutError> {
        self.refill(reader, needle)?;
        let next = self.pending.get(self.pending_index).copied();
        self.pending_index += usize::from(next.is_some());
        Ok(next)
    }

    fn refill(&mut self, reader: &mut DocumentReader<'_>, needle: &str) -> Result<(), TutError> {
        while self.pending_index == self.pending.len() && !self.complete {
            self.pending.clear();
            self.pending_index = 0;
            if self.cursor >= self.stop || self.cursor >= self.visible.end {
                self.complete = true;
                break;
            }

            self.cursor = scan_window(reader, needle, self.cursor, reader.source_end(), |range| {
                if range.start() < self.stop
                    && range.start() < self.visible.end
                    && range.end() > self.visible.start
                {
                    if self.pending.len() == self.pending.capacity() {
                        self.pending
                            .try_reserve(1)
                            .map_err(|_| SearchError::Allocation)?;
                    }
                    self.pending.push(range);
                }
                Ok(())
            })?;
        }
        Ok(())
    }
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
    use std::path::Path;

    use super::*;
    use crate::document::{Document, DocumentCache};

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
        let mut replay = index.replay(visible).unwrap();
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut matches = Vec::new();
        while let Some(range) = replay.next(&mut reader, needle).unwrap() {
            matches.push(range);
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

    #[test]
    fn builds_a_global_nonoverlapping_index() {
        let (document, index) = complete("aaaaaa", "aa");
        assert_eq!(
            matches(
                &document,
                &index,
                "aa",
                SourceOffset::ZERO..SourceOffset::new(6)
            ),
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
    fn viewport_replay_includes_matches_crossing_its_start() {
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
    fn partial_replay_never_reads_beyond_the_committed_frontier() {
        let mut text = "cat".to_owned();
        text.push_str(&"x".repeat(SOURCE_WINDOW_BYTES * 2));
        text.push_str("cat");
        let source_end = SourceOffset::from_usize(text.len());
        let document = Document::from_text(Path::new("search.txt"), text);
        let mut cache = DocumentCache::default();
        let mut reader = document.reader(&mut cache);
        let mut index = SearchIndex::new(&reader, "cat", SourceOffset::ZERO)
            .unwrap()
            .unwrap();
        index.advance(&mut reader, "cat").unwrap();

        assert_eq!(
            matches(&document, &index, "cat", SourceOffset::ZERO..source_end),
            vec![SearchRange::new(SourceOffset::ZERO, SourceOffset::new(3)).unwrap()]
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
    }
}
