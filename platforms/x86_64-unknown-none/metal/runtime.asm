format ELF64

; ---------------------------------------------------------------------------
; PVH ELF note — QEMU 10+ requires this to load a bare-metal kernel via -kernel
;
; XEN_ELFNOTE_PHYS32_ENTRY (type 18): tells QEMU the 32-bit protected-mode
; entry point.  QEMU finds this via the PT_NOTE program header (produced by
; the linker script), then jumps here in 32-bit PE mode with paging OFF.
; ---------------------------------------------------------------------------
section '.note.Xen' align 4
    dd 4            ; namesz: length of "Xen\0"
    dd 4            ; descsz: one 32-bit address
    dd 18           ; type  = XEN_ELFNOTE_PHYS32_ENTRY
    db 'X','e','n',0   ; name
    dd _start32     ; 32-bit entry point (resolved by linker)

; ---------------------------------------------------------------------------
; 32-bit protected-mode entry: set up long mode and jump to 64-bit code
; ---------------------------------------------------------------------------
section '.text.boot' executable
use32

public _start32
_start32:
    cli

    ; Set up a temporary 32-bit stack in BSS
    mov esp, __lang_boot_stack_top

    ; Zero page-table area in BSS (PVH does NOT pre-zero BSS)
    xor eax, eax
    mov edi, __lang_pt_start
    mov ecx, (__lang_pt_end - __lang_pt_start)
    shr ecx, 2
    rep stosd

    ; Build identity-map page tables (first 1 GiB, 2MB pages)
    ; PML4[0] → PDPT
    mov eax, __lang_pdpt
    or  eax, 3       ; Present + RW
    mov [__lang_pml4], eax

    ; PDPT[0] → PD
    mov eax, __lang_pd
    or  eax, 3
    mov [__lang_pdpt], eax

    ; PD: 512 entries × 2MB = 1GB identity map
    mov edi, __lang_pd
    mov eax, 0x83    ; Present + RW + PS (2MB page)
    mov ecx, 512
.fill_pd:
    mov [edi], eax
    add eax, 0x200000
    add edi, 8
    loop .fill_pd

    ; Load CR3
    mov eax, __lang_pml4
    mov cr3, eax

    ; Enable PAE
    mov eax, cr4
    or  eax, (1 shl 5)   ; CR4.PAE
    mov cr4, eax

    ; Set EFER.LME
    mov ecx, 0xC0000080
    rdmsr
    or  eax, (1 shl 8)   ; EFER.LME
    wrmsr

    ; Load 64-bit GDT
    lgdt [gdt64_ptr]

    ; Enable paging → activates long mode
    mov eax, cr0
    or  eax, (1 shl 31) or (1 shl 0)   ; CR0.PG + CR0.PE
    mov cr0, eax

    ; Far jump into 64-bit code segment (selector 0x08)
    jmp 0x08:_start64_trampoline

; 16-byte GDT: null + 64-bit code descriptor
align 8
gdt64:
    dq 0                      ; null descriptor
    dq 0x00AF9A000000FFFF     ; 64-bit code: L=1, P=1, DPL=0, Execute/Read
gdt64_ptr:
    dw (gdt64_ptr - gdt64 - 1)
    dd gdt64

; ---------------------------------------------------------------------------
; 64-bit runtime
; ---------------------------------------------------------------------------
section '.text' executable
use64

; External symbols provided by the compiled user modules.
extrn w_1f5962a2ce9803c8    ; main ( -- i64 )

_start64_trampoline:
    ; We arrive here in 64-bit mode but with 32-bit address space constraints
    ; on the stack pointer.  Fix up RSP to 64-bit.
    mov rsp, __lang_stack_top

