use std::num::NonZeroUsize;

#[cfg(test)]
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub(super) struct SourceOffset(u64);

impl SourceOffset {
    pub(super) const ZERO: Self = Self(0);

    pub(super) const fn new(value: u64) -> Self {
        Self(value)
    }

    pub(super) const fn get(self) -> u64 {
        self.0
    }

    pub(super) fn from_usize(value: usize) -> Self {
        Self::new(u64::try_from(value).expect("usize fits in source coordinates"))
    }

    pub(super) fn checked_add(self, bytes: usize) -> Option<Self> {
        let bytes = u64::try_from(bytes).ok()?;
        self.0.checked_add(bytes).map(Self::new)
    }

    pub(super) fn checked_sub(self, bytes: usize) -> Option<Self> {
        let bytes = u64::try_from(bytes).ok()?;
        self.0.checked_sub(bytes).map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowRequest {
    start: SourceOffset,
    target_bytes: NonZeroUsize,
}

impl WindowRequest {
    pub(super) const fn new(start: SourceOffset, target_bytes: NonZeroUsize) -> Self {
        Self {
            start,
            target_bytes,
        }
    }

    pub(super) const fn start(self) -> SourceOffset {
        self.start
    }

    pub(super) const fn target_bytes(self) -> usize {
        self.target_bytes.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BackwardWindowRequest {
    end: SourceOffset,
    target_bytes: NonZeroUsize,
}

impl BackwardWindowRequest {
    pub(super) const fn new(end: SourceOffset, target_bytes: NonZeroUsize) -> Self {
        Self { end, target_bytes }
    }

    pub(super) const fn end(self) -> SourceOffset {
        self.end
    }

    pub(super) const fn target_bytes(self) -> usize {
        self.target_bytes.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SourceText<'a> {
    text: &'a str,
    start: SourceOffset,
    end: SourceOffset,
}

impl<'a> SourceText<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        Self::with_start(text, SourceOffset::ZERO).expect("usize fits in source coordinates")
    }

    pub(super) fn with_start(text: &'a str, start: SourceOffset) -> Option<Self> {
        Some(Self {
            text,
            start,
            end: start.checked_add(text.len())?,
        })
    }

    pub(super) const fn as_str(self) -> &'a str {
        self.text
    }

    pub(super) const fn start(self) -> SourceOffset {
        self.start
    }

    pub(super) const fn end(self) -> SourceOffset {
        self.end
    }

    pub(super) const fn len_bytes(self) -> usize {
        self.text.len()
    }

    #[cfg(test)]
    pub(super) fn relative_offset(self, offset: SourceOffset) -> Option<usize> {
        if offset < self.start || offset > self.end {
            return None;
        }
        usize::try_from(offset.get() - self.start.get()).ok()
    }

    #[cfg(test)]
    pub(super) fn slice(self, range: Range<SourceOffset>) -> Option<&'a str> {
        let start = self.relative_offset(range.start)?;
        let end = self.relative_offset(range.end)?;
        self.text.get(start..end)
    }

    #[cfg(test)]
    pub(super) fn window(self, request: WindowRequest) -> Option<Self> {
        let start = self.relative_offset(request.start())?;
        if !self.text.is_char_boundary(start) {
            return None;
        }

        let mut end = start
            .saturating_add(request.target_bytes())
            .min(self.text.len());
        while end < self.text.len() && !self.text.is_char_boundary(end) {
            end += 1;
        }
        Self::with_start(self.text.get(start..end)?, request.start())
    }

    #[cfg(test)]
    pub(super) fn window_ending_at(self, request: BackwardWindowRequest) -> Option<Self> {
        let end = self.relative_offset(request.end())?;
        if end == 0 || !self.text.is_char_boundary(end) {
            return None;
        }

        let mut start = end.saturating_sub(request.target_bytes());
        while start > 0 && !self.text.is_char_boundary(start) {
            start -= 1;
        }
        let source_start = self.start.checked_add(start)?;
        Self::with_start(self.text.get(start..end)?, source_start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_text_round_trips_offsets_above_u32() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 17);
        let source = SourceText::with_start("aé", start).unwrap();
        let end = start.checked_add("aé".len()).unwrap();

        assert_eq!(source.start()..source.end(), start..end);
        assert_eq!(source.relative_offset(end), Some("aé".len()));
        assert_eq!(source.slice(start..end), Some("aé"));
    }

    #[test]
    fn source_text_rejects_out_of_bounds_and_non_utf8_boundaries() {
        let source = SourceText::with_start("é", SourceOffset::new(10)).unwrap();

        assert_eq!(source.relative_offset(SourceOffset::new(9)), None);
        assert_eq!(source.relative_offset(SourceOffset::new(13)), None);
        assert_eq!(
            source.slice(SourceOffset::new(10)..SourceOffset::new(11)),
            None
        );
    }

    #[test]
    fn windows_make_progress_without_splitting_utf8() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 9);
        let source = SourceText::with_start("a🙂éz", start).unwrap();
        let one = NonZeroUsize::new(1).unwrap();

        let first = source.window(WindowRequest::new(start, one)).unwrap();
        let second = source.window(WindowRequest::new(first.end(), one)).unwrap();
        let third = source
            .window(WindowRequest::new(second.end(), one))
            .unwrap();

        assert_eq!(first.as_str(), "a");
        assert_eq!(second.as_str(), "🙂");
        assert_eq!(third.as_str(), "é");
        assert!(
            source
                .window(WindowRequest::new(start.checked_add(2).unwrap(), one))
                .is_none()
        );
    }

    #[test]
    fn backward_windows_extend_to_utf8_boundaries() {
        let start = SourceOffset::new(u64::from(u32::MAX) + 9);
        let source = SourceText::with_start("a🙂éz", start).unwrap();
        let one = NonZeroUsize::new(1).unwrap();
        let end = start.checked_add("a🙂é".len()).unwrap();

        let third = source
            .window_ending_at(BackwardWindowRequest::new(end, one))
            .unwrap();
        let second = source
            .window_ending_at(BackwardWindowRequest::new(third.start(), one))
            .unwrap();
        let first = source
            .window_ending_at(BackwardWindowRequest::new(second.start(), one))
            .unwrap();

        assert_eq!(third.as_str(), "é");
        assert_eq!(second.as_str(), "🙂");
        assert_eq!(first.as_str(), "a");
        assert!(
            source
                .window_ending_at(BackwardWindowRequest::new(
                    start.checked_add(2).unwrap(),
                    one
                ))
                .is_none()
        );
    }
}
