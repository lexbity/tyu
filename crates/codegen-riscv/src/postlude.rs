use crate::ophelpers::{slice_span, write_res_label, write_u32};
use crate::RiscVBackend;
use frontend::parse::DeclKind;
use codegen_core::strings::decode_string_bytes;
use codegen_core::{AsmMode, CodegenError};

impl<'a> RiscVBackend<'a> {
    pub fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        if self.str_len > 0 {
            self.out.write(b"\n\t.section .rodata\n");
            for i in 0..self.str_len {
                let id = self.str_ids[i];
                let span = self.str_spans[i];
                let bytes = decode_string_bytes(self.src, span)
                    .ok_or(CodegenError::MalformedStringLiteral)?;
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
                // Resource storage globals (`.comm`), one per declared
                // resource — the AddrOf{Runtime} lowering references these
                // (mirrors the ARM backend's postlude).
                let module_name = slice_span(self.src, self.module.name);
                for d in self.module.decls.iter() {
                    if d.kind != DeclKind::Resource {
                        continue;
                    }
                    self.out.write(b"\t.comm ");
                    write_res_label(self.out, module_name, slice_span(self.src, d.name));
                    self.out.write(b",8,4\n");
                }
                RiscVBackend::emit_modinfo_section(self)?;
                Ok(())
            }
        }
    }
}
