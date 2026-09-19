use codegen_core::{CodegenError, MmioApertureKind};
use frontend::span::Span;
use ir as lir;

use crate::util::{mask_for_bits, write_u32, write_u64_hex};
use crate::X86_64HostedBackend;

/// The size of the emulated MMIO aperture (`__mmio_mem`), sourced from the
/// backend's descriptor-driven aperture table (P3, D-7). The former hardcoded
/// 64 KiB constant is gone. This is the single source for BOTH the bounds
/// check and the Executable-mode BSS reservation — a reservation smaller
/// than the checked bound would admit MMIO past the array into adjacent
/// `.bss` (platform-layer spec, finding F1).
pub(crate) fn emulated_aperture_size(gen: &X86_64HostedBackend<'_>) -> Result<u32, CodegenError> {
    for i in 0..gen.mmio_aperture_count {
        if gen.mmio_apertures[i].kind == MmioApertureKind::Emulated {
            return Ok(gen.mmio_apertures[i].size);
        }
    }
    Err(CodegenError::NoMmioAperture)
}

pub fn emit_mmio_bounds_check(
    gen: &mut X86_64HostedBackend<'_>,
    width: u32,
    span: Span,
) -> Result<(), CodegenError> {
    let size = emulated_aperture_size(gen)?;
    let ok = gen.fresh_label();
    let max = size.saturating_sub(width);
    gen.out.write(b"  cmp rax, ");
    write_u32(gen.out, max);
    gen.out.write(b"\n");
    gen.out.write(b"  jbe .mmio_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b"\n");
    gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
    gen.out.write(b".mmio_ok_");
    write_u32(gen.out, ok);
    gen.out.write(b":\n");
    Ok(())
}

pub fn emit_mmio_load(
    gen: &mut X86_64HostedBackend<'_>,
    width: u32,
    signed: bool,
    span: Span,
) -> Result<(), CodegenError> {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, width, span)?;
    match (width, signed) {
        (1, true) => gen.out.write(b"  movsx rax, byte [__mmio_mem + rax]\n"),
        (1, false) => gen.out.write(b"  movzx rax, byte [__mmio_mem + rax]\n"),
        (2, true) => gen.out.write(b"  movsx rax, word [__mmio_mem + rax]\n"),
        (2, false) => gen.out.write(b"  movzx rax, word [__mmio_mem + rax]\n"),
        (4, true) => gen.out.write(b"  movsxd rax, dword [__mmio_mem + rax]\n"),
        (4, false) => gen.out.write(b"  mov eax, dword [__mmio_mem + rax]\n"),
        (8, _) => gen.out.write(b"  mov rax, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return Ok(());
        }
    }
    gen.out.write(b"  mov [r15], rax\n");
    gen.out.write(b"  add r15, 8\n");
    Ok(())
}

pub fn emit_mmio_store(
    gen: &mut X86_64HostedBackend<'_>,
    width: u32,
    write_kind: lir::WriteKind,
    span: Span,
) -> Result<(), CodegenError> {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rcx, [r15]\n"); // value to store
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n"); // address
    emit_mmio_bounds_check(gen, width, span)?;

    match write_kind {
        lir::WriteKind::Plain => {
            // Plain write: mov [mem+rax], value
            match width {
                1 => gen.out.write(b"  mov byte [__mmio_mem + rax], cl\n"),
                2 => gen.out.write(b"  mov word [__mmio_mem + rax], cx\n"),
                4 => gen.out.write(b"  mov dword [__mmio_mem + rax], ecx\n"),
                8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rcx\n"),
                _ => gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span),
            }
        }
        lir::WriteKind::W1c => {
            // Write-1-to-clear: *reg = *reg & ~value
            match width {
                1 => gen.out.write(b"  movzx rdx, byte [__mmio_mem + rax]\n"),
                2 => gen.out.write(b"  movzx rdx, word [__mmio_mem + rax]\n"),
                4 => gen.out.write(b"  mov edx, dword [__mmio_mem + rax]\n"),
                8 => gen.out.write(b"  mov rdx, qword [__mmio_mem + rax]\n"),
                _ => {
                    gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
                    return Ok(());
                }
            }
            gen.out.write(b"  not rcx\n");
            gen.out.write(b"  and rdx, rcx\n");
            match width {
                1 => gen.out.write(b"  mov byte [__mmio_mem + rax], dl\n"),
                2 => gen.out.write(b"  mov word [__mmio_mem + rax], dx\n"),
                4 => gen.out.write(b"  mov dword [__mmio_mem + rax], edx\n"),
                8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rdx\n"),
                _ => {}
            }
        }
        lir::WriteKind::W1s => {
            // Write-1-to-set: *reg = *reg | value
            match width {
                1 => gen.out.write(b"  movzx rdx, byte [__mmio_mem + rax]\n"),
                2 => gen.out.write(b"  movzx rdx, word [__mmio_mem + rax]\n"),
                4 => gen.out.write(b"  mov edx, dword [__mmio_mem + rax]\n"),
                8 => gen.out.write(b"  mov rdx, qword [__mmio_mem + rax]\n"),
                _ => {
                    gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
                    return Ok(());
                }
            }
            gen.out.write(b"  or rdx, rcx\n");
            match width {
                1 => gen.out.write(b"  mov byte [__mmio_mem + rax], dl\n"),
                2 => gen.out.write(b"  mov word [__mmio_mem + rax], dx\n"),
                4 => gen.out.write(b"  mov dword [__mmio_mem + rax], edx\n"),
                8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rdx\n"),
                _ => {}
            }
        }
        lir::WriteKind::Xor => {
            // Write-1-to-invert: *reg = *reg ^ value
            match width {
                1 => gen.out.write(b"  movzx rdx, byte [__mmio_mem + rax]\n"),
                2 => gen.out.write(b"  movzx rdx, word [__mmio_mem + rax]\n"),
                4 => gen.out.write(b"  mov edx, dword [__mmio_mem + rax]\n"),
                8 => gen.out.write(b"  mov rdx, qword [__mmio_mem + rax]\n"),
                _ => {
                    gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
                    return Ok(());
                }
            }
            gen.out.write(b"  xor rdx, rcx\n");
            match width {
                1 => gen.out.write(b"  mov byte [__mmio_mem + rax], dl\n"),
                2 => gen.out.write(b"  mov word [__mmio_mem + rax], dx\n"),
                4 => gen.out.write(b"  mov dword [__mmio_mem + rax], edx\n"),
                8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rdx\n"),
                _ => {}
            }
        }
    }
    Ok(())
}

