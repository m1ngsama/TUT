use std::num::{NonZeroU16, NonZeroUsize};

#[cfg(test)]
use std::ops::Range;

use unicode_segmentation::{GraphemeIndices, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

use crate::{
    document::{DocumentId, DocumentReader, SourceGrapheme},
    error::{LayoutError, TutError, is_terminal_control},
    source::{SourceOffset, SourceText},
};

pub(super) const REPLACEMENT_CHARACTER: &str = "\u{fffd}";
pub(super) const DOTTED_CIRCLE: &str = "\u{25cc}";
const MAX_RENDER_GRAPHEME_BYTES: usize = 1024;
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
    Oversized,
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
            DisplayAtomKind::Control | DisplayAtomKind::Oversized => replacement(),
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
}

impl<'a> Iterator for DisplayAtoms<'a> {
    type Item = DisplayAtom<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (relative_start, grapheme) = self.inner.next()?;
        let start = self
            .base
            .checked_add(relative_start)
            .expect("source span coordinates were validated");
        let end = start
            .checked_add(grapheme.len())
            .expect("source span coordinates were validated");
        Some(DisplayAtom::from_text(
            GraphemeRange::new(start, end).expect("segmentation never yields an empty grapheme"),
            grapheme,
        ))
    }
}

impl<'a> DisplayAtom<'a> {
    fn from_grapheme(grapheme: SourceGrapheme<'a>) -> Self {
        let source = GraphemeRange::new(grapheme.start(), grapheme.end())
            .expect("segmentation never yields an empty grapheme");
        match grapheme.text() {
            Some(text) => Self::from_text(source, text),
            None => Self {
                source,
                source_text: "",
                kind: DisplayAtomKind::Oversized,
                unicode_whitespace: false,
                measured_width: DisplayColumn::new(1),
            },
        }
    }

    fn from_text(source: GraphemeRange, grapheme: &'a str) -> Self {
        if grapheme.len() > MAX_RENDER_GRAPHEME_BYTES {
            return Self {
                source,
                source_text: "",
                kind: DisplayAtomKind::Oversized,
                unicode_whitespace: false,
                measured_width: DisplayColumn::new(1),
            };
        }
        let measured_width = u32::try_from(UnicodeWidthStr::width(grapheme)).unwrap_or(u32::MAX);
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

        Self {
            source,
            source_text: grapheme,
            kind,
            unicode_whitespace: grapheme.chars().all(char::is_whitespace),
            measured_width: DisplayColumn::new(measured_width),
        }
    }
}

#[derive(Debug)]
pub(super) struct ViewportLayout {
    document_id: DocumentId,
    width: ContentWidth,
    height: BodyHeight,
    source_start: SourceOffset,
    source_end: SourceOffset,
}

pub(super) trait ProjectedRowSink {
    type Checkpoint: Copy;

    fn checkpoint(&self) -> Self::Checkpoint;
    fn push(&mut self, atom: ProjectedAtom<'_>) -> Result<(), TutError>;
    fn finish_row(&mut self, through: Self::Checkpoint, carry_tail: bool) -> Result<(), TutError>;
}

impl ViewportLayout {
    pub(super) const fn row_cache_key(&self) -> (DocumentId, ContentWidth) {
        (self.document_id, self.width)
    }

    pub(super) fn project_visible_rows<S>(
        &self,
        reader: &mut DocumentReader<'_>,
        start: SourceOffset,
        sink: &mut S,
    ) -> Result<(usize, SourceOffset), TutError>
    where
        S: ProjectedRowSink,
    {
        self.require_visible_start(reader, start)?;
        let extent = scan_projected_rows(
            reader,
            start,
            self.width,
            NonZeroUsize::new(usize::from(self.height.get())).expect("body heights are nonzero"),
            sink,
        )?;
        Ok((extent.rows, extent.boundary.end))
    }

