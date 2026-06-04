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
extrn w_6d61696e    ; main ( -- i64 )

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
    xor rbp, rbp

    call w_6d61696e          ; call main (mangled: 'main' in hex)

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
; Trap / overflow handlers (same public names as hosted runtime)
; ---------------------------------------------------------------------------

public __lang_trap
__lang_trap:
public __lang_trap_loc
__lang_trap_loc:
    mov ax, 0xff
    mov dx, 0x501
    out dx, ax
    cli
    hlt

public __stack_overflow
__stack_overflow:
    mov ax, 0x0a
    mov dx, 0x501
    out dx, ax
    cli
    hlt

; ---------------------------------------------------------------------------
; testio words — called by compiled tyu_lang code via normal ABI
;
; Symbols use the same mangling convention as langc:
;   word name bytes encoded as lowercase hex, prefixed with w_
;
; testio.write-byte ( i64 -- )
;   Pop one cell from data stack, write low byte to QEMU debugcon (port 0xe9)
;   Mangled: "testio.write-byte"
;     t=74 e=65 s=73 t=74 i=69 o=6f .=2e w=77 r=72 i=69 t=74 e=65 -=2d b=62 y=79 t=74 e=65
; ---------------------------------------------------------------------------

; testio.write-byte
public w_74657374696f2e77726974652d62797465
w_74657374696f2e77726974652d62797465:
    sub r15, 8
    mov rax, [r15]
    out 0xe9, al
    ret

; testio.write-str ( str -- )
;   str is a pointer to a length-prefixed byte string: [u64 len][u8 bytes...]
;   Mangled: "testio.write-str"
;     t=74 e=65 s=73 t=74 i=69 o=6f .=2e w=77 r=72 i=69 t=74 e=65 -=2d s=73 t=74 r=72
public w_74657374696f2e77726974652d737472
w_74657374696f2e77726974652d737472:
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
;   Mangled: "testio.exit"
;     t=74 e=65 s=73 t=74 i=69 o=6f .=2e e=65 x=78 i=69 t=74
public w_74657374696f2e65786974
w_74657374696f2e65786974:
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

    ; Call stack — 64KB
    rb 65536
__lang_stack_top:

    ; Data stack — 128KB. R15 starts at __lang_ds_base (low address)
    ; and grows upward. R14 = __lang_ds_limit (upper bound, exclusive).
__lang_ds_base:
    rb 131072
__lang_ds_limit:
public __lang_ds_high
__lang_ds_high:
    dq 0
public __lang_expected_abi_hash
__lang_expected_abi_hash:
    dq 0x7ff852243aa7202b