pub fn emit_mmio_load_field(
    gen: &mut X86_64HostedBackend<'_>,
    reg_width: u32,
    field_bits: u16,
    field_signed: bool,
    mask: u64,
    shift: u8,
    span: Span,
) -> Result<(), CodegenError> {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n");
    emit_mmio_bounds_check(gen, reg_width, span)?;

    match reg_width {
        1 => gen.out.write(b"  movzx rcx, byte [__mmio_mem + rax]\n"),
        2 => gen.out.write(b"  movzx rcx, word [__mmio_mem + rax]\n"),
        4 => gen.out.write(b"  mov ecx, dword [__mmio_mem + rax]\n"),
        8 => gen.out.write(b"  mov rcx, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return Ok(());
        }
    }

    gen.out.write(b"  mov r8, ");
    write_u64_hex(gen.out, mask);
    gen.out.write(b"\n");
    gen.out.write(b"  and rcx, r8\n");
    if shift != 0 {
        gen.out.write(b"  shr rcx, ");
        write_u32(gen.out, shift as u32);
        gen.out.write(b"\n");
    }

    gen.out.write(b"  mov rax, rcx\n");
    if field_bits < 64 {
        if field_bits <= 32 {
            gen.out.write(b"  and eax, ");
            write_u64_hex(gen.out, mask_for_bits(field_bits));
            gen.out.write(b"\n");
        } else {
            gen.out.write(b"  and rax, ");
            write_u64_hex(gen.out, mask_for_bits(field_bits));
            gen.out.write(b"\n");
        }
        if field_signed {
            let sh = 64u32 - (field_bits as u32);
            gen.out.write(b"  shl rax, ");
            write_u32(gen.out, sh);
            gen.out.write(b"\n");
            gen.out.write(b"  sar rax, ");
            write_u32(gen.out, sh);
            gen.out.write(b"\n");
        }
    }

    gen.out.write(b"  mov [r15], rax\n");
    gen.out.write(b"  add r15, 8\n");
    Ok(())
}

pub fn emit_mmio_store_field(
    gen: &mut X86_64HostedBackend<'_>,
    reg_width: u32,
    mask: u64,
    shift: u8,
    span: Span,
) -> Result<(), CodegenError> {
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rcx, [r15]\n"); // field value
    gen.out.write(b"  sub r15, 8\n");
    gen.out.write(b"  mov rax, [r15]\n"); // addr
    emit_mmio_bounds_check(gen, reg_width, span)?;

    match reg_width {
        1 => gen.out.write(b"  movzx rdx, byte [__mmio_mem + rax]\n"),
        2 => gen.out.write(b"  movzx rdx, word [__mmio_mem + rax]\n"),
        4 => gen.out.write(b"  mov edx, dword [__mmio_mem + rax]\n"),
        8 => gen.out.write(b"  mov rdx, qword [__mmio_mem + rax]\n"),
        _ => {
            gen.emit_trap_with_loc(lir::trap_code_u32(lir::TrapCode::Unreachable), span);
            return Ok(());
        }
    }

    gen.out.write(b"  mov r8, ");
    write_u64_hex(gen.out, mask);
    gen.out.write(b"\n");
    gen.out.write(b"  mov r9, r8\n");
    gen.out.write(b"  not r9\n");
    gen.out.write(b"  and rdx, r9\n");

    gen.out.write(b"  mov r10, rcx\n");
    if shift != 0 {
        gen.out.write(b"  shl r10, ");
        write_u32(gen.out, shift as u32);
        gen.out.write(b"\n");
    }
    gen.out.write(b"  and r10, r8\n");
    gen.out.write(b"  or rdx, r10\n");

    match reg_width {
        1 => gen.out.write(b"  mov byte [__mmio_mem + rax], dl\n"),
        2 => gen.out.write(b"  mov word [__mmio_mem + rax], dx\n"),
        4 => gen.out.write(b"  mov dword [__mmio_mem + rax], edx\n"),
        8 => gen.out.write(b"  mov qword [__mmio_mem + rax], rdx\n"),
        _ => {}
    }
    Ok(())
}
