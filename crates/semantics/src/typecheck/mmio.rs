use crate::typecheck::error::TcError;
use crate::typecheck::place::{PlacePath, Step};
use crate::typecheck::util::{parse_u32_any, slice_span};
use crate::types::TypeAtom;
use codegen_core::compiled_desc::{
    CompiledDescriptor, REG_ACCESS_RO, REG_ACCESS_RW, REG_ACCESS_WO,
};
use frontend::fixed::FixedVec;
use frontend::lex::Lexer;
use frontend::parse::{DeclKind, ModuleAst};
use frontend::span::Span;
use frontend::token::TokenKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Ro,
    Wo,
    Rw,
    W1c,
    W1s,
    Rc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioFieldInfo {
    pub name: TypeAtom,
    pub lo: u8,
    pub hi: u8,
    pub ty: TypeAtom,
    pub access: AccessMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioMapDecl {
    pub name: TypeAtom,
    pub body: Span,
}

/// How a register-map instance is based (P4, §5.7).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceBase {
    /// Raw absolute address (`MAP @ 0x…`), the legacy descriptor-less path.
    Raw(u64),
    /// Symbolic board instance (`MAP @ board.<instance>`, P4): aperture-relative.
    Symbolic { aperture: u16, base_offset: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioInstance {
    pub name: TypeAtom,
    pub map: TypeAtom,
    pub base: InstanceBase,
    /// The descriptor device name from `@ board.<name>`, when symbolic (P5).
    pub board: Option<TypeAtom>,
}

pub struct MmioDb {
    pub maps: FixedVec<MmioMapDecl, 16>,
    pub instances: FixedVec<MmioInstance, 64>,
    /// Descriptor access meta per board *instance* (P5). The same register-map
    /// name can be instantiated as several devices with different register
    /// sets (e.g. `Scratch` @ scratch / datascratch), so the rows are keyed by
    /// the instance name, not the map name.
    pub reg_meta: FixedVec<InstanceAccessMeta, 64>,
}

/// Descriptor access rows for one board instance (P5).
/// No derives: `FixedVec` is a plain aggregate.
pub struct InstanceAccessMeta {
    pub instance: TypeAtom,
    pub rows: FixedVec<RegAccessMeta, 32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegAccessMeta {
    pub offset: u32,
    pub meta: MmioAccessMeta,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioRegInfo {
    pub offset: u32,
    pub reg: TypeAtom,
    pub reg_ty: TypeAtom,
    pub access: AccessMode,
    pub volatile: bool,
    pub array_len: Option<u32>,
}

/// Descriptor-derived register access meta (P5, D-3), attached at resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioAccessMeta {
    pub write_kind: ir::WriteKind,
    pub read_kind: ir::ReadKind,
    pub atomic_max: u8,
    pub barrier: ir::BarrierKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioResolvedReg {
    pub map: TypeAtom,
    pub reg: TypeAtom,
    pub reg_ty: TypeAtom,
    pub access: AccessMode,
    pub volatile: bool,
    /// Aperture-relative place (P4): the module aperture-use id + byte offset.
    pub aperture: u16,
    pub offset: u32,
    pub array_len: Option<u32>,
    /// Descriptor access semantics (P5).
    pub meta: MmioAccessMeta,
    // The original source span of the whole place (e.g. `gpio.OUT_SET`).
    pub place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmioResolvedField {
    pub map: TypeAtom,
    pub reg: TypeAtom,
    pub reg_ty: TypeAtom,
    pub reg_access: AccessMode,
    pub field: MmioFieldInfo,
    pub volatile: bool,
    pub aperture: u16,
    pub offset: u32,
    pub array_len: Option<u32>,
    /// Descriptor access semantics of the enclosing register (P5).
    pub meta: MmioAccessMeta,
    pub place_span: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmioResolved {
    Reg(MmioResolvedReg),
    Field(MmioResolvedField),
}

pub fn access_can_read(access: AccessMode) -> bool {
    !matches!(access, AccessMode::Wo)
}

pub fn access_can_write(access: AccessMode) -> bool {
    !matches!(access, AccessMode::Ro)
}

/// Fused per-aperture access-mask bits for a register access (design doc §5.5).
pub fn aperture_access_bits(access: AccessMode) -> u8 {
    use ir::{
        ACCESS_EFFECTFUL_READ, ACCESS_READ, ACCESS_W1C, ACCESS_W1S, ACCESS_WRITE,
    };
    match access {
        AccessMode::Ro => ACCESS_READ,
        AccessMode::Wo => ACCESS_WRITE,
        AccessMode::Rw => ACCESS_READ | ACCESS_WRITE,
        AccessMode::W1c => ACCESS_WRITE | ACCESS_W1C,
        AccessMode::W1s => ACCESS_WRITE | ACCESS_W1S,
        AccessMode::Rc => ACCESS_READ | ACCESS_EFFECTFUL_READ,
    }
}

pub fn field_mask_shift(field: &MmioFieldInfo) -> (u64, u8) {
    let lo = field.lo as u32;
    let hi = field.hi as u32;
    let width = hi.saturating_sub(lo).saturating_add(1).min(64);
    let mask = if width >= 64 {
        !0u64
    } else {
        (1u64 << width) - 1
    };
    (mask << lo, field.lo)
}

pub fn mmio_type_width_bytes(ty: &[u8]) -> Option<u32> {
    match ty {
        b"bool" | b"u8" | b"i8" => Some(1),
        b"u16" | b"i16" => Some(2),
        b"u32" | b"i32" => Some(4),
        b"u64" | b"i64" => Some(8),
        _ => None,
    }
}

pub fn build_mmio_db(
    module: &ModuleAst,
    src: &[u8],
    descriptor: Option<&CompiledDescriptor>,
) -> Result<MmioDb, TcError> {
    let mut db = MmioDb {
        maps: FixedVec::new(),
        instances: FixedVec::new(),
        reg_meta: FixedVec::new(),
    };

    for inst in module.instances.iter() {
        // A name that exceeds the atom limit would otherwise make the
        // instance silently vanish from the db — every lookup would then
        // fail with a misleading "map/instance not found" (BUG-013).
        let name = TypeAtom::new(slice_span(src, inst.name))
            .ok_or(TcError::MmioNameInvalid { span: inst.name })?;
        let map = TypeAtom::new(slice_span(src, inst.map))
            .ok_or(TcError::MmioNameInvalid { span: inst.map })?;
        let base = resolve_instance_base(inst, src, descriptor)?;
        let board = inst
            .board_instance
            .map(|bs| {
                TypeAtom::new(slice_span(src, bs)).ok_or(TcError::MmioNameInvalid { span: bs })
            })
            .transpose()?;
        db.instances
            .push(MmioInstance {
                name,
                map,
                base,
                board,
            })
            .map_err(|_| TcError::MmioInstanceCapacityExceeded { span: inst.name })?;
        // P5: the board instance's device rows are this instance's
        // authoritative register access semantics.
        let rows = match (&board, descriptor) {
            (Some(b), Some(desc)) => descriptor_instance_meta(desc, *b)?,
            _ => FixedVec::new(),
        };
        db.reg_meta
            .push(InstanceAccessMeta {
                instance: board.unwrap_or(name),
                rows,
            })
            .map_err(|_| TcError::MmioInstanceCapacityExceeded { span: inst.name })?;
    }

    for decl in module.decls.iter() {
        if decl.kind != DeclKind::RegisterMap {
            continue;
        }
        let Some(body) = decl.body else {
            continue;
        };
        let map_name = TypeAtom::new(slice_span(src, decl.name))
            .ok_or(TcError::MmioNameInvalid { span: decl.name })?;
        validate_regmap_body(src, body)?;
        if let Some(desc) = descriptor {
            check_regmap_against_descriptor(src, body, map_name, desc)?;
        }
        db.maps
            .push(MmioMapDecl {
                name: map_name,
                body,
            })
            .map_err(|_| TcError::MmioMapCapacityExceeded { span: decl.name })?;
    }

    Ok(db)
}

/// Build the descriptor's access-meta rows for a board instance (P5). Refuses
/// unknown register-access kinds (E3646): a descriptor written against a newer
/// registry must not be silently half-understood.
fn descriptor_instance_meta(
    desc: &CompiledDescriptor,
    instance: TypeAtom,
) -> Result<FixedVec<RegAccessMeta, 32>, TcError> {
    let mut rows: FixedVec<RegAccessMeta, 32> = FixedVec::new();
    let inst = ir::Atom::new(instance.as_bytes());
    let Some(inst) = inst else {
        return Ok(rows);
    };
    let Some(device) = desc.device(inst) else {
        return Ok(rows);
    };
    for r in device.registers() {
        let write_kind = match r.write_kind {
            codegen_core::compiled_desc::REG_WRITE_PLAIN => ir::WriteKind::Plain,
            codegen_core::compiled_desc::REG_WRITE_W1C => ir::WriteKind::W1c,
            codegen_core::compiled_desc::REG_WRITE_W1S => ir::WriteKind::W1s,
            codegen_core::compiled_desc::REG_WRITE_XOR => ir::WriteKind::Xor,
            _ => return Err(TcError::MmioUnknownRegisterKind { span: Span::UNKNOWN }),
        };
        let read_kind = match r.read_kind {
            codegen_core::compiled_desc::REG_READ_EFFECTFUL => ir::ReadKind::Effectful,
            codegen_core::compiled_desc::REG_READ_PLAIN => ir::ReadKind::Plain,
            _ => return Err(TcError::MmioUnknownRegisterKind { span: Span::UNKNOWN }),
        };
        let barrier = match r.barrier {
            codegen_core::compiled_desc::REG_BARRIER_BEFORE => ir::BarrierKind::Before,
            codegen_core::compiled_desc::REG_BARRIER_AFTER => ir::BarrierKind::After,
            codegen_core::compiled_desc::REG_BARRIER_BOTH => ir::BarrierKind::Both,
            codegen_core::compiled_desc::REG_BARRIER_NONE => ir::BarrierKind::None,
            _ => return Err(TcError::MmioUnknownRegisterKind { span: Span::UNKNOWN }),
        };
        let _ = rows.push(RegAccessMeta {
            offset: r.offset,
            meta: MmioAccessMeta {
                write_kind,
                read_kind,
                atomic_max: r.atomic_max,
                barrier,
            },
        });
    }
    Ok(rows)
}

/// The descriptor access meta for a register offset in a board instance (P5).
fn reg_access_meta(db: &MmioDb, instance: TypeAtom, offset: u32) -> MmioAccessMeta {
    let default = MmioAccessMeta {
        write_kind: ir::WriteKind::Plain,
        read_kind: ir::ReadKind::Plain,
        atomic_max: 64,
        barrier: ir::BarrierKind::None,
    };
    for m in db.reg_meta.iter() {
        if m.instance != instance {
            continue;
        }
        for row in m.rows.iter() {
            if row.offset == offset {
                return row.meta;
            }
        }
    }
    default
}

/// Resolve an instance's base operand (§5.7): `board.<instance>` looks the
/// device up in the descriptor; a raw integer is the legacy path (rejected
/// under a descriptor, E3641).
fn resolve_instance_base(
    inst: &frontend::parse::RegMapInstanceAst,
    src: &[u8],
    descriptor: Option<&CompiledDescriptor>,
) -> Result<InstanceBase, TcError> {
    if let Some(board_span) = inst.board_instance {
        // `board.<instance>` requires a descriptor (E3640 if absent).
        let Some(desc) = descriptor else {
            return Err(TcError::MmioNeedsDescriptor { span: inst.name });
        };
        let instance = TypeAtom::new(slice_span(src, board_span))
            .ok_or(TcError::MmioNameInvalid { span: board_span })?;
        let atom = ir::Atom::new(instance.as_bytes())
            .ok_or(TcError::MmioNameInvalid { span: board_span })?;
        let device = desc.device(atom).ok_or(TcError::MmioBoardInstanceNotFound {
            span: board_span,
        })?;
        return Ok(InstanceBase::Symbolic {
            aperture: device.aperture,
            base_offset: device.base_offset,
        });
    }
    // Raw integer base.
    let base_addr = parse_u32_any(slice_span(src, inst.base_addr))
        .ok_or(TcError::MmioAddrInvalid { span: inst.base_addr })?
        as u64;
    if descriptor.is_some() {
        return Err(TcError::MmioRawBaseUnderDescriptor { span: inst.base_addr });
    }
    Ok(InstanceBase::Raw(base_addr))
}

/// E3647: every source register-map row must be matched by a descriptor
/// device row with the same (offset, name, width, access).
fn check_regmap_against_descriptor(
    src: &[u8],
    body: Span,
    map_name: TypeAtom,
    descriptor: &CompiledDescriptor,
) -> Result<(), TcError> {
    let slice = &src[body.start..body.end];
    let mut lex = Lexer::new(slice);
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            return Ok(());
        }
        if tok.kind != TokenKind::Number {
            continue;
        }
        let offset = parse_u32_any(&slice[tok.span.start..tok.span.end]).ok_or(
            TcError::MmioParseFailed {
                span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
            },
        )?;
        let name_tok = lex.next();
        if name_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedIdent {
                span: Span::new(
                    body.start + name_tok.span.start,
                    body.start + name_tok.span.end,
                ),
            });
        }
        // Skip optional `[N]` bracket array suffix.
        {
            let mut probe = lex;
            let maybe_bracket = probe.next();
            if maybe_bracket.kind == TokenKind::PunctLBracket {
                probe.next();
                probe.next();
                lex = probe;
            }
        }
        let ty_tok = lex.next();
        if ty_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedType {
                span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end),
            });
        }
        let ty_bytes = &slice[ty_tok.span.start..ty_tok.span.end];
        let Some(width) = mmio_type_width_bytes(ty_bytes) else {
            return Err(TcError::MmioRegUnknownWidth {
                span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end),
            });
        };
        let access_tok = lex.next();
        if access_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedAccess {
                span: Span::new(
                    body.start + access_tok.span.start,
                    body.start + access_tok.span.end,
                ),
            });
        }
        let Some(access) = parse_access_mode(&slice[access_tok.span.start..access_tok.span.end])
        else {
            return Err(TcError::MmioInvalidAccess {
                span: Span::new(
                    body.start + access_tok.span.start,
                    body.start + access_tok.span.end,
                ),
            });
        };
        // Skip optional `volatile` and field block `{ ... }`.
        {
            let mut probe = lex;
            let next = probe.next();
            if next.kind == TokenKind::Ident
                && &slice[next.span.start..next.span.end] == b"volatile"
            {
                lex = probe;
            }
            let mut probe2 = lex;
            if probe2.next().kind == TokenKind::PunctLBrace {
                lex = probe2;
                loop {
                    match lex.next().kind {
                        TokenKind::PunctRBrace => break,
                        TokenKind::Eof => {
                            return Err(TcError::MmioUnexpectedEof {
                                span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
                            })
                        }
                        _ => {}
                    }
                }
            }
        }

        let reg_name_bytes = &slice[name_tok.span.start..name_tok.span.end];
        let Some(reg_name) = TypeAtom::new(reg_name_bytes) else {
            return Err(TcError::MmioNameInvalid { span: name_tok.span });
        };
        if !descriptor_has_row(descriptor, map_name, offset, reg_name, width, access) {
            return Err(TcError::MmioRowDivergesFromDescriptor {
                span: Span::new(
                    body.start + tok.span.start,
                    body.start + ty_tok.span.end,
                ),
            });
        }
    }
}