public __lang_start
__lang_start:
    mov rsp, __lang_stack_top
    ; R15 = data stack pointer (grows upward from __lang_ds_base)
    mov r15, __lang_ds_base
    ; R14 = data stack upper limit (exclusive)
    mov r14, __lang_ds_limit
    ; Initialize high-water mark to DS base
    mov qword [__lang_ds_high], r15
    ; Initialize V-once flag (BSS is not pre-zeroed under PVH)
    mov qword [__lang_v_emitted], 0
    xor rbp, rbp

    call w_1f5962a2ce9803c8          ; call main

    ; Emit high-water mark: 'H' (0x48) + u32-le (peak DS depth in slots)
    mov rax, [__lang_ds_high]
    sub rax, __lang_ds_base          ; bytes used
    shr rax, 3                       ; slots (slot_bytes = 8)
    mov r12, rax                     ; save
    mov al, 'H'
    mov dx, 0xe9
    out dx, al                       ; prefix byte
    mov rax, r12
    out dx, al                       ; byte 0 (LSB)
    shr rax, 8
    out dx, al                       ; byte 1
    shr rax, 8
    out dx, al                       ; byte 2
    shr rax, 8
    out dx, al                       ; byte 3 (MSB)

    ; Pop exit code from data stack
    sub r15, 8
    mov rax, [r15]

    ; Signal exit via isa-debug-exit port 0x501
    ; guest writes 0 → QEMU exits (0<<1)|1 = 1 (pass)
    ; guest writes N≠0 → QEMU exits (N<<1)|1 (fail)
    test rax, rax
    jz .pass
    mov ax, 1
.pass:
    mov dx, 0x501
    out dx, ax
    cli
    hlt

; ---------------------------------------------------------------------------
; Trap / overflow handlers
;
; Each handler emits a framed D diagnostic record (with data-stack slot dump)
; over port 0xe9, then signals exit via isa-debug-exit port 0x501.
;
; __lang_trap_loc     — trap with source location (rdi=trap_code, rsi=valid,
;                       rdx=line, rcx=word_hash)
; __lang_trap         — generic trap (rdi=trap_code, no payload -> valid=0)
; __stack_overflow    — data-stack overflow detected at runtime
;
; Register convention for emit_diag:
;   r8  = original DS pointer (for slot dump, r15 at trap time)
;   r12 = trap_code          (u64, low 16 bits used)
;   r13 = valid flag         (0 or 1)
;   r14 = source_line        (u64, low 32 bits used)
;   r15 = word_hash          (full u64)
;   rbp = slot_count         (capped at 16, also used as ds_depth in header)
; ---------------------------------------------------------------------------

; ---- isa-debug-exit tail (shared) ----
; Writes 0xff to port 0x501 and halts.
trap_exit:
    mov ax, 0xff
    mov dx, 0x501
    out dx, ax
    cli
    hlt

; ---- Shared D record + slot dump emitter ----
; Jumped to from each handler after setting r8, r12-r15, rbp per convention
; above.  Emits V (once) + framed D record (header + slot data).
slot_emit_max equ 16               ; cap: never dump more than 16 slots

emit_diag:
    ; Emit V version record at most once.
    cmp qword [__lang_v_emitted], 0
    jne emit_diag_header
    mov al, 'V'
    out 0xe9, al
    mov al, 1
    out 0xe9, al
    xor al, al
    out 0xe9, al
    mov al, 1
    out 0xe9, al
    mov qword [__lang_v_emitted], 1

emit_diag_header:
    ; D marker (0x44)
    mov al, 'D'
    out 0xe9, al

    ; Total payload length = 35 (header) + slot_count * 8.
    mov r9, rbp
    shl r9, 3           ; r9 = slot_count * 8
    add r9, 35          ; r9 = total length (fits in u16)
    mov rax, r9
    out 0xe9, al        ; length byte 0 (LSB)
    shr rax, 8
    out 0xe9, al        ; length byte 1 (MSB)

    ; Byte  0: DiagRecord.version = 1
    mov al, 1
    out 0xe9, al

    ; Byte  1: origin = IN_GUEST (1)
    mov al, 1
    out 0xe9, al

    ; Byte  2: valid = r13 (0 or 1)
    mov al, r13b
    out 0xe9, al

    ; Bytes 3-4: trap_code (u16-le) from r12
    mov rax, r12
    out 0xe9, al
    shr rax, 8
    out 0xe9, al

    ; Bytes 5-8: source_line (u32-le) from r14
    mov rax, r14
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al

    ; Bytes 9-16: word_hash (u64-le) from r15
    mov rax, r15
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al

    ; Bytes 17-24: trap_pc (u64-le) = 0 (not available in-guest)
    xor al, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al

    ; Bytes 25-28: ds_depth (u32-le) from rbp (same as slot_count)
    mov rax, rbp
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al

    ; Bytes 29-32: ds_declared = 0xFFFFFFFF (unknown / ⊤)
    mov al, 0xFF
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al
    out 0xe9, al

    ; Bytes 33-34: slot_count (u16-le) from rbp
    mov rax, rbp
    out 0xe9, al
    shr rax, 8
    out 0xe9, al

    ; ---- Data-stack slot dump (deepest-last order) ----
    ; rbp = slot_count, r8 = original DS pointer (top of live area).
    ; Iterate downward: each slot is 8 bytes, read at [r8 - 8*k].
    mov rcx, rbp        ; remaining slot count
    test rcx, rcx
    jz slot_emit_done

