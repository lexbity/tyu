use codegen_core::{AsmMode, CodegenError};
use codegen_core::strings::decode_string_bytes;
use crate::ophelpers::write_u32;
use crate::ArmThumbBackend;

impl<'a> ArmThumbBackend<'a> {
    pub fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        if self.str_len > 0 {
            self.out.write(b"\n\t.section .rodata\n");
            for i in 0..self.str_len {
                let id = self.str_ids[i];
                let span = self.str_spans[i];
                let bytes = decode_string_bytes(self.src, span)
                    .ok_or(CodegenError::MalformedStringLiteral)?;
                // Emit [u64 len][u8 bytes...] for testio.write-str compat.
                self.out.write(b"__lang_str_");
                write_u32(self.out, id);
                self.out.write(b":\n\t.quad ");
                write_u32(self.out, bytes.len() as u32);
                self.out.write(b", 0\n\t.byte ");
                for (j, b) in bytes.iter().enumerate() {
                    if j != 0 {
                        self.out.write(b",");
                    }
                    write_u32(self.out, *b as u32);
                }
                if bytes.is_empty() {
                    self.out.write(b"0");
                }
                self.out.write(b"\n");
            }
        }
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"\n\t.section .bss\n");
                Ok(())
            }
            AsmMode::Object => {
                ArmThumbBackend::emit_modinfo_section(self)?;
                Ok(())
            }
        }
    }
}