/// True when some descriptor device with `map_name` declares a register row
/// matching (offset, name, width, access).
fn descriptor_has_row(
    descriptor: &CompiledDescriptor,
    map_name: TypeAtom,
    offset: u32,
    reg_name: TypeAtom,
    width: u32,
    access: AccessMode,
) -> bool {
    let map = ir::Atom::new(map_name.as_bytes());
    let Some(map) = map else {
        return false;
    };
    for device in descriptor.devices() {
        if device.map != map {
            continue;
        }
        for r in device.registers() {
            if r.offset == offset
                && r.name.as_bytes() == reg_name.as_bytes()
                && r.width as u32 == width.wrapping_mul(8)
                && compiled_access_matches(access, r.access)
            {
                return true;
            }
        }
    }
    false
}

/// Map a source `AccessMode` onto the compiled access discriminant for the
/// E3647 match (ro/wo/rw only — w1c/w1s/rc are source-level refinements).
fn compiled_access_matches(source: AccessMode, compiled: u8) -> bool {
    match source {
        AccessMode::Ro => compiled == REG_ACCESS_RO,
        AccessMode::Wo => compiled == REG_ACCESS_WO,
        AccessMode::Rw | AccessMode::W1c | AccessMode::W1s | AccessMode::Rc => {
            compiled == REG_ACCESS_RW
        }
    }
}

