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
pub(super) struct ViewportLayout {
    width: ContentWidth,
    height: BodyHeight,
    source_start: SourceOffset,
    source_end: SourceOffset,
    max_top: SourceOffset,
}

impl ViewportLayout {
    pub(super) fn projected_row<'a>(
        &self,
        source: SourceText<'a>,
        start: SourceOffset,
    ) -> Option<ProjectedRowAtoms<'a>> {
        let range = self.row_range(source, start)?;
        Some(ProjectedRowAtoms {
            inner: DisplayAtoms::between(source, range.start, range.end)?,
            width: self.width,
            column: DisplayColumn::ZERO,
        })
    }

    pub(super) fn row_range(
        &self,
        source: SourceText<'_>,
        start: SourceOffset,
    ) -> Option<Range<SourceOffset>> {
        let boundary = self.row_boundary(source, start)?;
        Some(start..boundary.end)
    }

    pub(super) const fn max_top(&self) -> SourceOffset {
        self.max_top
    }

    pub(super) fn resolve_top(
        &self,
        source: SourceText<'_>,
        anchor: SourceOffset,
        follow_end: bool,
    ) -> Option<SourceOffset> {
        if follow_end {
            return self.matches(source).then_some(self.max_top);
        }
        Some(
            self.row_start_at_or_before(source, anchor)?
                .min(self.max_top),
        )
    }

    pub(super) fn move_row_start(
        &self,
        source: SourceText<'_>,
        start: SourceOffset,
        downward: bool,
        amount: usize,
    ) -> Option<SourceOffset> {
        let mut current = self.row_start_at_or_before(source, start)?;
        if downward {
            current = current.min(self.max_top);
        }
        for _ in 0..amount {
            let next = if downward {
                self.next_row_start(source, current)?
                    .unwrap_or(current)
                    .min(self.max_top)
            } else {
                previous_row_start(source, current, self.width)?
            };
            if next == current {
                break;
            }
            current = next;
        }
        Some(current.min(self.max_top))
    }

    pub(super) fn visible_extent(
        &self,
        source: SourceText<'_>,
        top: SourceOffset,
    ) -> Option<(usize, SourceOffset)> {
        if !self.matches(source) || top < self.source_start || top > self.max_top {
            return None;
        }

        let mut start = top;
        let mut visible_rows = 0;
        let mut visible_end = top;
        while visible_rows < usize::from(self.height.get()) {
            let boundary = self.row_boundary(source, start)?;
            visible_rows += 1;
            visible_end = boundary.end;
            let Some(next) = boundary.next else {
                break;
            };
            start = next;
        }
        Some((visible_rows, visible_end))
    }

    pub(super) fn next_row_start(
        &self,
        source: SourceText<'_>,
        start: SourceOffset,
    ) -> Option<Option<SourceOffset>> {
        Some(self.row_boundary(source, start)?.next)
    }

    pub(super) fn row_start_at_or_before(
        &self,
        source: SourceText<'_>,
        offset: SourceOffset,
    ) -> Option<SourceOffset> {
        if !self.matches(source) {
            return None;
        }
        row_start_at_or_before(source, offset, self.width)
    }

    fn row_boundary(
        &self,
        source: SourceText<'_>,
        start: SourceOffset,
    ) -> Option<VisualRowBoundary> {
        if !self.matches(source) {
            return None;
        }
        visual_row_boundary(source, start, self.width)
    }

    fn matches(&self, source: SourceText<'_>) -> bool {
        source.start() == self.source_start && source.end() == self.source_end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisualRowBoundary {
    end: SourceOffset,
    next: Option<SourceOffset>,
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

pub(super) fn rebuild_viewport_layout(
    slot: &mut Option<ViewportLayout>,
    source: SourceText<'_>,
    width: ContentWidth,
    height: BodyHeight,
) -> Result<(), LayoutError> {
    let max_top = last_viewport_start(source, width, height)?;
    *slot = Some(ViewportLayout {
        width,
        height,
        source_start: source.start(),
        source_end: source.end(),
        max_top,
    });
    Ok(())
}

pub(super) fn ensure_viewport_layout(
    slot: &mut Option<ViewportLayout>,
    source: SourceText<'_>,
    width: Option<ContentWidth>,
    height: Option<BodyHeight>,
) -> Result<bool, LayoutError> {
    let (Some(width), Some(height)) = (width, height) else {
        return Ok(false);
    };
    if slot.as_ref().is_some_and(|layout| {
        layout.width == width
            && layout.height == height
            && layout.source_start == source.start()
            && layout.source_end == source.end()
    }) {
        return Ok(false);
    }
    rebuild_viewport_layout(slot, source, width, height)?;
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

fn visual_row_boundary(
    source: SourceText<'_>,
    start: SourceOffset,
    width: ContentWidth,
) -> Option<VisualRowBoundary> {
    let relative = source.relative_offset(start)?;
    if !source.as_str().is_char_boundary(relative) {
        return None;
    }

    let mut column = DisplayColumn::ZERO;
    let mut consumed = false;
    let mut last_whitespace = None;
    for atom in DisplayAtoms::between(source, start, source.end())? {
        if atom.kind() == DisplayAtomKind::LineFeed {
            return Some(VisualRowBoundary {
                end: atom.source().end(),
                next: Some(atom.source().end()),
            });
        }

        let projected = atom
            .project(column, width)
            .expect("only line feeds have no projection");
        if projected.fits_after(column, width) {
            column = column.plus(projected.width());
            if atom.is_unicode_whitespace() {
                last_whitespace = Some(atom.source().end());
            }
            consumed = true;
            continue;
        }

        debug_assert!(consumed);
        let next = last_whitespace.unwrap_or_else(|| atom.source().start());
        return Some(VisualRowBoundary {
            end: next,
            next: Some(next),
        });
    }

    Some(VisualRowBoundary {
        end: source.end(),
        next: None,
    })
}

fn physical_line_start(source: SourceText<'_>, offset: SourceOffset) -> Option<SourceOffset> {
    let relative = source.relative_offset(offset)?;
    if !source.as_str().is_char_boundary(relative) {
        return None;
    }
    let bytes = source.as_str().as_bytes();
    for index in (0..relative).rev() {
        let line_end = match bytes[index] {
            b'\n' => index + 1,
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => index + 2,
            b'\r' => index + 1,
            _ => continue,
        };
        if line_end <= relative {
            return source.start().checked_add(line_end);
        }
    }
    Some(source.start())
}

fn previous_row_start(
    source: SourceText<'_>,
    start: SourceOffset,
    width: ContentWidth,
) -> Option<SourceOffset> {
    if start <= source.start() {
        return Some(source.start());
    }
    let relative = source.relative_offset(start)?;
    let previous = source.as_str()[..relative].char_indices().next_back()?.0;
    let probe = source.start().checked_add(previous)?;
    row_start_at_or_before(source, probe, width)
}

fn row_start_at_or_before(
    source: SourceText<'_>,
    offset: SourceOffset,
    width: ContentWidth,
) -> Option<SourceOffset> {
    let mut row_start = physical_line_start(source, offset)?;
    loop {
        let Some(next) = visual_row_boundary(source, row_start, width)?.next else {
            return Some(row_start);
        };
        if next > offset {
            return Some(row_start);
        }
        row_start = next;
        if row_start == offset {
            return Some(row_start);
        }
    }
}

fn last_viewport_start(
    source: SourceText<'_>,
    width: ContentWidth,
    height: BodyHeight,
) -> Result<SourceOffset, LayoutError> {
    let capacity = usize::from(height.get());
    let mut trailing = VecDeque::new();
    trailing
        .try_reserve_exact(capacity)
        .map_err(|_| LayoutError::Allocation)?;
    let mut start = source.start();

    loop {
        if trailing.len() == capacity {
            let _ = trailing.pop_front();
        }
        trailing.push_back(start);
        let Some(next) = visual_row_boundary(source, start, width)
            .expect("source starts are valid layout boundaries")
            .next
        else {
            break;
        };
        if next <= start {
            return Err(LayoutError::NonIncreasingRowStart {
                previous: start.get(),
                next: next.get(),
            });
        }
        start = next;
    }

    Ok(*trailing
        .front()
        .expect("nonzero viewport height retains a row"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_for(source: SourceText<'_>, width: u16, height: u16) -> ViewportLayout {
        let mut slot = None;
        rebuild_viewport_layout(
            &mut slot,
            source,
            ContentWidth::new(width).unwrap(),
            BodyHeight::new(height).unwrap(),
        )
        .unwrap();
        slot.unwrap()
    }

    fn layout(text: &str, width: u16) -> ViewportLayout {
        layout_for(SourceText::new(text), width, 2)
    }

    fn row_starts(layout: &ViewportLayout, source: SourceText<'_>) -> Vec<SourceOffset> {
        let mut starts = vec![source.start()];
        while let Some(next) = layout
            .next_row_start(source, *starts.last().unwrap())
            .unwrap()
        {
            starts.push(next);
        }
        starts
    }

    fn projected(layout: &ViewportLayout, text: &str, row: usize) -> Vec<(String, u32)> {
        let source = SourceText::new(text);
        let start = row_starts(layout, source)[row];
        layout
            .projected_row(source, start)
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
        let layout = layout(text, 16);
        assert_eq!(
            projected(&layout, text, 0),
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
        let layout = layout(text, 4);
        let starts: Vec<_> = row_starts(&layout, SourceText::new(text))
            .into_iter()
            .map(SourceOffset::get)
            .collect();
        assert_eq!(starts, vec![0, 4, 8, 9, 13]);
        assert_eq!(projected(&layout, text, 0)[3].0, " ");
        assert!(projected(&layout, text, 2).is_empty());
        assert!(projected(&layout, text, 4).is_empty());
    }

    #[test]
    fn a_tab_that_cannot_fit_on_a_fresh_row_becomes_one_cell() {
        let text = "a\t";
        let layout = layout(text, 2);
        assert_eq!(
            row_starts(&layout, SourceText::new(text)),
            [SourceOffset::ZERO, SourceOffset::new(1)]
        );
        assert_eq!(projected(&layout, text, 1), vec![("�".to_owned(), 1)]);
    }

    #[test]
    fn viewport_resolution_and_progress_are_clamped() {
        let source = SourceText::new("abcdefgh");
        let layout = layout_for(source, 2, 2);
        assert_eq!(layout.max_top(), SourceOffset::new(4));
        assert_eq!(
            layout.resolve_top(source, SourceOffset::new(7), false),
            Some(SourceOffset::new(4))
        );
        assert_eq!(
            layout.move_row_start(source, SourceOffset::ZERO, true, 1),
            Some(SourceOffset::new(2))
        );
        assert_eq!(
            layout.move_row_start(source, SourceOffset::new(2), false, 1),
            Some(SourceOffset::ZERO)
        );
        assert_eq!(
            layout.move_row_start(source, SourceOffset::ZERO, true, usize::MAX),
            Some(SourceOffset::new(4))
        );
        assert_eq!(progress_percent(source, SourceOffset::new(4)), 50);
        assert_eq!(
            progress_percent(SourceText::new(""), SourceOffset::ZERO),
            100
        );
    }

    #[test]
    fn raw_line_endings_keep_large_source_offsets() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 11);
        let source = SourceText::with_start("a\r\nb\rc\n", start).unwrap();
        let layout = layout_for(source, 8, 2);

        assert_eq!(
            row_starts(&layout, source),
            vec![
                start,
                start.checked_add(3).unwrap(),
                start.checked_add(5).unwrap(),
                start.checked_add(7).unwrap(),
            ]
        );
        assert_eq!(progress_percent(source, start.checked_add(3).unwrap()), 42);
        assert_eq!(
            layout
                .projected_row(source, start)
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
                let layout = layout(text, width);
                let starts = row_starts(&layout, source);
                assert_eq!(starts.first(), Some(&source.start()));
                assert!(starts.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(starts.iter().all(|offset| {
                    source
                        .relative_offset(*offset)
                        .is_some_and(|offset| text.is_char_boundary(offset))
                }));
                assert!(starts.len() <= text.graphemes(true).count() + 1);

                for start in &starts {
                    let range = layout.row_range(source, *start).unwrap();
                    assert!(text.is_char_boundary(source.relative_offset(range.start).unwrap()));
                    assert!(text.is_char_boundary(source.relative_offset(range.end).unwrap()));
                    let used: u32 = layout
                        .projected_row(source, *start)
                        .unwrap()
                        .map(|atom| atom.width().get())
                        .sum();
                    assert!(used <= u32::from(width));
                }

                assert_eq!(
                    layout
                        .row_range(source, *starts.last().unwrap())
                        .unwrap()
                        .end,
                    source.end()
                );
            }
        }
    }

    #[test]
    fn local_navigation_round_trips_every_row_boundary() {
        let samples = [
            "",
            "\n",
            "\r\n",
            "a\rb\r\nc\n",
            "one two three four",
            "e\u{301}🙂\u{200b}\ttext",
        ];

        for text in samples {
            for width in 1..=8 {
                let source = SourceText::new(text);
                let layout = layout_for(source, width, 1);
                let starts = row_starts(&layout, source);

                for (index, start) in starts.iter().copied().enumerate() {
                    assert_eq!(layout.row_start_at_or_before(source, start), Some(start));
                    let expected_up = starts[index.saturating_sub(1)];
                    assert_eq!(
                        layout.move_row_start(source, start, false, 1),
                        Some(expected_up)
                    );
                    let expected_down = starts.get(index + 1).copied().unwrap_or(start);
                    assert_eq!(
                        layout.move_row_start(source, start, true, 1),
                        Some(expected_down)
                    );
                }

                for relative in text
                    .char_indices()
                    .map(|(relative, _)| relative)
                    .chain(std::iter::once(text.len()))
                {
                    let offset = source.start().checked_add(relative).unwrap();
                    let expected = starts
                        .iter()
                        .copied()
                        .take_while(|start| *start <= offset)
                        .last()
                        .unwrap();
                    assert_eq!(
                        layout.row_start_at_or_before(source, offset),
                        Some(expected)
                    );
                }
            }
        }
    }
}
