#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// Sentinel value meaning "no source location available".
    /// Used for error codes generated without a known source position.
    pub const UNKNOWN: Span = Span::new(0, 0);

    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

