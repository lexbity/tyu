use codegen_core::{AsmMode, CodegenError};
use crate::ArmThumbBackend;

impl<'a> ArmThumbBackend<'a> {
    pub fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"\n\t.section .rodata\n");
                self.out.write(b"\t.section .bss\n");
                Ok(())
            }
            AsmMode::Object => {
                ArmThumbBackend::emit_modinfo_section(self)?;
                Ok(())
            }
        }
    }
}