slot_emit_loop:
    sub r8, 8           ; move down one slot (toward base)
    mov rax, [r8]       ; read slot value (u64)
    ; emit 8 bytes LE
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    shr rax, 8
    out 0xe9, al
    dec rcx
    jnz slot_emit_loop

slot_emit_done:
    jmp trap_exit

; ---- __lang_trap_loc: trap with payload (debug_trap_loc=true) ----
; Register contract:
;   rdi = trap_code, rsi = valid (1), rdx = line, rcx = word_hash
public __lang_trap_loc
__lang_trap_loc:
    ; Save original DS pointer and compute capped slot_count.
    mov r8, r15          ; save DS pointer for slot dump
    mov rax, r15
    sub rax, __lang_ds_base
    shr rax, 3           ; rax = live slots
    cmp rax, slot_emit_max
    jbe .slot_cap_loc
    mov rax, slot_emit_max
.slot_cap_loc:
    mov rbp, rax          ; rbp = capped slot_count
    ; Save payload registers.
    mov r12, rdi          ; trap_code
    mov r13, rsi          ; valid
    mov r14, rdx          ; line
    mov r15, rcx          ; word_hash
    jmp emit_diag

; ---- __lang_trap: generic trap (no payload) ----
; rdi = trap_code (set by codegen), rsi/rdx/rcx = undefined.
public __lang_trap
__lang_trap:
    mov r8, r15
    mov rax, r15
    sub rax, __lang_ds_base
    shr rax, 3
    cmp rax, slot_emit_max
    jbe .slot_cap_trap
    mov rax, slot_emit_max
.slot_cap_trap:
    mov rbp, rax
    mov r12, rdi
    xor r13, r13          ; valid = 0
    xor r14, r14          ; line = 0
    xor r15, r15          ; word_hash = 0
    jmp emit_diag

; ---- __stack_overflow: data-stack overflow ----
public __stack_overflow
__stack_overflow:
    mov r8, r15
    mov rax, r15
    sub rax, __lang_ds_base
    shr rax, 3
    cmp rax, slot_emit_max
    jbe .slot_cap_stk
    mov rax, slot_emit_max
.slot_cap_stk:
    mov rbp, rax
    mov r12, 10           ; trap_code = STACK_OVERFLOW
    xor r13, r13          ; valid = 0
    xor r14, r14          ; line = 0
    xor r15, r15          ; word_hash = 0
    jmp emit_diag

; ---------------------------------------------------------------------------
; testio words — called by compiled tyu_lang code via normal ABI
;
; Symbols use the same mangling convention as langc:
;   fnv1a_u64(word name bytes), prefixed with w_
;
; testio.write-byte ( i64 -- )
;   Pop one cell from data stack, write low byte to QEMU debugcon (port 0xe9)
;   fnv1a_u64("testio.write-byte") = accb676a903a06d9
; ---------------------------------------------------------------------------

; platform.uart.init ( baud -- )
;   QEMU debugcon needs no initialization; consume the baud value and return.
;   fnv1a_u64("platform.uart.init") = 6f29c37992fecaf8
public w_6f29c37992fecaf8
w_6f29c37992fecaf8:
    sub r15, 8
    ret

; platform.uart.tx ( u8 -- )
;   Write low byte to QEMU debugcon (port 0xe9).
;   fnv1a_u64("platform.uart.tx") = 38276faeb09bf91e
public w_38276faeb09bf91e
w_38276faeb09bf91e:
    sub r15, 8
    mov rax, [r15]
    out 0xe9, al
    ret

; platform.uart.rx ( -- u8 ok )
;   QEMU debugcon is write-only in this pack; return no byte available.
;   fnv1a_u64("platform.uart.rx") = 38126baeb0899648
public w_38126baeb0899648
w_38126baeb0899648:
    xor rax, rax
    mov [r15], rax
    add r15, 8
    mov [r15], rax
    add r15, 8
    ret