fn validate_regmap_body(src: &[u8], body: Span) -> Result<(), TcError> {
    let slice = &src[body.start..body.end];
    let mut lex = Lexer::new(slice);
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            break;
        }
        if tok.kind != TokenKind::Number {
            // Skip a `{ field ... }` block so its bit numbers are not
            // mistaken for register offsets.
            if tok.kind == TokenKind::PunctLBrace {
                let mut depth = 1usize;
                while depth > 0 {
                    let inner = lex.next();
                    if inner.kind == TokenKind::PunctLBrace {
                        depth += 1;
                    } else if inner.kind == TokenKind::PunctRBrace {
                        depth -= 1;
                    } else if inner.kind == TokenKind::Eof {
                        return Err(TcError::MmioUnexpectedEof { span: body });
                    }
                }
            }
            continue;
        }

        let offset = parse_u32_any(&slice[tok.span.start..tok.span.end]).ok_or(
            TcError::MmioParseFailed {
                span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
            },
        )?;

        let name_tok = lex.next();
        if name_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedIdent {
                span: Span::new(
                    body.start + name_tok.span.start,
                    body.start + name_tok.span.end,
                ),
            });
        }
        // S-13: skip optional `[N]` bracket array suffix (now separate tokens)
        {
            let mut probe = lex;
            let maybe_bracket = probe.next();
            if maybe_bracket.kind == TokenKind::PunctLBracket {
                probe.next(); // skip number
                probe.next(); // skip ']'
                lex = probe; // commit: advance past `[N]`
            }
            // else: not an array — keep lex unchanged
        }

        let ty_tok = lex.next();
        if ty_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedType {
                span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end),
            });
        }
        let ty_bytes = &slice[ty_tok.span.start..ty_tok.span.end];
        let Some(width) = mmio_type_width_bytes(ty_bytes) else {
            return Err(TcError::MmioRegUnknownWidth {
                span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end),
            });
        };
        if offset % width != 0 {
            return Err(TcError::MmioRegMisaligned {
                span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
            });
        }

        let access_tok = lex.next();
        if access_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedAccess {
                span: Span::new(
                    body.start + access_tok.span.start,
                    body.start + access_tok.span.end,
                ),
            });
        }
        if parse_access_mode(&slice[access_tok.span.start..access_tok.span.end]).is_none() {
            return Err(TcError::MmioInvalidAccess {
                span: Span::new(
                    body.start + access_tok.span.start,
                    body.start + access_tok.span.end,
                ),
            });
        }

        // Optional `volatile`
        let mut probe = lex;
        let next = probe.next();
        if next.kind == TokenKind::Ident && &slice[next.span.start..next.span.end] == b"volatile" {
            lex = probe;
        }

        // Optional field block `{ ... }`: validate ranges and access tokens.
        let mut probe2 = lex;
        let maybe_lbrace = probe2.next();
        if maybe_lbrace.kind == TokenKind::PunctLBrace {
            lex = probe2;
            loop {
                let ftok = lex.next();
                match ftok.kind {
                    TokenKind::PunctRBrace => break,
                    TokenKind::Eof => {
                        return Err(TcError::MmioUnexpectedEof {
                            span: Span::new(
                                body.start + maybe_lbrace.span.start,
                                body.start + maybe_lbrace.span.end,
                            ),
                        })
                    }
                    TokenKind::Ident => {
                        let lo_tok = lex.next();
                        if lo_tok.kind != TokenKind::Number {
                            return Err(TcError::MmioExpectedLowBit {
                                span: Span::new(
                                    body.start + lo_tok.span.start,
                                    body.start + lo_tok.span.end,
                                ),
                            });
                        }
                        let lo = parse_u32_any(&slice[lo_tok.span.start..lo_tok.span.end]).ok_or(
                            TcError::MmioBadLowBit {
                                span: Span::new(
                                    body.start + lo_tok.span.start,
                                    body.start + lo_tok.span.end,
                                ),
                            },
                        )?;
                        let mut hi = lo;
                        let mut probe3 = lex;
                        if probe3.next().kind == TokenKind::PunctDblDot {
                            let hi_tok = probe3.next();
                            if hi_tok.kind != TokenKind::Number {
                                return Err(TcError::MmioExpectedHighBit {
                                    span: Span::new(
                                        body.start + hi_tok.span.start,
                                        body.start + hi_tok.span.end,
                                    ),
                                });
                            }
                            hi = parse_u32_any(&slice[hi_tok.span.start..hi_tok.span.end]).ok_or(
                                TcError::MmioBadHighBit {
                                    span: Span::new(
                                        body.start + hi_tok.span.start,
                                        body.start + hi_tok.span.end,
                                    ),
                                },
                            )?;
                            lex = probe3;
                        }
                        let hi = hi.max(lo);
                        let bit_limit = width * 8;
                        if hi >= bit_limit {
                            return Err(TcError::MmioFieldBitRange {
                                span: Span::new(
                                    body.start + lo_tok.span.start,
                                    body.start + lo_tok.span.end,
                                ),
                            });
                        }
                        let fty_tok = lex.next();
                        if fty_tok.kind != TokenKind::Ident {
                            return Err(TcError::MmioExpectedFieldType {
                                span: Span::new(
                                    body.start + fty_tok.span.start,
                                    body.start + fty_tok.span.end,
                                ),
                            });
                        }
                        let faccess_tok = lex.next();
                        if faccess_tok.kind != TokenKind::Ident {
                            return Err(TcError::MmioExpectedFieldAccess {
                                span: Span::new(
                                    body.start + faccess_tok.span.start,
                                    body.start + faccess_tok.span.end,
                                ),
                            });
                        }
                        if parse_access_mode(&slice[faccess_tok.span.start..faccess_tok.span.end])
                            .is_none()
                        {
                            return Err(TcError::MmioInvalidFieldAccess {
                                span: Span::new(
                                    body.start + faccess_tok.span.start,
                                    body.start + faccess_tok.span.end,
                                ),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn parse_access_mode(bytes: &[u8]) -> Option<AccessMode> {
    match bytes {
        b"ro" => Some(AccessMode::Ro),
        b"wo" => Some(AccessMode::Wo),
        b"rw" => Some(AccessMode::Rw),
        b"w1c" => Some(AccessMode::W1c),
        b"w1s" => Some(AccessMode::W1s),
        b"rc" => Some(AccessMode::Rc),
        _ => None,
    }
}

fn parse_name_array(token: &[u8]) -> (&[u8], Option<u32>) {
    // NAME[40]
    let mut i = 0usize;
    while i < token.len() {
        if token[i] == b'\'' {
            let base = &token[..i];
            let mut j = i + 1;
            while j < token.len() && token[j] != b'.' {
                j += 1;
            }
            if j > i + 1 {
                return (base, parse_u32_any(&token[i + 1..j]));
            }
            return (token, None);
        }
        if token[i] == b'[' {
            let base = &token[..i];
            let mut j = i + 1;
            while j < token.len() && token[j] != b']' {
                j += 1;
            }
            if j < token.len() && token[j] == b']' {
                return (base, parse_u32_any(&token[i + 1..j]));
            }
            return (token, None);
        }
        i += 1;
    }
    (token, None)
}

fn find_instance(db: &MmioDb, name: TypeAtom) -> Option<MmioInstance> {
    for inst in db.instances.iter() {
        if inst.name == name {
            return Some(*inst);
        }
    }
    None
}

fn find_map_decl(db: &MmioDb, name: TypeAtom) -> Option<MmioMapDecl> {
    for m in db.maps.iter() {
        if m.name == name {
            return Some(*m);
        }
    }
    None
}

pub fn scan_regmap_for_reg(
    src: &[u8],
    body: Span,
    want_reg: TypeAtom,
    want_field: Option<TypeAtom>,
    place_span: Span,
    _map_name: TypeAtom,
) -> Result<Option<(MmioRegInfo, Option<MmioFieldInfo>)>, TcError> {
    let slice = &src[body.start..body.end];
    let mut lex = Lexer::new(slice);
    loop {
        let tok = lex.next();
        if tok.kind == TokenKind::Eof {
            return Ok(None);
        }
        if tok.kind != TokenKind::Number {
            continue;
        }

        let offset = parse_u32_any(&slice[tok.span.start..tok.span.end]).ok_or(
            TcError::MmioParseFailed {
                span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
            },
        )?;

        let name_tok = lex.next();
        if name_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedIdent { span: place_span });
        }
        let name_bytes = &slice[name_tok.span.start..name_tok.span.end];
        let (base_name, mut array_len) = parse_name_array(name_bytes);
        // S-13: `[` may be a separate token after the ident (old `NAME[N]` syntax
        // was a single ident; now `[` is a separate PunctLBracket token).
        if array_len.is_none() {
            let probe = lex;
            let bracket = lex.next();
            if bracket.kind == TokenKind::PunctLBracket {
                let num_tok = lex.next();
                array_len = if num_tok.kind == TokenKind::Number {
                    parse_u32_any(&slice[num_tok.span.start..num_tok.span.end])
                } else {
                    lex = probe; // restore — not a bracket index after all
                    None
                };
                let close = lex.next(); // consume ']'
                if close.kind != TokenKind::PunctRBracket {
                    lex = probe; // malformed — restore and fall through
                    array_len = None;
                }
            } else {
                lex = probe; // not a bracket — restore
            }
        }
        let Some(reg_name) = TypeAtom::new(base_name) else {
            return Err(TcError::MmioRegNotFound { span: place_span });
        };

        let ty_tok = lex.next();
        if ty_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedType { span: place_span });
        }
        let ty_bytes = &slice[ty_tok.span.start..ty_tok.span.end];
        let Some(reg_ty) = TypeAtom::new(ty_bytes) else {
            return Err(TcError::TypeParseFailed { span: place_span });
        };
        let Some(width) = mmio_type_width_bytes(ty_bytes) else {
            return Err(TcError::MmioRegUnknownWidth {
                span: Span::new(body.start + ty_tok.span.start, body.start + ty_tok.span.end),
            });
        };
        if offset % width != 0 {
            return Err(TcError::MmioRegMisaligned {
                span: Span::new(body.start + tok.span.start, body.start + tok.span.end),
            });
        }

        let access_tok = lex.next();
        if access_tok.kind != TokenKind::Ident {
            return Err(TcError::MmioExpectedAccess { span: place_span });
        }
        let access = parse_access_mode(&slice[access_tok.span.start..access_tok.span.end])
            .ok_or(TcError::MmioInvalidAccess { span: place_span })?;

        let mut volatile = false;
        let mut probe = lex;
        let next = probe.next();
        if next.kind == TokenKind::Ident && &slice[next.span.start..next.span.end] == b"volatile" {
            volatile = true;
            lex = probe;
        }

        // Optional field block.
        let mut field_found: Option<MmioFieldInfo> = None;
        let mut probe2 = lex;
        let maybe_lbrace = probe2.next();
        if maybe_lbrace.kind == TokenKind::PunctLBrace {
            lex = probe2;
            loop {
                let ftok = lex.next();
                match ftok.kind {
                    TokenKind::PunctRBrace => break,
                    TokenKind::Eof => return Err(TcError::MmioUnexpectedEof { span: place_span }),
                    TokenKind::Ident => {
                        let fname_bytes = &slice[ftok.span.start..ftok.span.end];
                        let Some(fname) = TypeAtom::new(fname_bytes) else {
                            return Err(TcError::TypeParseFailed { span: place_span });
                        };

                        let lo_tok = lex.next();
                        if lo_tok.kind != TokenKind::Number {
                            return Err(TcError::MmioExpectedLowBit { span: place_span });
                        }
                        let lo = parse_u32_any(&slice[lo_tok.span.start..lo_tok.span.end])
                            .ok_or(TcError::MmioBadLowBit { span: place_span })?;
                        let mut hi = lo;
                        let mut probe3 = lex;
                        if probe3.next().kind == TokenKind::PunctDblDot {
                            let hi_tok = probe3.next();
                            if hi_tok.kind != TokenKind::Number {
                                return Err(TcError::MmioExpectedHighBit { span: place_span });
                            }
                            hi = parse_u32_any(&slice[hi_tok.span.start..hi_tok.span.end])
                                .ok_or(TcError::MmioBadHighBit { span: place_span })?;
                            lex = probe3;
                        }
                        let hi = hi.max(lo);
                        if hi >= (width * 8) {
                            return Err(TcError::MmioFieldBitRange { span: place_span });
                        }

                        let fty_tok = lex.next();
                        if fty_tok.kind != TokenKind::Ident {
                            return Err(TcError::MmioExpectedFieldType { span: place_span });
                        }
                        let Some(field_ty) =
                            TypeAtom::new(&slice[fty_tok.span.start..fty_tok.span.end])
                        else {
                            return Err(TcError::TypeParseFailed { span: place_span });
                        };

                        let faccess_tok = lex.next();
                        if faccess_tok.kind != TokenKind::Ident {
                            return Err(TcError::MmioExpectedFieldAccess { span: place_span });
                        }
                        let faccess =
                            parse_access_mode(&slice[faccess_tok.span.start..faccess_tok.span.end])
                                .ok_or(TcError::MmioInvalidFieldAccess { span: place_span })?;

                        if let Some(want) = want_field {
                            if fname == want && field_found.is_none() {
                                field_found = Some(MmioFieldInfo {
                                    name: fname,
                                    lo: lo.min(255) as u8,
                                    hi: hi.min(255) as u8,
                                    ty: field_ty,
                                    access: faccess,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if reg_name != want_reg {
            continue;
        }

        if let Some(_want) = want_field {
            if field_found.is_none() {
                return Err(TcError::MmioFieldNotFound { span: place_span });
            }
            return Ok(Some((
                MmioRegInfo {
                    offset,
                    reg: reg_name,
                    reg_ty,
                    access,
                    volatile,
                    array_len,
                },
                field_found,
            )));
        }
        return Ok(Some((
            MmioRegInfo {
                offset,
                reg: reg_name,
                reg_ty,
                access,
                volatile,
                array_len,
            },
            None,
        )));
    }
}

/// Construct a PlacePath from a qualified name (e.g. `gpio.OUT_SET`)
/// where the caller already has the resolved byte slice and span.
/// The root is set to the first segment; remaining segments become Field steps.
pub fn qualname_to_placepath(name: &[u8], name_span: Span) -> PlacePath {
    let mut steps: FixedVec<Step, 8> = FixedVec::new();
    // Find the first dot — root is everything before it.
    let first_dot = name.iter().position(|&b| b == b'.');
    let (root_full, seg_off_init) = if let Some(dot_pos) = first_dot {
        (
            Span::new(name_span.start, name_span.start + dot_pos),
            dot_pos + 1,
        )
    } else {
        // No dots: the entire name is the root, no steps.
        return PlacePath {
            root: name_span,
            steps,
            full: name_span,
        };
    };
    // Remaining segments after the first dot become Field steps.
    let mut i = seg_off_init;
    let mut seg_off = seg_off_init;
    while i <= name.len() {
        if i == name.len() || name[i] == b'.' {
            if i > seg_off {
                if let Some(atom) = TypeAtom::new(&name[seg_off..i]) {
                    let _ = steps.push(Step::Field(atom));
                }
            }
            seg_off = i + 1;
        }
        i += 1;
    }
    PlacePath {
        root: root_full,
        steps,
        full: name_span,
    }
}

pub fn resolve_mmio_place(
    db: &MmioDb,
    src: &[u8],
    place: &PlacePath,
    place_span: Span,
) -> Result<Option<MmioResolved>, TcError> {
    // PlacePath root is the MMIO instance name.
    let root_bytes = slice_span(src, place.root);
    let Some(inst_name) = TypeAtom::new(root_bytes) else {
        return Ok(None);
    };
    let Some(inst) = find_instance(db, inst_name) else {
        return Ok(None);
    };
    let Some(map_decl) = find_map_decl(db, inst.map) else {
        return Err(TcError::MmioMapNotFound { span: place_span });
    };

    // Walk the steps to extract register and optional field/index.
    // Expected pattern: root.Field(reg)[.Index(n)][.Field(field)]
    let step_len = place.steps.len();
    if step_len < 1 || step_len > 3 {
        return Err(TcError::MmioPlaceTooDeep { span: place_span });
    }

    // Step 0 must be a Field: the register name.
    let reg_step = place.steps.get(0).expect("step_len >= 1");
    let (reg_name, reg_idx) = match *reg_step {
        Step::Field(f) => {
            // Register name, possibly followed by an Index.
            let idx = if step_len >= 2 {
                match *place.steps.get(1).expect("step_len >= 2") {
                    Step::Index(n) => Some(n),
                    Step::DynamicIndex(_) => {
                        return Err(TcError::MmioArrayIndexNonArray { span: place_span })
                    }
                    _ => None,
                }
            } else {
                None
            };
            (f, idx)
        }
        _ => return Err(TcError::MmioRegNotFound { span: place_span }),
    };

    // Determine step offset for the field (if any).
    // After reg_name [+ Index], the next step (if any) is a Field.
    let field_offset = if reg_idx.is_some() { 2 } else { 1 };
    let want_field = if step_len > field_offset {
        match *place
            .steps
            .get(field_offset)
            .expect("step_len > field_offset")
        {
            Step::Field(f) => Some(f),
            _ => return Err(TcError::MmioFieldNotFound { span: place_span }),
        }
    } else {
        None
    };

    let reg_info = scan_regmap_for_reg(
        src,
        map_decl.body,
        reg_name,
        want_field,
        place_span,
        map_decl.name,
    )?;
    let Some((reg_info, field_info)) = reg_info else {
        return Err(TcError::MmioRegNotFound { span: place_span });
    };

    if let Some(n) = reg_info.array_len {
        if let Some(idx) = reg_idx {
            if idx >= n {
                return Err(TcError::MmioArrayIndexOob { span: place_span });
            }
        }
    } else if reg_idx.is_some() {
        return Err(TcError::MmioArrayIndexNonArray { span: place_span });
    }

    if let Some(field) = field_info {
        let width = mmio_type_width_bytes(reg_info.reg_ty.as_bytes()).unwrap_or(1) as u64;
        let idx = reg_idx.unwrap_or(0) as u64;
        let (aperture, offset) = place_aperture_offset(inst, reg_info.offset, idx, width);
        let meta = reg_access_meta(db, inst.board.unwrap_or(inst.name), reg_info.offset);
        Ok(Some(MmioResolved::Field(MmioResolvedField {
            map: map_decl.name,
            reg: reg_info.reg,
            reg_ty: reg_info.reg_ty,
            reg_access: reg_info.access,
            field,
            volatile: reg_info.volatile,
            aperture,
            offset,
            array_len: reg_info.array_len,
            meta,
            place_span,
        })))
    } else {
        let width = mmio_type_width_bytes(reg_info.reg_ty.as_bytes()).unwrap_or(1) as u64;
        let idx = reg_idx.unwrap_or(0) as u64;
        let (aperture, offset) = place_aperture_offset(inst, reg_info.offset, idx, width);
        let meta = reg_access_meta(db, inst.board.unwrap_or(inst.name), reg_info.offset);
        Ok(Some(MmioResolved::Reg(MmioResolvedReg {
            map: map_decl.name,
            reg: reg_info.reg,
            reg_ty: reg_info.reg_ty,
            access: reg_info.access,
            volatile: reg_info.volatile,
            aperture,
            offset,
            array_len: reg_info.array_len,
            meta,
            place_span,
        })))
    }
}

/// The aperture-relative place of a register access (P4): the aperture id and the
/// byte offset = instance base offset + register offset + array index stride.
fn place_aperture_offset(inst: MmioInstance, reg_offset: u32, idx: u64, width: u64) -> (u16, u32) {
    match inst.base {
        InstanceBase::Symbolic {
            aperture,
            base_offset,
        } => {
            let offset = base_offset
                .wrapping_add(reg_offset)
                .wrapping_add(idx.wrapping_mul(width) as u32);
            (aperture, offset as u32)
        }
        InstanceBase::Raw(base_addr) => {
            // Legacy raw path: aperture 0 is the module's single raw aperture and
            // the offset is the absolute address (a board-less compile — the
            // verifier treats aperture 0 as size-bounded by the descriptor,
            // which never reaches here; this path is unit-test-only).
            let _ = base_addr;
            (0, base_addr as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::vec::Vec;
    use frontend::parse::{DeclAst, RegMapInstanceAst};

    fn push_instance(
        src: &mut Vec<u8>,
        instances: &mut FixedVec<RegMapInstanceAst, 64>,
        name: &str,
        base_addr: &str,
    ) {
        let ns = src.len();
        src.extend_from_slice(name.as_bytes());
        let name_span = Span::new(ns, src.len());
        src.push(b'\n');
        let ms = src.len();
        src.extend_from_slice(b"map");
        let map_span = Span::new(ms, src.len());
        src.push(b'\n');
        let bs = src.len();
        src.extend_from_slice(base_addr.as_bytes());
        let base_span = Span::new(bs, src.len());
        src.push(b'\n');
        instances
            .push(RegMapInstanceAst {
                name: name_span,
                map: map_span,
                base_addr: base_span,
                board_instance: None,
            })
            .unwrap();
    }

    fn push_regmap(src: &mut Vec<u8>, decls: &mut FixedVec<DeclAst, 256>, name: &str) {
        let ns = src.len();
        src.extend_from_slice(name.as_bytes());
        let name_span = Span::new(ns, src.len());
        src.push(b'\n');
        let bs = src.len();
        src.extend_from_slice(b"0x00 REG u32 rw\n");
        let body_span = Span::new(bs, src.len());
        decls.push(DeclAst {
            kind: DeclKind::RegisterMap,
            name: name_span,
            sig: None,
            attrs: FixedVec::new(),
            body: Some(body_span),
            requires: None,
            ensures: None,
            cap_set: None,
            effect_bits: 0,
            effect_net: 0,
            effect_high: 0,
            has_explicit_performs: false,
        })
        .unwrap();
    }

    fn module(src: &[u8], instances: FixedVec<RegMapInstanceAst, 64>, decls: FixedVec<DeclAst, 256>) -> ModuleAst {
        ModuleAst {
            name: Span::new(0, 4.min(src.len())),
            imports: FixedVec::new(),
            exports: FixedVec::new(),
            decls,
            has_export_stmt: false,
            subtypes: FixedVec::new(),
            instances,
            structs: FixedVec::new(),
            enums: FixedVec::new(),
        }
    }

    #[test]
    fn malformed_base_address_is_an_error() {
        let mut src = Vec::new();
        let mut instances = FixedVec::new();
        push_instance(&mut src, &mut instances, "gpio", "not-a-number");
        let m = module(&src, instances, FixedVec::new());
        let err = build_mmio_db(&m, &src, None).map(|_| ()).unwrap_err();
        assert_eq!(err.code(), 3638, "malformed base addr must be MmioAddrInvalid");
    }

    #[test]
    fn seventeenth_register_map_is_capacity_error() {
        let mut src = Vec::new();
        let mut decls = FixedVec::new();
        for i in 0..17 {
            push_regmap(&mut src, &mut decls, &format!("m{}", i));
        }
        let m = module(&src, FixedVec::new(), decls);
        let err = build_mmio_db(&m, &src, None).map(|_| ()).unwrap_err();
        assert_eq!(err.code(), 3636, "17th map must be MmioMapCapacityExceeded");
    }

    #[test]
    fn sixteen_register_maps_and_sixty_four_instances_are_accepted() {
        let mut src = Vec::new();
        let mut instances = FixedVec::new();
        for i in 0..64 {
            push_instance(&mut src, &mut instances, &format!("i{}", i), "0x1000");
        }
        let mut decls = FixedVec::new();
        for i in 0..16 {
            push_regmap(&mut src, &mut decls, &format!("m{}", i));
        }
        let m = module(&src, instances, decls);
        assert!(build_mmio_db(&m, &src, None).is_ok(), "limits are inclusive");
    }

    /// Build a compiled descriptor with a single device/register.
    fn desc_with_write_kind(kind: u8) -> codegen_core::compiled_desc::CompiledDescriptor {
        use codegen_core::compiled_desc::{
            CompiledDevice, CompiledDescriptor, CompiledRegister, COMPILED_DESC_DEVICE_CAP,
            COMPILED_DESC_REGISTER_CAP, COMPILED_DESC_APERTURE_CAP,
        };
        use codegen_core::target::MmioApertureSpec;
        let mut regs = [CompiledRegister::EMPTY; COMPILED_DESC_REGISTER_CAP];
        regs[0] = CompiledRegister {
            offset: 0x00,
            name: ir::Atom::new(b"STATUS").unwrap(),
            width: 32,
            access: 1,
            write_kind: kind,
            read_kind: 0,
            atomic_max: 32,
            mask: 0,
            reset: 0,
            barrier: 0,
            interrupt: 0xFFFF,
            irq: 0xFFFF,
        };
        let mut devs = [CompiledDevice::EMPTY; COMPILED_DESC_DEVICE_CAP];
        devs[0] = CompiledDevice {
            map: ir::Atom::new(b"Strategy").unwrap(),
            instance: ir::Atom::new(b"strategy").unwrap(),
            aperture: 0,
            base_offset: 0,
            registers: regs,
            register_count: 1,
        };
        CompiledDescriptor {
            apertures: [MmioApertureSpec::EMPTY; COMPILED_DESC_APERTURE_CAP],
            aperture_count: 0,
            devices: devs,
            device_count: 1,
            platform_hash: 0,
        }
    }

    #[test]
    fn e3646_unknown_register_kind_is_refused() {
        // A descriptor written against a newer registry carries a kind this
        // compiler does not know; it must be refused (E3646), not silently
        // half-understood (forward-compat refusal).
        for kind in [99u8, 100, 255] {
            let desc = desc_with_write_kind(kind);
            let inst = TypeAtom::new(b"strategy").unwrap();
            let res = descriptor_instance_meta(&desc, inst);
            assert!(
                matches!(res, Err(TcError::MmioUnknownRegisterKind { .. })),
                "write_kind={kind} must be E3646"
            );
        }
        let desc = desc_with_write_kind(0); // REG_WRITE_PLAIN
        let inst = TypeAtom::new(b"strategy").unwrap();
        let rows = descriptor_instance_meta(&desc, inst).expect("known kinds accepted");
        assert_eq!(rows.iter().count(), 1);
        assert_eq!(rows.iter().next().unwrap().meta.write_kind, ir::WriteKind::Plain);
    }
}
