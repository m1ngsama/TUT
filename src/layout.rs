use std::{collections::VecDeque, num::NonZeroU16, ops::Range};

use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

use crate::{
    error::LayoutError,
    source::{SourceOffset, SourceText},
};

pub(super) const REPLACEMENT_CHARACTER: &str = "\u{fffd}";
pub(super) const DOTTED_CIRCLE: &str = "\u{25cc}";
const TAB_STOP: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub(super) struct DisplayColumn(u32);

impl DisplayColumn {
    pub(super) const ZERO: Self = Self(0);

    pub(super) const fn new(value: u32) -> Self {
        Self(value)
    }

    pub(super) const fn get(self) -> u32 {
        self.0
    }

    const fn plus(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub(super) struct VisualRowIndex(u32);

impl VisualRowIndex {
    pub(super) const ZERO: Self = Self(0);

    pub(super) const fn new(value: u32) -> Self {
        Self(value)
    }

    pub(super) const fn get(self) -> u32 {
        self.0
    }

    pub(super) const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(super) struct ContentWidth(NonZeroU16);

impl ContentWidth {
    pub(super) const fn new(value: u16) -> Option<Self> {
        match NonZeroU16::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub(super) const fn get(self) -> u16 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub(super) struct BodyHeight(NonZeroU16);

impl BodyHeight {
    pub(super) const fn new(value: u16) -> Option<Self> {
        match NonZeroU16::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub(super) const fn get(self) -> u16 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GraphemeRange {
    start: SourceOffset,
    end: SourceOffset,
}

impl GraphemeRange {
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
pub(super) enum DisplayAtomKind {
    LineFeed,
    Tab,
    Control,
    ZeroWidth,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayProjection<'a> {
    Text(&'a str),
    Spaces(u8),
    Replacement,
    DottedCircle(&'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProjectedAtom<'a> {
    source: GraphemeRange,
    width: DisplayColumn,
    projection: DisplayProjection<'a>,
}

impl<'a> ProjectedAtom<'a> {
    pub(super) const fn source(self) -> GraphemeRange {
        self.source
    }

    pub(super) const fn width(self) -> DisplayColumn {
        self.width
    }

    pub(super) const fn projection(self) -> DisplayProjection<'a> {
        self.projection
    }

    fn fits_after(self, column: DisplayColumn, content_width: ContentWidth) -> bool {
        u64::from(column.get()) + u64::from(self.width.get()) <= u64::from(content_width.get())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplayAtom<'a> {
    source: GraphemeRange,
    source_text: &'a str,
    kind: DisplayAtomKind,
    unicode_whitespace: bool,
    measured_width: DisplayColumn,
}

impl<'a> DisplayAtom<'a> {
    pub(super) const fn source(self) -> GraphemeRange {
        self.source
    }

    pub(super) const fn kind(self) -> DisplayAtomKind {
        self.kind
    }

    pub(super) const fn is_unicode_whitespace(self) -> bool {
        self.unicode_whitespace
    }

    pub(super) fn project(
        self,
        column: DisplayColumn,
        content_width: ContentWidth,
    ) -> Option<ProjectedAtom<'a>> {
        let replacement = || ProjectedAtom {
            source: self.source,
            width: DisplayColumn::new(1),
            projection: DisplayProjection::Replacement,
        };

        Some(match self.kind {
            DisplayAtomKind::LineFeed => return None,
            DisplayAtomKind::Tab => {
                let expansion = TAB_STOP - (column.get() % TAB_STOP);
                if column == DisplayColumn::ZERO && expansion > u32::from(content_width.get()) {
                    replacement()
                } else {
                    ProjectedAtom {
                        source: self.source,
                        width: DisplayColumn::new(expansion),
                        projection: DisplayProjection::Spaces(
                            u8::try_from(expansion).expect("tab expansion is at most four"),
                        ),
                    }
                }
            }
            DisplayAtomKind::Control => replacement(),
            DisplayAtomKind::ZeroWidth => ProjectedAtom {
                source: self.source,
                width: DisplayColumn::new(1),
                projection: DisplayProjection::DottedCircle(self.source_text),
            },
            DisplayAtomKind::Text if self.measured_width.get() > u32::from(content_width.get()) => {
                replacement()
            }
            DisplayAtomKind::Text => ProjectedAtom {
                source: self.source,
                width: self.measured_width,
                projection: DisplayProjection::Text(self.source_text),
            },
        })
    }
}

pub(super) struct DisplayAtoms<'a> {
    base: SourceOffset,
    inner: GraphemeIndices<'a>,
}

impl<'a> DisplayAtoms<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self::from_source(SourceText::new(text))
    }

    fn from_source(source: SourceText<'a>) -> Self {
        Self {
            base: source.start(),
            inner: source.as_str().grapheme_indices(true),
        }
    }

    fn between(source: SourceText<'a>, start: SourceOffset, end: SourceOffset) -> Option<Self> {
        Some(Self {
            base: start,
            inner: source.slice(start..end)?.grapheme_indices(true),
        })
    }
}

impl<'a> Iterator for DisplayAtoms<'a> {
    type Item = DisplayAtom<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (relative_start, grapheme) = self.inner.next()?;
        let measured_width = u32::try_from(UnicodeWidthStr::width(grapheme)).unwrap_or(u32::MAX);
        let start = self
            .base
            .checked_add(relative_start)
            .expect("source span coordinates were validated");
        let end = start
            .checked_add(grapheme.len())
            .expect("source span coordinates were validated");

        let kind = if matches!(grapheme, "\n" | "\r" | "\r\n") {
            DisplayAtomKind::LineFeed
        } else if grapheme == "\t" {
            DisplayAtomKind::Tab
        } else if grapheme.chars().any(is_terminal_control) {
            DisplayAtomKind::Control
        } else if measured_width == 0 {
            DisplayAtomKind::ZeroWidth
        } else {
            DisplayAtomKind::Text
        };

        Some(DisplayAtom {
            source: GraphemeRange::new(start, end)
                .expect("segmentation never yields an empty grapheme"),
            source_text: grapheme,
            kind,
            unicode_whitespace: grapheme.chars().all(char::is_whitespace),
            measured_width: DisplayColumn::new(measured_width),
        })
    }
}

fn is_terminal_control(character: char) -> bool {
    matches!(
        character,
        '\u{0000}'..='\u{001f}' | '\u{007f}' | '\u{0080}'..='\u{009f}'
    )
}

#[derive(Debug)]
pub(super) struct WrapIndex {
    width: ContentWidth,
    source_start: SourceOffset,
    source_end: SourceOffset,
    starts: Vec<SourceOffset>,
}

impl WrapIndex {
    pub(super) fn row_count(&self) -> usize {
        self.starts.len()
    }

    pub(super) fn row_start(&self, row: VisualRowIndex) -> Option<SourceOffset> {
        self.starts.get(row.as_usize()).copied()
    }

    pub(super) fn row_range(&self, row: VisualRowIndex) -> Option<Range<SourceOffset>> {
        let row = row.as_usize();
        let start = *self.starts.get(row)?;
        let end = self.starts.get(row + 1).copied().unwrap_or(self.source_end);
        Some(start..end)
    }

    pub(super) fn projected_row<'a>(
        &self,
        source: SourceText<'a>,
        row: VisualRowIndex,
    ) -> Option<ProjectedRowAtoms<'a>> {
        if source.start() != self.source_start || source.end() != self.source_end {
            return None;
        }
        let range = self.row_range(row)?;
        Some(ProjectedRowAtoms {
            inner: DisplayAtoms::between(source, range.start, range.end)?,
            width: self.width,
            column: DisplayColumn::ZERO,
        })
    }

    pub(super) fn row_at_or_before(&self, offset: SourceOffset) -> VisualRowIndex {
        let insertion = self.starts.partition_point(|start| *start <= offset);
        VisualRowIndex::new(
            u32::try_from(insertion.saturating_sub(1)).expect("source bounds row indices"),
        )
    }

    pub(super) fn max_top(&self, body_height: BodyHeight) -> VisualRowIndex {
        let row = self
            .row_count()
            .saturating_sub(usize::from(body_height.get()));
        VisualRowIndex::new(u32::try_from(row).expect("source bounds row indices"))
    }

    pub(super) fn resolve_top(
        &self,
        anchor: SourceOffset,
        follow_end: bool,
        body_height: BodyHeight,
    ) -> VisualRowIndex {
        let max_top = self.max_top(body_height);
        if follow_end {
            max_top
        } else {
            self.row_at_or_before(anchor).min(max_top)
        }
    }

    pub(super) fn visible_end(&self, top: VisualRowIndex, body_height: BodyHeight) -> SourceOffset {
        let top = top.as_usize().min(self.row_count().saturating_sub(1));
        let exclusive = (top + usize::from(body_height.get())).min(self.row_count());
        self.starts
            .get(exclusive)
            .copied()
            .unwrap_or(self.source_end)
    }
}

pub(super) struct ProjectedRowAtoms<'a> {
    inner: DisplayAtoms<'a>,
    width: ContentWidth,
    column: DisplayColumn,
}

impl<'a> Iterator for ProjectedRowAtoms<'a> {
    type Item = ProjectedAtom<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let projected = self.inner.next()?.project(self.column, self.width);
            let Some(projected) = projected else {
                continue;
            };
            debug_assert!(projected.fits_after(self.column, self.width));
            self.column = self.column.plus(projected.width());
            return Some(projected);
        }
    }
}

pub(super) fn rebuild_wrap_index(
    slot: &mut Option<WrapIndex>,
    source: SourceText<'_>,
    width: ContentWidth,
) -> Result<(), LayoutError> {
    let mut index = slot.take().unwrap_or_else(|| WrapIndex {
        width,
        source_start: source.start(),
        source_end: source.end(),
        starts: Vec::new(),
    });

    index.width = width;
    index.source_start = source.start();
    index.source_end = source.end();
    index.starts.clear();
    build_row_starts(&mut index.starts, source, width)?;
    *slot = Some(index);
    Ok(())
}

pub(super) fn ensure_wrap_index(
    slot: &mut Option<WrapIndex>,
    source: SourceText<'_>,
    width: Option<ContentWidth>,
) -> Result<bool, LayoutError> {
    let Some(width) = width else {
        return Ok(false);
    };
    if slot.as_ref().is_some_and(|index| {
        index.width == width
            && index.source_start == source.start()
            && index.source_end == source.end()
    }) {
        return Ok(false);
    }
    rebuild_wrap_index(slot, source, width)?;
    Ok(true)
}

pub(super) fn progress_percent(source: SourceText<'_>, visible_end: SourceOffset) -> u8 {
    if source.start() == source.end() || visible_end >= source.end() {
        return 100;
    }
    let total = source.end().get() - source.start().get();
    let visible = visible_end.get().saturating_sub(source.start().get());
    let percent = (u128::from(visible) * 100) / u128::from(total);
    u8::try_from(percent.min(99)).expect("progress is clamped to 99")
}

fn build_row_starts(
    starts: &mut Vec<SourceOffset>,
    source: SourceText<'_>,
    width: ContentWidth,
) -> Result<(), LayoutError> {
    try_push_row_start(starts, source.start())?;
    let mut atoms = DisplayAtoms::from_source(source);
    let mut pending = VecDeque::<DisplayAtom<'_>>::new();

    'rows: loop {
        let mut consumed = 0usize;
        let mut column = DisplayColumn::ZERO;
        let mut last_whitespace: Option<(usize, SourceOffset)> = None;

        loop {
            if consumed == pending.len() {
                match atoms.next() {
                    Some(atom) => try_push_pending(&mut pending, atom)?,
                    None => {
                        remove_front(&mut pending, consumed);
                        break 'rows;
                    }
                }
            }

            let atom = pending[consumed];
            if atom.kind() == DisplayAtomKind::LineFeed {
                let next_start = atom.source().end();
                remove_front(&mut pending, consumed + 1);
                try_push_row_start(starts, next_start)?;
                continue 'rows;
            }

            let projected = atom
                .project(column, width)
                .expect("only line feeds have no projection");
            if projected.fits_after(column, width) {
                column = column.plus(projected.width());
                if atom.is_unicode_whitespace() {
                    last_whitespace = Some((consumed, atom.source().end()));
                }
                consumed += 1;
                continue;
            }

            debug_assert!(consumed > 0);
            let next_start = if let Some((whitespace_index, offset)) = last_whitespace {
                remove_front(&mut pending, whitespace_index + 1);
                offset
            } else {
                remove_front(&mut pending, consumed);
                pending
                    .front()
                    .expect("overflow atom remains pending")
                    .source()
                    .start()
            };
            try_push_row_start(starts, next_start)?;
            continue 'rows;
        }
    }