    #[cfg(test)]
    fn visit_projected_row<F>(
        &self,
        reader: &mut DocumentReader<'_>,
        start: SourceOffset,
        mut visit: F,
    ) -> Result<Option<SourceOffset>, TutError>
    where
        F: FnMut(ProjectedAtom<'_>) -> Result<(), TutError>,
    {
        let boundary = reference_visual_row_boundary(reader, start, self.width)?;
        let mut graphemes = reader.graphemes(start)?;
        let mut column = DisplayColumn::ZERO;
        while let Some(grapheme) = graphemes.next_grapheme()? {
            if grapheme.start() >= boundary.end {
                break;
            }
            let atom = DisplayAtom::from_grapheme(grapheme);
            let Some(projected) = atom.project(column, self.width) else {
                continue;
            };
            debug_assert!(projected.fits_after(column, self.width));
            column = column.plus(projected.width());
            visit(projected)?;
        }
        Ok(boundary.next)
    }

    #[cfg(test)]
    pub(super) fn row_range(
        &self,
        reader: &mut DocumentReader<'_>,
        start: SourceOffset,
    ) -> Result<Range<SourceOffset>, TutError> {
        let boundary = self.row_boundary(reader, start)?;
        Ok(start..boundary.end)
    }

    #[cfg(test)]
    pub(super) fn resolve_top(
        &self,
        reader: &mut DocumentReader<'_>,
        anchor: SourceOffset,
    ) -> Result<SourceOffset, TutError> {
        self.require_matching_source(reader)?;
        let top = self.row_start_at_or_before(reader, anchor)?;
        self.clamp_to_last_viewport(reader, top)
    }

    #[cfg(test)]
    pub(super) fn move_row_start(
        &self,
        reader: &mut DocumentReader<'_>,
        start: SourceOffset,
        downward: bool,
        amount: usize,
    ) -> Result<SourceOffset, TutError> {
        let mut current = self.row_start_at_or_before(reader, start)?;
        for _ in 0..amount {
            let next = if downward {
                self.next_row_start(reader, current)?.unwrap_or(current)
            } else {
                previous_row_start(reader, current, self.width)?
            };
            if next == current {
                break;
            }
            current = next;
        }
        self.clamp_to_last_viewport(reader, current)
    }

    pub(super) fn visible_extent(
        &self,
        reader: &mut DocumentReader<'_>,
        top: SourceOffset,
    ) -> Result<(usize, SourceOffset), TutError> {
        self.require_visible_start(reader, top)?;

        let mut sink = DiscardProjectedRows;
        let extent = scan_projected_rows(
            reader,
            top,
            self.width,
            NonZeroUsize::new(usize::from(self.height.get())).expect("body heights are nonzero"),
            &mut sink,
        )?;
        Ok((extent.rows, extent.boundary.end))
    }

    #[cfg(test)]
    pub(super) fn is_last_viewport(
        &self,
        reader: &mut DocumentReader<'_>,
        top: SourceOffset,
    ) -> Result<bool, TutError> {
        self.require_matching_source(reader)?;
        let mut sink = DiscardProjectedRows;
        let extent = scan_projected_rows(
            reader,
            top,
            self.width,
            NonZeroUsize::new(usize::from(self.height.get())).expect("body heights are nonzero"),
            &mut sink,
        )?;
        Ok(extent.boundary.next.is_none())
    }

    #[cfg(test)]
    pub(super) fn last_viewport_start(
        &self,
        reader: &mut DocumentReader<'_>,
    ) -> Result<SourceOffset, TutError> {
        self.require_matching_source(reader)?;
        let last_row = self.row_start_at_or_before(reader, self.source_end)?;
        self.clamp_to_last_viewport(reader, last_row)
    }

    pub(super) fn next_row_start(
        &self,
        reader: &mut DocumentReader<'_>,
        start: SourceOffset,
    ) -> Result<Option<SourceOffset>, TutError> {
        Ok(self.row_boundary(reader, start)?.next)
    }

    #[cfg(test)]
    pub(super) fn row_start_at_or_before(
        &self,
        reader: &mut DocumentReader<'_>,
        offset: SourceOffset,
    ) -> Result<SourceOffset, TutError> {
        self.require_matching_source(reader)?;
        row_start_at_or_before(reader, offset, self.width)
    }

    fn row_boundary(
        &self,
        reader: &mut DocumentReader<'_>,
        start: SourceOffset,
    ) -> Result<VisualRowBoundary, TutError> {
        self.require_matching_source(reader)?;
        visual_row_boundary(reader, start, self.width)
    }

    #[cfg(test)]
    fn clamp_to_last_viewport(
        &self,
        reader: &mut DocumentReader<'_>,
        top: SourceOffset,
    ) -> Result<SourceOffset, TutError> {
        let (visible_rows, _) = self.visible_extent(reader, top)?;
        let mut top = top;
        for _ in visible_rows..usize::from(self.height.get()) {
            let previous = previous_row_start(reader, top, self.width)?;
            if previous == top {
                break;
            }
            top = previous;
        }
        Ok(top)
    }

    fn require_matching_source(&self, reader: &DocumentReader<'_>) -> Result<(), TutError> {
        if reader.document_id() != self.document_id {
            return Err(LayoutError::DocumentMismatch.into());
        }
        if reader.source_start() != self.source_start || reader.source_end() != self.source_end {
            return Err(LayoutError::SourceRangeMismatch {
                expected_start: self.source_start.get(),
                expected_end: self.source_end.get(),
                actual_start: reader.source_start().get(),
                actual_end: reader.source_end().get(),
            }
            .into());
        }
        Ok(())
    }

    fn require_visible_start(
        &self,
        reader: &DocumentReader<'_>,
        start: SourceOffset,
    ) -> Result<(), TutError> {
        self.require_matching_source(reader)?;
        if start >= self.source_start && start <= self.source_end {
            return Ok(());
        }
        Err(LayoutError::NonIncreasingRowStart {
            previous: self.source_start.get(),
            next: start.get(),
        }
        .into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisualRowBoundary {
    end: SourceOffset,
    next: Option<SourceOffset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProjectedRowsExtent {
    rows: usize,
    boundary: VisualRowBoundary,
}

#[derive(Debug, Clone, Copy)]
struct SoftWrap<C> {
    source_end: SourceOffset,
    checkpoint: C,
    column_after: DisplayColumn,
}

struct DiscardProjectedRows;

impl ProjectedRowSink for DiscardProjectedRows {
    type Checkpoint = ();

    fn checkpoint(&self) -> Self::Checkpoint {}

    fn push(&mut self, _atom: ProjectedAtom<'_>) -> Result<(), TutError> {
        Ok(())
    }

    fn finish_row(
        &mut self,
        _through: Self::Checkpoint,
        _carry_tail: bool,
    ) -> Result<(), TutError> {
        Ok(())
    }
}

pub(super) fn rebuild_viewport_layout(
    slot: &mut Option<ViewportLayout>,
    reader: &DocumentReader<'_>,
    width: ContentWidth,
    height: BodyHeight,
) {
    *slot = Some(ViewportLayout {
        document_id: reader.document_id(),
        width,
        height,
        source_start: reader.source_start(),
        source_end: reader.source_end(),
    });
}

pub(super) fn ensure_viewport_layout(
    slot: &mut Option<ViewportLayout>,
    reader: &DocumentReader<'_>,
    width: Option<ContentWidth>,
    height: Option<BodyHeight>,
) -> bool {
    let (Some(width), Some(height)) = (width, height) else {
        return false;
    };
    if slot.as_ref().is_some_and(|layout| {
        layout.document_id == reader.document_id()
            && layout.width == width
            && layout.height == height
            && layout.source_start == reader.source_start()
            && layout.source_end == reader.source_end()
    }) {
        return false;
    }
    rebuild_viewport_layout(slot, reader, width, height);
    true
}

pub(super) fn progress_percent(
    source_start: SourceOffset,
    source_end: SourceOffset,
    visible_end: SourceOffset,
) -> u8 {
    if source_start == source_end || visible_end >= source_end {
        return 100;
    }
    let total = source_end.get() - source_start.get();
    let visible = visible_end.get().saturating_sub(source_start.get());
    let percent = (u128::from(visible) * 100) / u128::from(total);
    u8::try_from(percent.min(99)).expect("progress is clamped to 99")
}

fn visual_row_boundary(
    reader: &mut DocumentReader<'_>,
    start: SourceOffset,
    width: ContentWidth,
) -> Result<VisualRowBoundary, TutError> {
    let mut sink = DiscardProjectedRows;
    Ok(scan_projected_rows(
        reader,
        start,
        width,
        NonZeroUsize::new(1).expect("one is nonzero"),
        &mut sink,
    )?
    .boundary)
}

fn scan_projected_rows<S>(
    reader: &mut DocumentReader<'_>,
    start: SourceOffset,
    width: ContentWidth,
    row_limit: NonZeroUsize,
    sink: &mut S,
) -> Result<ProjectedRowsExtent, TutError>
where
    S: ProjectedRowSink,
{
    let source_end = reader.source_end();
    let mut graphemes = reader.graphemes(start)?;
    let mut column = DisplayColumn::ZERO;
    let mut soft_wrap = None;
    let mut rows = 0;

    loop {
        let Some(grapheme) = graphemes.next_grapheme()? else {
            let checkpoint = sink.checkpoint();
            sink.finish_row(checkpoint, false)?;
            return Ok(ProjectedRowsExtent {
                rows: rows + 1,
                boundary: VisualRowBoundary {
                    end: source_end,
                    next: None,
                },
            });
        };
        let atom = DisplayAtom::from_grapheme(grapheme);
        if atom.kind() == DisplayAtomKind::LineFeed {
            let checkpoint = sink.checkpoint();
            sink.finish_row(checkpoint, false)?;
            rows += 1;
            let boundary = VisualRowBoundary {
                end: atom.source().end(),
                next: Some(atom.source().end()),
            };
            if rows == row_limit.get() {
                return Ok(ProjectedRowsExtent { rows, boundary });
            }
            column = DisplayColumn::ZERO;
            soft_wrap = None;
            continue;
        }

        loop {
            let projected = atom
                .project(column, width)
                .expect("only line feeds have no projection");
            if projected.fits_after(column, width) {
                column = column.plus(projected.width());
                sink.push(projected)?;
                if atom.is_unicode_whitespace() {
                    soft_wrap = Some(SoftWrap {
                        source_end: atom.source().end(),
                        checkpoint: sink.checkpoint(),
                        column_after: column,
                    });
                }
                break;
            }

            if let Some(saved) = soft_wrap.take() {
                let boundary = VisualRowBoundary {
                    end: saved.source_end,
                    next: Some(saved.source_end),
                };
                let carry_tail = rows + 1 < row_limit.get();
                sink.finish_row(saved.checkpoint, carry_tail)?;
                rows += 1;
                if rows == row_limit.get() {
                    return Ok(ProjectedRowsExtent { rows, boundary });
                }
                column = DisplayColumn::new(
                    column
                        .get()
                        .checked_sub(saved.column_after.get())
                        .expect("soft-wrap suffix widths remain nonnegative"),
                );
            } else {
                let boundary = VisualRowBoundary {
                    end: atom.source().start(),
                    next: Some(atom.source().start()),
                };
                let checkpoint = sink.checkpoint();
                sink.finish_row(checkpoint, false)?;
                rows += 1;
                if rows == row_limit.get() {
                    return Ok(ProjectedRowsExtent { rows, boundary });
                }
                column = DisplayColumn::ZERO;
            }
        }
    }
}

#[cfg(test)]
fn reference_visual_row_boundary(
    reader: &mut DocumentReader<'_>,
    start: SourceOffset,
    width: ContentWidth,
) -> Result<VisualRowBoundary, TutError> {
    let source_end = reader.source_end();
    let mut graphemes = reader.graphemes(start)?;
    let mut column = DisplayColumn::ZERO;
    let mut consumed = false;
    let mut last_whitespace = None;

    while let Some(grapheme) = graphemes.next_grapheme()? {
        let atom = DisplayAtom::from_grapheme(grapheme);
        if atom.kind() == DisplayAtomKind::LineFeed {
            return Ok(VisualRowBoundary {
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
        return Ok(VisualRowBoundary {
            end: next,
            next: Some(next),
        });
    }

    Ok(VisualRowBoundary {
        end: source_end,
        next: None,
    })
}

#[cfg(test)]
fn previous_row_start(
    reader: &mut DocumentReader<'_>,
    start: SourceOffset,
    width: ContentWidth,
) -> Result<SourceOffset, TutError> {
    if start <= reader.source_start() {
        return Ok(reader.source_start());
    }
    let Some(probe) = reader.previous_char_start(start)? else {
        return Ok(reader.source_start());
    };
    row_start_at_or_before(reader, probe, width)
}

#[cfg(test)]
fn row_start_at_or_before(
    reader: &mut DocumentReader<'_>,
    offset: SourceOffset,
    width: ContentWidth,
) -> Result<SourceOffset, TutError> {
    let mut row_start = reader.line_start_at_or_before(offset)?;
    loop {
        let Some(next) = visual_row_boundary(reader, row_start, width)?.next else {
            return Ok(row_start);
        };
        if next > offset {
            return Ok(row_start);
        }
        row_start = next;
        if row_start == offset {
            return Ok(row_start);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::document::{Document, DocumentCache};

    struct Fixture {
        document: Document,
        cache: DocumentCache,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum OwnedProjection {
        Text(String),
        Spaces(u8),
        Replacement,
        DottedCircle(String),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct OwnedProjectedAtom {
        source: GraphemeRange,
        width: DisplayColumn,
        projection: OwnedProjection,
    }

    impl OwnedProjectedAtom {
        fn from_projected(atom: ProjectedAtom<'_>) -> Self {
            let projection = match atom.projection() {
                DisplayProjection::Text(text) => OwnedProjection::Text(text.to_owned()),
                DisplayProjection::Spaces(count) => OwnedProjection::Spaces(count),
                DisplayProjection::Replacement => OwnedProjection::Replacement,
                DisplayProjection::DottedCircle(source) => {
                    OwnedProjection::DottedCircle(source.to_owned())
                }
            };
            Self {
                source: atom.source(),
                width: atom.width(),
                projection,
            }
        }
    }

    #[derive(Default)]
    struct CollectedRows {
        atoms: Vec<OwnedProjectedAtom>,
        rows: Vec<Range<usize>>,
        row_start: usize,
    }

    impl ProjectedRowSink for CollectedRows {
        type Checkpoint = usize;

        fn checkpoint(&self) -> Self::Checkpoint {
            self.atoms.len()
        }

        fn push(&mut self, atom: ProjectedAtom<'_>) -> Result<(), TutError> {
            self.atoms.push(OwnedProjectedAtom::from_projected(atom));
            Ok(())
        }

        fn finish_row(
            &mut self,
            through: Self::Checkpoint,
            carry_tail: bool,
        ) -> Result<(), TutError> {
            if !carry_tail {
                self.atoms.truncate(through);
            }
            self.rows.push(self.row_start..through);
            self.row_start = through;
            Ok(())
        }
    }

    impl CollectedRows {
        fn owned_rows(&self) -> Vec<Vec<OwnedProjectedAtom>> {
            self.rows
                .iter()
                .map(|row| self.atoms[row.clone()].to_vec())
                .collect()
        }

        fn text_rows(&self) -> Vec<String> {
            self.rows
                .iter()
                .map(|row| {
                    let mut text = String::new();
                    for atom in &self.atoms[row.clone()] {
                        match &atom.projection {
                            OwnedProjection::Text(source) => text.push_str(source),
                            OwnedProjection::Spaces(count) => {
                                text.extend(std::iter::repeat_n(' ', usize::from(*count)));
                            }
                            OwnedProjection::Replacement => {
                                text.push_str(REPLACEMENT_CHARACTER);
                            }
                            OwnedProjection::DottedCircle(source) => {
                                text.push_str(DOTTED_CIRCLE);
                                text.push_str(source);
                            }
                        }
                    }
                    text
                })
                .collect()
        }
    }

    impl Fixture {
        fn new(text: &str) -> Self {
            Self::at(text, SourceOffset::ZERO)
        }

        fn at(text: &str, start: SourceOffset) -> Self {
            Self {
                document: Document::from_text_at(Path::new("layout.txt"), text.to_owned(), start),
                cache: DocumentCache::with_window_bytes(1),
            }
        }

        fn layout(&mut self, width: u16, height: u16) -> ViewportLayout {
            let mut slot = None;
            let reader = self.document.reader(&mut self.cache);
            rebuild_viewport_layout(
                &mut slot,
                &reader,
                ContentWidth::new(width).unwrap(),
                BodyHeight::new(height).unwrap(),
            );
            slot.unwrap()
        }
    }

    fn case(text: &str, width: u16, height: u16) -> (Fixture, ViewportLayout) {
        let mut fixture = Fixture::new(text);
        let layout = fixture.layout(width, height);
        (fixture, layout)
    }

    fn row_starts(fixture: &mut Fixture, layout: &ViewportLayout) -> Vec<SourceOffset> {
        let mut starts = vec![fixture.document.source().start()];
        let mut reader = fixture.document.reader(&mut fixture.cache);
        while let Some(next) = layout
            .next_row_start(&mut reader, *starts.last().unwrap())
            .unwrap()
        {
            starts.push(next);
        }
        starts
    }

    fn projected(fixture: &mut Fixture, layout: &ViewportLayout, row: usize) -> Vec<(String, u32)> {
        let start = row_starts(fixture, layout)[row];
        let mut output = Vec::new();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        layout
            .visit_projected_row(&mut reader, start, |atom| {
                let text = match atom.projection() {
                    DisplayProjection::Text(text) => text.to_owned(),
                    DisplayProjection::Spaces(count) => " ".repeat(usize::from(count)),
                    DisplayProjection::Replacement => REPLACEMENT_CHARACTER.to_owned(),
                    DisplayProjection::DottedCircle(source) => format!("{DOTTED_CIRCLE}{source}"),
                };
                output.push((text, atom.width().get()));
                Ok(())
            })
            .unwrap();
        output
    }

    #[test]
    fn layouts_reject_another_document_with_the_same_source_range() {
        let mut first = Fixture::new("abcd");
        let layout = first.layout(4, 1);
        let mut second = Fixture::new("wxyz");

        let error = {
            let mut reader = second.document.reader(&mut second.cache);
            layout
                .visible_extent(&mut reader, SourceOffset::ZERO)
                .unwrap_err()
        };
        assert!(matches!(
            error,
            TutError::Layout(LayoutError::DocumentMismatch)
        ));

        let mut slot = Some(layout);
        let reader = second.document.reader(&mut second.cache);
        assert!(ensure_viewport_layout(
            &mut slot,
            &reader,
            ContentWidth::new(4),
            BodyHeight::new(1)
        ));
    }

    #[test]
    fn projection_sanitizes_controls_tabs_and_zero_width_graphemes() {
        let text = "a\t\u{001b}\u{202e}\u{200b}ｶﾞ";
        let (mut fixture, layout) = case(text, 16, 2);
        assert_eq!(
            projected(&mut fixture, &layout, 0),
            vec![
                ("a".to_owned(), 1),
                ("   ".to_owned(), 3),
                ("�".to_owned(), 1),
                ("�".to_owned(), 1),
                ("◌\u{200b}".to_owned(), 1),
                ("ｶﾞ".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn render_limit_replaces_complete_graphemes_without_losing_coordinates() {
        let mut cluster = String::from("a");
        cluster.extend(std::iter::repeat_n('\u{301}', 510));
        cluster.push('\u{20dd}');
        assert_eq!(cluster.len(), MAX_RENDER_GRAPHEME_BYTES);
        assert_eq!(cluster.graphemes(true).count(), 1);

        let mut text = cluster.clone();
        text.push('z');
        let (mut fixture, layout) = case(&text, 16, 1);
        assert_eq!(projected(&mut fixture, &layout, 0)[0].0, cluster);

        text.insert(text.len() - 1, '\u{301}');
        let cluster_end = SourceOffset::from_usize(text.len() - 1);
        let atom = DisplayAtoms::new(&text[..text.len() - 1]).next().unwrap();
        assert_eq!(atom.kind(), DisplayAtomKind::Oversized);
        assert_eq!(
            atom.project(DisplayColumn::ZERO, ContentWidth::new(16).unwrap())
                .unwrap()
                .projection(),
            DisplayProjection::Replacement
        );
        let (mut fixture, layout) = case(&text, 16, 1);
        let mut rows = CollectedRows::default();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        layout
            .project_visible_rows(&mut reader, SourceOffset::ZERO, &mut rows)
            .unwrap();
        let rows = rows.owned_rows();

        assert_eq!(
            rows[0][0].source,
            GraphemeRange::new(SourceOffset::ZERO, cluster_end).unwrap()
        );
        assert_eq!(rows[0][0].projection, OwnedProjection::Replacement);
        assert_eq!(rows[0][1].source.start(), cluster_end);
        assert_eq!(rows[0][1].projection, OwnedProjection::Text("z".to_owned()));
    }

    #[test]
    fn wrapping_is_greedy_and_preserves_whitespace_and_blank_lines() {
        let text = "one two\n\nend\n";
        let (mut fixture, layout) = case(text, 4, 2);
        let starts: Vec<_> = row_starts(&mut fixture, &layout)
            .into_iter()
            .map(SourceOffset::get)
            .collect();
        assert_eq!(starts, vec![0, 4, 8, 9, 13]);
        assert_eq!(projected(&mut fixture, &layout, 0)[3].0, " ");
        assert!(projected(&mut fixture, &layout, 2).is_empty());
        assert!(projected(&mut fixture, &layout, 4).is_empty());
    }

    #[test]
    fn single_pass_wrapping_carries_only_the_provisional_suffix() {
        let mut fixture = Fixture::new("a bcdef");
        let layout = fixture.layout(4, 3);
        let mut rows = CollectedRows::default();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        let extent = layout
            .project_visible_rows(&mut reader, SourceOffset::ZERO, &mut rows)
            .unwrap();

        assert_eq!(extent, (3, SourceOffset::new(7)));
        assert_eq!(rows.text_rows(), ["a ", "bcde", "f"]);

        let layout = fixture.layout(4, 1);
        let mut rows = CollectedRows::default();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        let extent = layout
            .project_visible_rows(&mut reader, SourceOffset::ZERO, &mut rows)
            .unwrap();
        assert_eq!(extent, (1, SourceOffset::new(2)));
        assert_eq!(rows.text_rows(), ["a "]);

        let mut fixture = Fixture::new("a bc\tz");
        let layout = fixture.layout(4, 3);
        let mut rows = CollectedRows::default();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        layout
            .project_visible_rows(&mut reader, SourceOffset::ZERO, &mut rows)
            .unwrap();
        assert_eq!(rows.text_rows(), ["a ", "bc  ", "z"]);

        let start = SourceOffset::new(u64::from(u32::MAX) + 17);
        let mut fixture = Fixture::at("a\r\nb", start);
        let layout = fixture.layout(4, 1);
        let mut rows = CollectedRows::default();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        let extent = layout
            .project_visible_rows(&mut reader, start, &mut rows)
            .unwrap();
        assert_eq!(extent, (1, start.checked_add(3).unwrap()));
        assert_eq!(rows.text_rows(), ["a"]);
    }

    #[test]
    fn single_pass_rows_match_the_row_oracle_across_unicode_and_endings() {
        let samples = [
            "",
            "\n",
            "\n\n",
            "a\r\nb\rc\n",
            "a bcdef",
            "a bc\tz",
            "a\u{a0}bc def",
            "a\u{001b}\u{0085}b",
            "e\u{301}\u{200b}🙂 text\n",
        ];

        for text in samples {
            for width in 1..=8 {
                let mut fixture = Fixture::new(text);
                let layout = fixture.layout(width, 64);
                let mut starts = vec![fixture.document.source().start()];
                loop {
                    let mut reader = fixture.document.reader(&mut fixture.cache);
                    let boundary = reference_visual_row_boundary(
                        &mut reader,
                        *starts.last().unwrap(),
                        layout.width,
                    )
                    .unwrap();
                    let Some(next) = boundary.next else {
                        break;
                    };
                    starts.push(next);
                }
                let mut expected = Vec::new();
                for start in starts {
                    let mut row = Vec::new();
                    let mut reader = fixture.document.reader(&mut fixture.cache);
                    layout
                        .visit_projected_row(&mut reader, start, |atom| {
                            row.push(OwnedProjectedAtom::from_projected(atom));
                            Ok(())
                        })
                        .unwrap();
                    expected.push(row);
                }

                let mut actual = CollectedRows::default();
                let mut reader = fixture.document.reader(&mut fixture.cache);
                let source_start = reader.source_start();
                let (row_count, visible_end) = layout
                    .project_visible_rows(&mut reader, source_start, &mut actual)
                    .unwrap();

                assert_eq!(row_count, expected.len(), "{text:?}, width {width}");
                assert_eq!(visible_end, reader.source_end(), "{text:?}, width {width}");
                assert_eq!(actual.owned_rows(), expected, "{text:?}, width {width}");
            }
        }
    }

    #[test]
    fn a_tab_that_cannot_fit_on_a_fresh_row_becomes_one_cell() {
        let text = "a\t";
        let (mut fixture, layout) = case(text, 2, 2);
        assert_eq!(
            row_starts(&mut fixture, &layout),
            [SourceOffset::ZERO, SourceOffset::new(1)]
        );
        assert_eq!(
            projected(&mut fixture, &layout, 1),
            vec![("�".to_owned(), 1)]
        );
    }

    #[test]
    fn viewport_resolution_and_progress_are_clamped() {
        let (mut fixture, layout) = case("abcdefgh", 2, 2);
        let mut reader = fixture.document.reader(&mut fixture.cache);
        assert_eq!(
            layout.last_viewport_start(&mut reader).unwrap(),
            SourceOffset::new(4)
        );
        assert_eq!(
            layout
                .resolve_top(&mut reader, SourceOffset::new(7))
                .unwrap(),
            SourceOffset::new(4)
        );
        assert_eq!(
            layout
                .move_row_start(&mut reader, SourceOffset::ZERO, true, 1)
                .unwrap(),
            SourceOffset::new(2)
        );
        assert_eq!(
            layout
                .move_row_start(&mut reader, SourceOffset::new(2), false, 1)
                .unwrap(),
            SourceOffset::ZERO
        );
        assert_eq!(
            layout
                .move_row_start(&mut reader, SourceOffset::ZERO, true, usize::MAX)
                .unwrap(),
            SourceOffset::new(4)
        );
        assert_eq!(
            progress_percent(
                SourceOffset::ZERO,
                SourceOffset::new(8),
                SourceOffset::new(4)
            ),
            50
        );
        assert_eq!(
            progress_percent(SourceOffset::ZERO, SourceOffset::ZERO, SourceOffset::ZERO),
            100
        );
    }

    #[test]
    fn raw_line_endings_keep_large_source_offsets() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 11);
        let mut fixture = Fixture::at("a\r\nb\rc\n", start);
        let layout = fixture.layout(8, 2);

        assert_eq!(
            row_starts(&mut fixture, &layout),
            vec![
                start,
                start.checked_add(3).unwrap(),
                start.checked_add(5).unwrap(),
                start.checked_add(7).unwrap(),
            ]
        );
        assert_eq!(
            progress_percent(
                start,
                start.checked_add(7).unwrap(),
                start.checked_add(3).unwrap()
            ),
            42
        );
        let mut atoms = Vec::new();
        let mut reader = fixture.document.reader(&mut fixture.cache);
        layout
            .visit_projected_row(&mut reader, start, |atom| {
                let text = match atom.projection() {
                    DisplayProjection::Text(text) => text.to_owned(),
                    DisplayProjection::Spaces(count) => " ".repeat(usize::from(count)),
                    DisplayProjection::Replacement => REPLACEMENT_CHARACTER.to_owned(),
                    DisplayProjection::DottedCircle(source) => format!("{DOTTED_CIRCLE}{source}"),
                };
                atoms.push(text);
                Ok(())
            })
            .unwrap();
        assert_eq!(atoms, vec!["a"]);
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
                let (mut fixture, layout) = case(text, width, 2);
                let starts = row_starts(&mut fixture, &layout);
                let source_start = fixture.document.source().start();
                let source_end = fixture.document.source().end();
                assert_eq!(starts.first(), Some(&source_start));
                assert!(starts.windows(2).all(|pair| pair[0] < pair[1]));
                assert!(starts.iter().all(|offset| {
                    usize::try_from(offset.get() - source_start.get())
                        .ok()
                        .is_some_and(|relative| text.is_char_boundary(relative))
                }));
                assert!(starts.len() <= text.graphemes(true).count() + 1);

                for start in &starts {
                    let mut reader = fixture.document.reader(&mut fixture.cache);
                    let range = layout.row_range(&mut reader, *start).unwrap();
                    let relative_start =
                        usize::try_from(range.start.get() - source_start.get()).unwrap();
                    let relative_end =
                        usize::try_from(range.end.get() - source_start.get()).unwrap();
                    assert!(text.is_char_boundary(relative_start));
                    assert!(text.is_char_boundary(relative_end));
                    let mut used = 0;
                    layout
                        .visit_projected_row(&mut reader, *start, |atom| {
                            used += atom.width().get();
                            Ok(())
                        })
                        .unwrap();
                    assert!(used <= u32::from(width));
                }

                let mut reader = fixture.document.reader(&mut fixture.cache);
                assert_eq!(
                    layout
                        .row_range(&mut reader, *starts.last().unwrap())
                        .unwrap()
                        .end,
                    source_end
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
                let (mut fixture, layout) = case(text, width, 1);
                let starts = row_starts(&mut fixture, &layout);

                for (index, start) in starts.iter().copied().enumerate() {
                    let mut reader = fixture.document.reader(&mut fixture.cache);
                    assert_eq!(
                        layout.row_start_at_or_before(&mut reader, start).unwrap(),
                        start
                    );
                    let expected_up = starts[index.saturating_sub(1)];
                    assert_eq!(
                        layout.move_row_start(&mut reader, start, false, 1).unwrap(),
                        expected_up
                    );
                    let expected_down = starts.get(index + 1).copied().unwrap_or(start);
                    assert_eq!(
                        layout.move_row_start(&mut reader, start, true, 1).unwrap(),
                        expected_down
                    );
                }

                let source_start = fixture.document.source().start();
                for relative in text
                    .char_indices()
                    .map(|(relative, _)| relative)
                    .chain(std::iter::once(text.len()))
                {
                    let offset = source_start.checked_add(relative).unwrap();
                    let expected = starts
                        .iter()
                        .copied()
                        .take_while(|start| *start <= offset)
                        .last()
                        .unwrap();
                    let mut reader = fixture.document.reader(&mut fixture.cache);
                    assert_eq!(
                        layout.row_start_at_or_before(&mut reader, offset).unwrap(),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn last_viewport_matches_forward_row_partitioning() {
        let samples = [
            "",
            "\n",
            "\n\n",
            "a\n",
            "one two three four",
            "a\rb\r\nc\n",
            "e\u{301}🙂\u{200b}\ttext",
        ];

        for text in samples {
            for width in 1..=8 {
                for height in 1..=8 {
                    let (mut fixture, layout) = case(text, width, height);
                    let starts = row_starts(&mut fixture, &layout);
                    let expected = starts[starts.len().saturating_sub(usize::from(height))];
                    let mut reader = fixture.document.reader(&mut fixture.cache);

                    assert_eq!(
                        layout.last_viewport_start(&mut reader).unwrap(),
                        expected,
                        "text={text:?}, width={width}, height={height}"
                    );
                }
            }
        }
    }
}
