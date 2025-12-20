use frontend::span::Span;

pub trait Output {
    fn write(&mut self, bytes: &[u8]);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TcError {
    pub code: u32,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksMode {
    Off,
    Contracts,
    All,
}
