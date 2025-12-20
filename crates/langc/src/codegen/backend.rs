use ir as lir;

pub trait CodegenBackend {
    fn emit_prelude(&mut self) -> Result<(), u32>;
    fn emit_word(&mut self, w: &lir::Word) -> Result<(), u32>;
    fn emit_postlude(&mut self) -> Result<(), u32>;
}