; platform.time.now_us ( -- i64 )
;   Monotonic timestamp derived from rdtsc. Good enough for QEMU smoke tests.
;   fnv1a_u64("platform.time.now_us") = 6a5791a972f2fbd0
public w_6a5791a972f2fbd0
w_6a5791a972f2fbd0:
    rdtsc
    shl rdx, 32
    or rax, rdx
    mov [r15], rax
    add r15, 8
    ret

; platform.time.reboot ( -- )
;   No restart path in this pack; halt after requesting exit.
;   fnv1a_u64("platform.time.reboot") = 46f6f74f7859ca64
public w_46f6f74f7859ca64
w_46f6f74f7859ca64:
    cli
    hlt

; platform.gpio.init ( pin mode -- )
;   QEMU stub: consume the arguments and clear the synthetic latch.
;   fnv1a_u64("platform.gpio.init") = a6b1202e57aa7cc9
public w_a6b1202e57aa7cc9
w_a6b1202e57aa7cc9:
    sub r15, 16
    mov byte [__lang_gpio_state], 0
    ret

; platform.gpio.write ( pin bool -- )
;   QEMU stub: store the bool in a synthetic latch.
;   fnv1a_u64("platform.gpio.write") = eb1d0a3c5e7c2e92
public w_eb1d0a3c5e7c2e92
w_eb1d0a3c5e7c2e92:
    sub r15, 16
    mov al, [r15+8]
    mov byte [__lang_gpio_state], al
    ret

; platform.gpio.read ( pin -- bool )
;   QEMU stub: return the synthetic latch.
;   fnv1a_u64("platform.gpio.read") = 034a1ff17acf93d3
public w_034a1ff17acf93d3
w_034a1ff17acf93d3:
    sub r15, 8
    xor rax, rax
    mov al, [__lang_gpio_state]
    mov [r15], rax
    add r15, 8
    ret

; testio.write-byte ( i64 -- )
;   Alias to platform.uart.tx.
;   fnv1a_u64("testio.write-byte") = accb676a903a06d9
; ---------------------------------------------------------------------------

; testio.write-byte
public w_accb676a903a06d9
w_accb676a903a06d9:
    jmp w_38276faeb09bf91e

; testio.write-str ( str -- )
;   str is a pointer to a length-prefixed byte string: [u64 len][u8 bytes...]
;   fnv1a_u64("testio.write-str") = eb06855547211672
public w_eb06855547211672
w_eb06855547211672:
    sub r15, 8
    mov rsi, [r15]       ; rsi = pointer to string struct
    mov rcx, [rsi]       ; rcx = length
    add rsi, 8           ; rsi = pointer to first byte
    test rcx, rcx
    jz .done
.loop:
    mov al, [rsi]
    out 0xe9, al
    inc rsi
    dec rcx
    jnz .loop
.done:
    ret

; testio.exit ( i64 -- )
;   fnv1a_u64("testio.exit") = f91ca4f233247b4d
public w_f91ca4f233247b4d
w_f91ca4f233247b4d:
    sub r15, 8
    mov rax, [r15]
    test rax, rax
    jz .pass
    mov ax, 1
.pass:
    mov dx, 0x501
    out dx, ax
    cli
    hlt

; ---------------------------------------------------------------------------
; BSS — stacks and page tables
; ---------------------------------------------------------------------------
section '.bss' align 4096 writeable

    ; Boot stack (used during 32→64 transition only)
    rb 4096
__lang_boot_stack_top:

    ; Page tables: PML4, PDPT, PD (4KB each)
__lang_pt_start:
__lang_pml4:
    rb 4096
__lang_pdpt:
    rb 4096
__lang_pd:
    rb 4096
__lang_pt_end:

    ; Synthetic GPIO latch for the QEMU stub capability surface.
__lang_gpio_state:
    db 0

    ; Call stack — 64KB
    rb 65536
__lang_stack_top:

    ; Data stack — 128KB. R15 starts at __lang_ds_base (low address)
    ; and grows upward. R14 = __lang_ds_limit (upper bound, exclusive).
public __lang_ds_base
__lang_ds_base:
    rb 131072
public __lang_ds_limit
__lang_ds_limit:
public __lang_ds_high
__lang_ds_high:
    dq 0
public __lang_expected_abi_hash
__lang_expected_abi_hash:
    ; compute_abi_hash(ARCH_TAG_X86_64=1, slot=8, word=64, MODINFO_VER=2), recipe v2
    dq 0xf2f245c307c5986a

    ; V-once flag — 0 before V is emitted, 1 after.
public __lang_v_emitted
__lang_v_emitted:
    dq 0
