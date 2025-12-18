use frontend::span::Span;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TypeAtom {
    len: u8,
    bytes: [u8; 32],
}

impl TypeAtom {
    pub const fn new(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 32 {
            return None;
        }
        let mut out = [0u8; 32];
        let mut i = 0usize;
        while i < bytes.len() {
            out[i] = bytes[i];
            i += 1;
        }
        Some(Self {
            len: bytes.len() as u8,
            bytes: out,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordSig {
    pub in_len: u8,
    pub out_len: u8,
    pub inputs: [TypeAtom; 8],
    pub outputs: [TypeAtom; 8],
}

impl WordSig {
    pub const fn empty() -> Self {
        const Z: TypeAtom = TypeAtom {
            len: 0,
            bytes: [0u8; 32],
        };
        Self {
            in_len: 0,
            out_len: 0,
            inputs: [Z; 8],
            outputs: [Z; 8],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WordEntry {
    pub name: TypeAtom,
    pub sig: WordSig,
    pub may_suspend: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SigParseError {
    pub code: u32,
    pub span: Span,
}
