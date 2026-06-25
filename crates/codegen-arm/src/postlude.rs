use crate::ophelpers::{slice_span, write_res_label, write_u32};
use crate::ArmThumbBackend;
use codegen_core::strings::decode_string_bytes;
use codegen_core::{AsmMode, CodegenError};
use frontend::parse::DeclKind;

fn resource_decl_names<'a>(
    module: &'a frontend::parse::ModuleAst,
    src: &'a [u8],
) -> impl Iterator<Item = &'a [u8]> {
    module.decls.iter().filter_map(move |d| {
        if d.kind != DeclKind::Resource {
            return None;
        }
        Some(slice_span(src, d.name))
    })
}

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
                let module_name = slice_span(self.src, self.module.name);
                for name in resource_decl_names(self.module, self.src) {
                    self.out.write(b"\t.comm ");
                    write_res_label(self.out, module_name, name);
                    self.out.write(b",8,4\n");
                }
                ArmThumbBackend::emit_modinfo_section(self)?;
                Ok(())
            }
        }
    }
}
