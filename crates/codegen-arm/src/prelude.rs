use codegen_core::{AsmMode, CodegenError};
use semantics::typecheck;
use semantics::types::WordSig;

use crate::ophelpers::{slice_span, write_sym_label};
use crate::ArmThumbBackend;

impl<'a> ArmThumbBackend<'a> {
    pub fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"\t.syntax unified\n");
                self.out.write(b"\t.thumb\n");
                self.out.write(b"\t.global __lang_start\n");
                self.out.write(b"\t.type __lang_start, %function\n");
                self.out.write(b"__lang_start:\n");
                self.out.write(b"\tldr r4, =__lang_ds_base\n");
                self.out.write(b"\tldr r5, =__lang_ds_limit\n");

                let main_decl = find_word_decl(self.module, self.src, b"main")
                    .ok_or(CodegenError::MissingEntryPoint { name: b"main" })?;
                let main_sig = main_decl
                    .sig
                    .and_then(|s| typecheck::parse_word_sig(self.src, s).ok())
                    .unwrap_or(WordSig::empty());

                self.out.write(b"\tbl ");
                write_sym_label(self.out, b"main");
                self.out.write(b"\n");

                if main_sig.out_len > 0 {
                    self.out.write(b"\tsubs r4, r4, #8\n");
                    self.out.write(b"\tldr r0, [r4]\n");
                } else {
                    self.out.write(b"\tmovs r0, #0\n");
                }
                self.out.write(b"\tb __lang_trap\n");
                self.out.write(b"\n");
                self.out.write(b"\t.global __lang_trap\n");
                self.out.write(b"\t.type __lang_trap, %function\n");
                self.out.write(b"__lang_trap:\n");
                self.out.write(b"\t.global __stack_overflow\n");
                self.out.write(b"\t.type __stack_overflow, %function\n");
                self.out.write(b"__stack_overflow:\n");
                self.out.write(b"\tb __lang_trap\n");
                Ok(())
            }
            AsmMode::Object => {
                self.out.write(b"\t.syntax unified\n");
                self.out.write(b"\t.thumb\n");
                self.out.write(b"\t.section .text\n");
                self.out.write(b"\t.global __lang_trap\n");
                self.out.write(b"\t.global __stack_overflow\n");
                self.out.write(b"\t.extern __lang_trap\n");
                self.out.write(b"\t.extern __stack_overflow\n");
                Ok(())
            }
        }
    }

    pub fn emit_extern_word(&mut self, name: &[u8]) {
        if self.mode != AsmMode::Object {
            return;
        }
        self.out.write(b"\t.thumb_func\n");
        self.out.write(b"\t.global ");
        write_sym_label(self.out, name);
        self.out.write(b"\n");
        self.out.write(b"\t.type ");
        write_sym_label(self.out, name);
        self.out.write(b", %function\n");

        let idx = self.mi_import_count;
        if idx < self.mi_imports.len() {
            if let Some(n) = ir::Atom::new(name) {
                self.mi_imports[idx] = crate::ModInfoImport { name: n };
                self.mi_import_count = idx + 1;
            }
        }
    }
}

fn find_word_decl<'a>(
    module: &'a frontend::parse::ModuleAst,
    src: &'a [u8],
    name: &[u8],
) -> Option<&'a frontend::parse::DeclAst> {
    for d in module.decls.iter() {
        if d.kind != frontend::parse::DeclKind::Word {
            continue;
        }
        let dname = slice_span(src, d.name);
        if dname == name {
            return Some(d);
        }
    }
    None
}
