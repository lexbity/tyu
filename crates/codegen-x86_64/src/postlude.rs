use codegen_core::{AsmMode, CodegenError};

use crate::ophelpers::write_u32;
use crate::task;
use crate::X86_64HostedBackend;
use codegen_core::strings::decode_string_bytes;

impl<'a> X86_64HostedBackend<'a> {
    /// Emit the interned string table into the current section.
    /// Called from both Executable and Object mode postludes.
    fn emit_string_table(&mut self) -> Result<(), CodegenError> {
        for i in 0..self.str_len {
            let id = self.str_ids[i];
            let span = self.str_spans[i];
            let bytes =
                decode_string_bytes(self.src, span).ok_or(CodegenError::MalformedStringLiteral)?;

            self.out.write(b"\n__lang_str_");
            write_u32(self.out, id);
            self.out.write(b":\n");
            self.out.write(b"  dq __lang_str_");
            write_u32(self.out, id);
            self.out.write(b"_bytes\n");
            self.out.write(b"  dq ");
            write_u32(self.out, bytes.len() as u32);
            self.out.write(b"\n");

            self.out.write(b"__lang_str_");
            write_u32(self.out, id);
            self.out.write(b"_bytes db ");
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
        Ok(())
    }

    pub fn emit_postlude(&mut self) -> Result<(), CodegenError> {
        match self.mode {
            AsmMode::Executable => {
                if self.str_len > 0 {
                    self.out.write(b"\nsegment readable\n");
                    self.emit_string_table()?;
                }

                if self.uses_tasks {
                    task::emit_task_runtime(self.out);
                }

                self.out.write(b"\nsegment readable writeable\n");
                if self.uses_channels {
                    self.out.write(b"__chan_next dq 0\n");
                    self.out.write(b"__chan_inuse rq 16\n");
                    self.out.write(b"__chan_head rq 16\n");
                    self.out.write(b"__chan_tail rq 16\n");
                    self.out.write(b"__chan_buf rq ");
                    write_u32(self.out, 16 * 64);
                    self.out.write(b"\n");
                    self.out.write(b"__chan_wait_recv_head rq 16\n");
                    self.out.write(b"__chan_wait_recv_tail rq 16\n");
                    self.out.write(b"__chan_wait_recv_buf rq ");
                    write_u32(self.out, 16 * 8);
                    self.out.write(b"\n");
                    self.out.write(b"__chan_wait_send_head rq 16\n");
                    self.out.write(b"__chan_wait_send_tail rq 16\n");
                    self.out.write(b"__chan_wait_send_buf rq ");
                    write_u32(self.out, 16 * 8);
                    self.out.write(b"\n");
                }
                if self.uses_regions {
                    self.out.write(b"__region_next dq 0\n");
                    self.out.write(b"__region_base rq 16\n");
                    self.out.write(b"__region_size rq 16\n");
                    self.out.write(b"__region_off rq 16\n");
                }
                if self.uses_tasks {
                    self.out.write(b"__task_current dq 0\n");
                    self.out.write(b"__task_worker dq 0\n");
                    self.out.write(b"__task_state rq 16\n");
                    self.out.write(b"__task_rsp rq 16\n");
                    self.out.write(b"__task_r15 rq 16\n");
                    self.out.write(b"__task_r14 rq 16\n");
                    self.out.write(b"__task_entry rq 16\n");
                    self.out.write(b"__task_w_head rq 4\n");
                    self.out.write(b"__task_w_tail rq 4\n");
                    self.out.write(b"__task_w_buf rq ");
                    write_u32(self.out, 4 * 8);
                    self.out.write(b"\n");
                    self.out.write(b"__task_g_head dq 0\n");
                    self.out.write(b"__task_g_tail dq 0\n");
                    self.out.write(b"__task_g_buf rq 16\n");
                    self.out.write(b"__task_ds_mem rb 1048576\n");
                    self.out.write(b"__task_cs_mem rb 1048576\n");
                }
                if self.uses_mmio {
                    self.out.write(b"__mmio_mem rb 65536\n");
                }
                self.out.write(b"__lang_ds_base rb 65536\n");
                self.out.write(b"__lang_ds_limit:\n");
                self.out.write(b"__lang_ds_high dq 0\n");
                Ok(())
            }
            AsmMode::Object => {
                if self.str_len > 0 {
                    self.out.write(b"\nsection '.rodata'\n");
                    self.emit_string_table()?;
                }
                X86_64HostedBackend::emit_modinfo_section(self)?;
                X86_64HostedBackend::emit_debugsec_section(self)?;
                Ok(())
            }
        }
    }
}