    Ok(())
}

fn try_push_pending<'a>(
    pending: &mut VecDeque<DisplayAtom<'a>>,
    atom: DisplayAtom<'a>,
) -> Result<(), LayoutError> {
    if pending.len() == pending.capacity() {
        pending
            .try_reserve(1)
            .map_err(|_| LayoutError::Allocation)?;
    }
    pending.push_back(atom);
    Ok(())
}

fn remove_front<T>(pending: &mut VecDeque<T>, count: usize) {
    for _ in 0..count {
        let _ = pending.pop_front();
    }
}

fn try_push_row_start(
    starts: &mut Vec<SourceOffset>,
    start: SourceOffset,
) -> Result<(), LayoutError> {
    if let Some(previous) = starts.last().copied()
        && previous >= start
    {
        return Err(LayoutError::NonIncreasingRowStart {
            previous: previous.get(),
            next: start.get(),
        });
    }
    if starts.len() == starts.capacity() {
        starts.try_reserve(1).map_err(|_| LayoutError::Allocation)?;
    }
    starts.push(start);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &str, width: u16) -> WrapIndex {
        let mut slot = None;
        rebuild_wrap_index(
            &mut slot,
            SourceText::new(text),
            ContentWidth::new(width).unwrap(),
        )
        .unwrap();
        slot.unwrap()
    }

    fn projected(index: &WrapIndex, text: &str, row: u32) -> Vec<(String, u32)> {
        index
            .projected_row(SourceText::new(text), VisualRowIndex::new(row))
            .unwrap()
            .map(|atom| {
                let text = match atom.projection() {
                    DisplayProjection::Text(text) => text.to_owned(),
                    DisplayProjection::Spaces(count) => " ".repeat(usize::from(count)),
                    DisplayProjection::Replacement => REPLACEMENT_CHARACTER.to_owned(),
                    DisplayProjection::DottedCircle(source) => format!("{DOTTED_CIRCLE}{source}"),
                };
                (text, atom.width().get())
            })
            .collect()
    }

    #[test]
    fn projection_sanitizes_controls_tabs_and_zero_width_graphemes() {
        let text = "a\t\u{001b}\u{200b}ｶﾞ";
        let index = index(text, 16);
        assert_eq!(
            projected(&index, text, 0),
            vec![
                ("a".to_owned(), 1),
                ("   ".to_owned(), 3),
                ("�".to_owned(), 1),
                ("◌\u{200b}".to_owned(), 1),
                ("ｶﾞ".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn wrapping_is_greedy_and_preserves_whitespace_and_blank_lines() {
        let text = "one two\n\nend\n";
        let index = index(text, 4);
        let starts: Vec<_> = index.starts.iter().map(|offset| offset.get()).collect();
        assert_eq!(starts, vec![0, 4, 8, 9, 13]);
        assert_eq!(projected(&index, text, 0)[3].0, " ");
        assert!(projected(&index, text, 2).is_empty());
        assert!(projected(&index, text, 4).is_empty());
    }

    #[test]
    fn a_tab_that_cannot_fit_on_a_fresh_row_becomes_one_cell() {
        let text = "a\t";
        let index = index(text, 2);
        assert_eq!(index.starts, &[SourceOffset::ZERO, SourceOffset::new(1)]);
        assert_eq!(projected(&index, text, 1), vec![("�".to_owned(), 1)]);
    }

    #[test]
    fn viewport_resolution_and_progress_are_clamped() {
        let index = index("abcdefgh", 2);
        let height = BodyHeight::new(2).unwrap();
        assert_eq!(index.max_top(height), VisualRowIndex::new(2));
        assert_eq!(
            index.resolve_top(SourceOffset::new(7), false, height),
            VisualRowIndex::new(2)
        );
        assert_eq!(
            progress_percent(SourceText::new("abcdefgh"), SourceOffset::new(4)),
            50
        );
        assert_eq!(
            progress_percent(SourceText::new(""), SourceOffset::ZERO),
            100
        );
    }

    #[test]
    fn raw_line_endings_keep_large_source_offsets() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 11);
        let source = SourceText::with_start("a\r\nb\rc\n", start).unwrap();
        let mut slot = None;
        rebuild_wrap_index(&mut slot, source, ContentWidth::new(8).unwrap()).unwrap();
        let index = slot.unwrap();

        assert_eq!(
            index.starts,
            vec![
                start,
                start.checked_add(3).unwrap(),
                start.checked_add(5).unwrap(),
                start.checked_add(7).unwrap(),
            ]
        );
        assert_eq!(progress_percent(source, start.checked_add(3).unwrap()), 42);
        assert_eq!(
            index
                .projected_row(source, VisualRowIndex::ZERO)
                .unwrap()
                .map(|atom| atom.projection())
                .collect::<Vec<_>>(),
            vec![DisplayProjection::Text("a")]
        );
    }

    #[test]
    fn every_projected_row_respects_source_boundaries_and_width() {
        let samples = [
            "",
            "\n",
            "\n\n",
            "a\n",
            "one two three",
            "\twide\ttext",
            "a\u{001b}\u{0085}b",
            "e\u{301}\u{200b}🙂",
        ];

        for text in samples {
            for width in 1..=16 {
                let source = SourceText::new(text);
                let index = index(text, width);
                assert_eq!(index.starts.first(), Some(&source.start()));
                assert!(index.starts.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(index.starts.iter().all(|offset| {
                    source
                        .relative_offset(*offset)
                        .is_some_and(|offset| text.is_char_boundary(offset))
                }));
                assert!(index.row_count() <= text.graphemes(true).count() + 1);

                for row in 0..index.row_count() {
                    let row = VisualRowIndex::new(u32::try_from(row).unwrap());
                    let range = index.row_range(row).unwrap();
                    assert!(text.is_char_boundary(source.relative_offset(range.start).unwrap()));
                    assert!(text.is_char_boundary(source.relative_offset(range.end).unwrap()));
                    let used: u32 = index
                        .projected_row(source, row)
                        .unwrap()
                        .map(|atom| atom.width().get())
                        .sum();
                    assert!(used <= u32::from(width));
                }

                assert_eq!(
                    index
                        .row_range(VisualRowIndex::new(
                            u32::try_from(index.row_count() - 1).unwrap()
                        ))
                        .unwrap()
                        .end,
                    source.end()
                );
            }
        }
    }
}
