use codegen_core::{AsmMode, CodegenError};
use semantics::types::WordSig;
use semantics::typecheck;

use crate::ophelpers::{emit_stack_overflow, write_label};
use crate::util::find_word_decl;
use crate::X86_64HostedBackend;

impl<'a> X86_64HostedBackend<'a> {
    pub fn emit_prelude(&mut self) -> Result<(), CodegenError> {
        match self.mode {
            AsmMode::Executable => {
                self.out.write(b"format ELF64 executable\n");
                self.out.write(b"entry __lang_start\n\n");

                self.out.write(b"segment readable executable\n");
                self.out.write(b"__lang_start:\n");
                self.out.write(b"  mov r15, __lang_ds_base\n");
                self.out.write(b"  mov r14, __lang_ds_limit\n");

                let main_decl = find_word_decl(self.module, self.src, b"main").ok_or(CodegenError::MissingEntryPoint { name: b"main" })?;
                let main_sig = main_decl
                    .sig
                    .and_then(|s| typecheck::parse_word_sig(self.src, s).ok())
                    .unwrap_or(WordSig::empty());

                self.out.write(b"  call ");
                write_label(self.out, b"main");
                self.out.write(b"\n");

                if main_sig.out_len > 0 {
                    self.out.write(b"  sub r15, 8\n");
                    self.out.write(b"  mov rdi, [r15]\n");
                    self.out.write(b"  and rdi, 0xff\n");
                } else {
                    self.out.write(b"  xor rdi, rdi\n");
                }
                self.out.write(b"  mov rax, 60\n");
                self.out.write(b"  syscall\n\n");

                self.out.write(b"__lang_trap:\n");
                self.out.write(b"  mov rax, 60\n");
                self.out.write(b"  syscall\n\n");
                if self.debug_trap_loc {
                    self.out.write(b"__lang_trap_loc:\n");
                    self.out.write(b"  mov rax, 60\n");
                    self.out.write(b"  syscall\n\n");
                }
                emit_stack_overflow(self.out);
                Ok(())
            }
            AsmMode::Object => {
                self.out.write(b"format ELF64\n\n");
                self.out.write(b"section '.text' executable\n");
                self.out.write(b"extrn __lang_trap\n");
                self.out.write(b"extrn __lang_trap_loc\n");
                self.out.write(b"extrn __stack_overflow\n");
                self.out.write(b"extrn __mmio_mem\n");
                self.out.write(b"extrn __chan_next\n");
                self.out.write(b"extrn __chan_inuse\n");
                self.out.write(b"extrn __chan_head\n");
                self.out.write(b"extrn __chan_tail\n");
                self.out.write(b"extrn __chan_buf\n");
                self.out.write(b"extrn __chan_wait_recv_head\n");
                self.out.write(b"extrn __chan_wait_recv_tail\n");
                self.out.write(b"extrn __chan_wait_recv_buf\n");
                self.out.write(b"extrn __chan_wait_send_head\n");
                self.out.write(b"extrn __chan_wait_send_tail\n");
                self.out.write(b"extrn __chan_wait_send_buf\n");
                self.out.write(b"extrn __task_current\n");
                self.out.write(b"extrn __task_state\n");
                self.out.write(b"extrn __task_g_head\n");
                self.out.write(b"extrn __task_g_tail\n");
                self.out.write(b"extrn __task_g_buf\n");
                self.out.write(b"extrn __region_next\n");
                self.out.write(b"extrn __region_base\n");
                self.out.write(b"extrn __region_size\n");
                self.out.write(b"extrn __region_off\n");
                self.out.write(b"extrn __task_spawn\n");
                self.out.write(b"extrn __task_join\n");
                self.out.write(b"extrn __task_yield\n");
                self.out.write(b"extrn __task_sleep_ms\n");
                self.out.write(b"extrn __task_sleep_us\n");
                self.out.write(b"\n");
                Ok(())
            }
        }
    }

    pub fn emit_extern_word(&mut self, name: &[u8]) {
        if self.mode != AsmMode::Object {
            return;
        }
        self.out.write(b"extrn ");
        write_label(self.out, name);
        self.out.write(b"\n");
    }
}
