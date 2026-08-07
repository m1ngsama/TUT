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

    pub(super) fn relative_offset(self, offset: SourceOffset) -> Option<usize> {
        if offset < self.start || offset > self.end {
            return None;
        }
        usize::try_from(offset.get() - self.start.get()).ok()
    }

    pub(super) fn slice(self, range: Range<SourceOffset>) -> Option<&'a str> {
        let start = self.relative_offset(range.start)?;
        let end = self.relative_offset(range.end)?;
        self.text.get(start..end)
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
}
