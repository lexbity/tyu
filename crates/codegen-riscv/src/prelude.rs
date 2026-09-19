use crate::ophelpers::write_sym_label;
use crate::RiscVBackend;
use codegen_core::{AsmMode, CodegenError};

impl<'a> RiscVBackend<'a> {
    pub fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"\t.section .text\n");
                self.out.write(b"\t.globl __lang_start\n");
                self.out.write(b"\t.type __lang_start, @function\n");
                self.out.write(b"__lang_start:\n");
                self.out.write(b"\tli s2, 0\n"); // DS base placeholder
                self.out.write(b"\tli s3, 0\n"); // DS limit placeholder
                self.out.write(b"\tli ra, 0\n");
                self.out.write(b"\tjal ");
                write_sym_label(self.out, b"main");
                self.out.write(b"\n");
                self.out.write(b"\taddi s2, s2, -8\n");
                self.out.write(b"\tlw a0, 0(s2)\n");
                self.out.write(b"\tj __lang_trap\n");
                self.out.write(b"\n");
                self.out.write(b"\t.globl __lang_trap\n");
                self.out.write(b"\t.type __lang_trap, @function\n");
                self.out.write(b"__lang_trap:\n");
                self.out.write(b"\t.globl __stack_overflow\n");
                self.out.write(b"\t.type __stack_overflow, @function\n");
                self.out.write(b"__stack_overflow:\n");
                self.out.write(b"\tj __lang_trap\n");
                Ok(())
            }
            AsmMode::Object => {
                self.out.write(b"\t.section .text\n");
                self.out.write(b"\t.globl __lang_trap\n");
                self.out.write(b"\t.globl __stack_overflow\n");
                self.out.write(b"\t.extern __lang_trap\n");
                self.out.write(b"\t.extern __stack_overflow\n");
                self.out.write(b"\t.extern __lang_stack_limit\n");
                // P6: window-base literal sites reference these externs.
                for i in 0..self.mmio_window_count {
                    let w = &self.mmio_windows[i];
                    if w.reloc_isa.is_some() {
                        self.out.write(b"\t.extern __lang_window_");
                        crate::ophelpers::write_u32(self.out, w.id as u32);
                        self.out.write(b"_base\n");
                    }
                }
                Ok(())
            }
        }
    }

    pub fn emit_extern_word(&mut self, name: &[u8]) -> Result<(), CodegenError> {
        if self.mode != AsmMode::Object {
            return Ok(());
        }
        self.out.write(b"\t.globl ");
        write_sym_label(self.out, name);
        self.out.write(b"\n\t.type ");
        write_sym_label(self.out, name);
        self.out.write(b", @function\n");
        let idx = self.mi_import_count;
        if idx < self.mi_imports.len() {
            if let Some(n) = ir::Atom::new(name) {
                self.mi_imports[idx] = crate::ModInfoImport { name: n };
                self.mi_import_count = idx + 1;
            }
            Ok(())
        } else {
            Err(CodegenError::ModInfoTooLarge)
        }
    }
}
