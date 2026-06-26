; Core runtime for the x86_64-unknown-linux-gnu (Linux-hosted) target.
;
; The per-triple runtime resolver (build.rs::assemble_runtime) looks for
; runtime/<triple>/runtime.asm.  The canonical hosted runtime lives at the
; flat path runtime/linux-x86_64-hosted.asm, where it is also consumed
; directly by the langc tooling tests and the linux-x86_64-hosted platform
; manifest.  Bridge the two without duplicating the source: include resolves
; relative to this file's directory.
include '../linux-x86_64-hosted.asm'
