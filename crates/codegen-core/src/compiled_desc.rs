//! Compiled platform descriptor — the runtime-side form (apertures + devices +
//! hash).
//!
//! Produced by `tyu` from a validated descriptor (design doc §5.3/§5.8) and
//! consumed by `langc` (P3: descriptor-sourced aperture sizes; P4: symbolic
//! `board.<instance>` resolution) and later the loader (P6: platform
//! binding). It lives in `codegen-core` so the producer and every consumer
//! share one encoder/decoder — the format is never reimplemented in a
//! consumer crate.
//!
//! Wire format version 2 (little-endian):
//! ```text
//! 0  4   magic b"TYDP"
//! 4  1   format version (2)
//! 5  8   platform_hash (u64 LE)
//! 13 1   aperture_count
//! 14 ..  per aperture:
//!           u16 id
//!           u8  name_len
//!           [u8; name_len] name
//!           u8  kind (0 = bus, 1 = emulated)
//!           u64 base (0 = none / link-time)
//!           u32 size
//!       then device_count (u8)
//!       then per device:
//!           u8  map_len
//!           [u8; map_len] map
//!           u8  instance_len
//!           [u8; instance_len] instance
//!           u16 aperture
//!           u32 base_offset
//!           u8  register_count
//!           per register:
//!             u32 offset
//!             u8  name_len
//!             [u8; name_len] name
//!             u8  width
//!             u8  access (0=ro 1=wo 2=rw)
//!             u8  write_kind (0=plain 1=w1s 2=w1c)
//!             u8  read_kind (0=plain 1=effectful)
//!             u8  atomic_max
//!             u64 mask
//!             u64 reset
//!             u8  barrier (0=none 1=before 2=after 3=both)
//! ```

use core::fmt;

use super::target::{MmioApertureKind, MmioApertureSpec, RelocIsa};

/// Maximum number of apertures a compiled descriptor may carry (matches the
/// loader's aperture-use table cap, design doc §5.5).
pub const COMPILED_DESC_APERTURE_CAP: usize = 8;

/// Maximum number of devices.
pub const COMPILED_DESC_DEVICE_CAP: usize = 16;

/// Maximum number of registers per device.
pub const COMPILED_DESC_REGISTER_CAP: usize = 32;

/// Maximum serialized size (16 devices × 32 registers × ~62 B + apertures).
pub const COMPILED_DESC_MAX_BYTES: usize = 32 * 1024;

const MAGIC: &[u8; 4] = b"TYDP";
const FORMAT_VER: u8 = 4;

/// Register access discriminants (D-3 fields, shared with the descriptor
/// model's enum ordering — never renumber).
pub const REG_ACCESS_RO: u8 = 0;
pub const REG_ACCESS_WO: u8 = 1;
pub const REG_ACCESS_RW: u8 = 2;
pub const REG_WRITE_PLAIN: u8 = 0;
pub const REG_WRITE_W1S: u8 = 1;
pub const REG_WRITE_W1C: u8 = 2;
pub const REG_READ_PLAIN: u8 = 0;
pub const REG_READ_EFFECTFUL: u8 = 1;
pub const REG_BARRIER_NONE: u8 = 0;
pub const REG_BARRIER_BEFORE: u8 = 1;
pub const REG_BARRIER_AFTER: u8 = 2;
pub const REG_BARRIER_BOTH: u8 = 3;

/// A single descriptor register row with its declared access semantics (D-3).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledRegister {
    pub offset: u32,
    pub name: ir::Atom,
    pub width: u8,
    pub access: u8,
    pub write_kind: u8,
    pub read_kind: u8,
    pub atomic_max: u8,
    pub mask: u64,
    pub reset: u64,
    pub barrier: u8,
    /// Datasheet NVIC IRQ number for an `@interrupt` binding (P8).
    /// `0xFFFF` = none.
    pub interrupt: u16,
    /// Datasheet interrupt request number (P8). `0xFFFF` = none.
    pub irq: u16,
}

impl CompiledRegister {
    pub const EMPTY: Self = Self {
        offset: 0,
        name: ir::AT_EMPTY,
        width: 0,
        access: 0,
        write_kind: 0,
        read_kind: 0,
        atomic_max: 0,
        mask: 0,
        reset: 0,
        barrier: 0,
        interrupt: 0xFFFF,
        irq: 0xFFFF,
    };
}

/// A board-exposed device instance (`board.<instance>`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledDevice {
    pub map: ir::Atom,
    pub instance: ir::Atom,
    pub aperture: u16,
    pub base_offset: u32,
    pub registers: [CompiledRegister; COMPILED_DESC_REGISTER_CAP],
    pub register_count: usize,
}

impl CompiledDevice {
    pub const EMPTY: Self = Self {
        map: ir::AT_EMPTY,
        instance: ir::AT_EMPTY,
        aperture: 0,
        base_offset: 0,
        registers: [CompiledRegister::EMPTY; COMPILED_DESC_REGISTER_CAP],
        register_count: 0,
    };

    /// The registers as a slice.
    pub fn registers(&self) -> &[CompiledRegister] {
        &self.registers[..self.register_count]
    }
}

/// The compiled, loadable form of a platform descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompiledDescriptor {
    pub apertures: [MmioApertureSpec; COMPILED_DESC_APERTURE_CAP],
    pub aperture_count: usize,
    pub devices: [CompiledDevice; COMPILED_DESC_DEVICE_CAP],
    pub device_count: usize,
    pub platform_hash: u64,
}

impl CompiledDescriptor {
    /// The apertures as a slice of length `aperture_count`.
    pub fn apertures(&self) -> &[MmioApertureSpec] {
        &self.apertures[..self.aperture_count]
    }

    /// The devices as a slice of length `device_count`.
    pub fn devices(&self) -> &[CompiledDevice] {
        &self.devices[..self.device_count]
    }

    /// Look up a device by board instance name.
    pub fn device(&self, instance: ir::Atom) -> Option<&CompiledDevice> {
        self.devices().iter().find(|d| d.instance == instance)
    }
}

impl Default for CompiledDescriptor {
    fn default() -> Self {
        Self {
            apertures: [MmioApertureSpec::EMPTY; COMPILED_DESC_APERTURE_CAP],
            aperture_count: 0,
            devices: [CompiledDevice::EMPTY; COMPILED_DESC_DEVICE_CAP],
            device_count: 0,
            platform_hash: 0,
        }
    }
}

/// Errors from encoding/decoding/validating a compiled descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompiledDescError {
    BadMagic,
    BadVersion,
    TooManyApertures,
    TooManyDevices,
    TooManyRegisters,
    BufferTooSmall,
    Truncated,
    BadNameLen,
    BadKind,
    ApertureSizeZero,
    ApertureIdGap,
    ApertureOverlap,
    DeviceUnknownAperture,
    DeviceDuplicate,
    TrailingBytes,
}

impl CompiledDescError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadMagic => "bad compiled-descriptor magic",
            Self::BadVersion => "unsupported compiled-descriptor format version",
            Self::TooManyApertures => "too many apertures (> 8)",
            Self::TooManyDevices => "too many devices (> 16)",
            Self::TooManyRegisters => "too many registers in a device (> 32)",
            Self::BufferTooSmall => "output buffer too small for compiled descriptor",
            Self::Truncated => "compiled descriptor truncated",
            Self::BadNameLen => "aperture name length exceeds 32 bytes",
            Self::BadKind => "unknown aperture kind",
            Self::ApertureSizeZero => "aperture size must be > 0",
            Self::ApertureIdGap => "aperture ids must be unique and densely numbered from 0",
            Self::ApertureOverlap => "bus apertures overlap",
            Self::DeviceUnknownAperture => "device references an unknown aperture id",
            Self::DeviceDuplicate => "device (map, instance) identity must be unique",
            Self::TrailingBytes => "trailing bytes after compiled descriptor",
        }
    }
}

impl fmt::Display for CompiledDescError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Encode `cd` into `out`, returning the number of bytes written.
pub fn encode_compiled_desc(
    cd: &CompiledDescriptor,
    out: &mut [u8],
) -> Result<usize, CompiledDescError> {
    let needed = serialized_len(cd)?;
    if out.len() < needed {
        return Err(CompiledDescError::BufferTooSmall);
    }
    let mut p = 0usize;
    out[p..p + 4].copy_from_slice(MAGIC);
    p += 4;
    out[p] = FORMAT_VER;
    p += 1;
    out[p..p + 8].copy_from_slice(&cd.platform_hash.to_le_bytes());
    p += 8;
    out[p] = cd.aperture_count as u8;
    p += 1;
    for w in cd.apertures() {
        out[p..p + 2].copy_from_slice(&w.id.to_le_bytes());
        p += 2;
        let name = w.name.as_bytes();
        out[p] = name.len() as u8;
        p += 1;
        out[p..p + name.len()].copy_from_slice(name);
        p += name.len();
        out[p] = aperture_kind_disc(w.kind);
        p += 1;
        out[p] = reloc_isa_disc(w.reloc_isa);
        p += 1;
        out[p..p + 8].copy_from_slice(&w.base.unwrap_or(0).to_le_bytes());
        p += 8;
        out[p..p + 4].copy_from_slice(&w.size.to_le_bytes());
        p += 4;
    }
    out[p] = cd.device_count as u8;
    p += 1;
    for d in cd.devices() {
        let map = d.map.as_bytes();
        out[p] = map.len() as u8;
        p += 1;
        out[p..p + map.len()].copy_from_slice(map);
        p += map.len();
        let instance = d.instance.as_bytes();
        out[p] = instance.len() as u8;
        p += 1;
        out[p..p + instance.len()].copy_from_slice(instance);
        p += instance.len();
        out[p..p + 2].copy_from_slice(&d.aperture.to_le_bytes());
        p += 2;
        out[p..p + 4].copy_from_slice(&d.base_offset.to_le_bytes());
        p += 4;
        out[p] = d.register_count as u8;
        p += 1;
        for r in d.registers() {
            out[p..p + 4].copy_from_slice(&r.offset.to_le_bytes());
            p += 4;
            let name = r.name.as_bytes();
            out[p] = name.len() as u8;
            p += 1;
            out[p..p + name.len()].copy_from_slice(name);
            p += name.len();
            out[p] = r.width;
            p += 1;
            out[p] = r.access;
            p += 1;
            out[p] = r.write_kind;
            p += 1;
            out[p] = r.read_kind;
            p += 1;
            out[p] = r.atomic_max;
            p += 1;
            out[p..p + 8].copy_from_slice(&r.mask.to_le_bytes());
            p += 8;
            out[p..p + 8].copy_from_slice(&r.reset.to_le_bytes());
            p += 8;
            out[p] = r.barrier;
            p += 1;
            out[p..p + 2].copy_from_slice(&r.interrupt.to_le_bytes());
            p += 2;
            out[p..p + 2].copy_from_slice(&r.irq.to_le_bytes());
            p += 2;
        }
    }
    Ok(p)
}

/// Decode a compiled descriptor from `bytes` (structural checks only;
/// semantic validation is `validate_compiled_desc`).
pub fn decode_compiled_desc(bytes: &[u8]) -> Result<CompiledDescriptor, CompiledDescError> {
    let mut p = 0usize;
    if bytes.len() < 14 {
        return Err(CompiledDescError::Truncated);
    }
    if &bytes[p..p + 4] != MAGIC {
        return Err(CompiledDescError::BadMagic);
    }
    p += 4;
    if bytes[p] != FORMAT_VER {
        return Err(CompiledDescError::BadVersion);
    }
    p += 1;
    let mut hash_le = [0u8; 8];
    hash_le.copy_from_slice(&bytes[p..p + 8]);
    let platform_hash = u64::from_le_bytes(hash_le);
    p += 8;
    let aperture_count = bytes[p] as usize;
    p += 1;
    if aperture_count > COMPILED_DESC_APERTURE_CAP {
        return Err(CompiledDescError::TooManyApertures);
    }
    let mut apertures = [MmioApertureSpec::EMPTY; COMPILED_DESC_APERTURE_CAP];
    for slot in apertures.iter_mut().take(aperture_count) {
        if bytes.len() < p + 2 {
            return Err(CompiledDescError::Truncated);
        }
        let mut id_le = [0u8; 2];
        id_le.copy_from_slice(&bytes[p..p + 2]);
        let id = u16::from_le_bytes(id_le);
        p += 2;
        if p >= bytes.len() {
            return Err(CompiledDescError::Truncated);
        }
        let name_len = bytes[p] as usize;
        p += 1;
        if name_len > 32 || bytes.len() < p + name_len {
            return Err(CompiledDescError::BadNameLen);
        }
        let name = ir::Atom::new(&bytes[p..p + name_len]).ok_or(CompiledDescError::BadNameLen)?;
        p += name_len;
        if p >= bytes.len() {
            return Err(CompiledDescError::Truncated);
        }
        let kind = match bytes[p] {
            0 => MmioApertureKind::Bus,
            1 => MmioApertureKind::Emulated,
            _ => return Err(CompiledDescError::BadKind),
        };
        p += 1;
        let reloc_isa = match bytes[p] {
            0 => None,
            1 => Some(RelocIsa::ArmThumbLdrLiteral),
            2 => Some(RelocIsa::RiscVHi20Lo12),
            _ => return Err(CompiledDescError::BadKind),
        };
        p += 1;
        if bytes.len() < p + 12 {
            return Err(CompiledDescError::Truncated);
        }
        let mut base_le = [0u8; 8];
        base_le.copy_from_slice(&bytes[p..p + 8]);
        let base_raw = u64::from_le_bytes(base_le);
        p += 8;
        let mut size_le = [0u8; 4];
        size_le.copy_from_slice(&bytes[p..p + 4]);
        let size = u32::from_le_bytes(size_le);
        p += 4;
        *slot = MmioApertureSpec {
            id,
            name,
            kind,
            base: (base_raw != 0).then_some(base_raw),
            size,
            reloc_isa,
        };
    }

    if p >= bytes.len() {
        return Err(CompiledDescError::Truncated);
    }
    let device_count = bytes[p] as usize;
    p += 1;
    if device_count > COMPILED_DESC_DEVICE_CAP {
        return Err(CompiledDescError::TooManyDevices);
    }
    let mut devices = [CompiledDevice::EMPTY; COMPILED_DESC_DEVICE_CAP];
    for slot in devices.iter_mut().take(device_count) {
        if p >= bytes.len() {
            return Err(CompiledDescError::Truncated);
        }
        let map_len = bytes[p] as usize;
        p += 1;
        if map_len > 32 || bytes.len() < p + map_len {
            return Err(CompiledDescError::BadNameLen);
        }
        let map = ir::Atom::new(&bytes[p..p + map_len]).ok_or(CompiledDescError::BadNameLen)?;
        p += map_len;
        if p >= bytes.len() {
            return Err(CompiledDescError::Truncated);
        }
        let instance_len = bytes[p] as usize;
        p += 1;
        if instance_len > 32 || bytes.len() < p + instance_len {
            return Err(CompiledDescError::BadNameLen);
        }
        let instance =
            ir::Atom::new(&bytes[p..p + instance_len]).ok_or(CompiledDescError::BadNameLen)?;
        p += instance_len;
        if bytes.len() < p + 6 {
            return Err(CompiledDescError::Truncated);
        }
        let mut aperture_le = [0u8; 2];
        aperture_le.copy_from_slice(&bytes[p..p + 2]);
        let aperture = u16::from_le_bytes(aperture_le);
        p += 2;
        let mut off_le = [0u8; 4];
        off_le.copy_from_slice(&bytes[p..p + 4]);
        let base_offset = u32::from_le_bytes(off_le);
        p += 4;
        let register_count = bytes[p] as usize;
        p += 1;
        if register_count > COMPILED_DESC_REGISTER_CAP {
            return Err(CompiledDescError::TooManyRegisters);
        }
        let mut registers = [CompiledRegister::EMPTY; COMPILED_DESC_REGISTER_CAP];
        for rslot in registers.iter_mut().take(register_count) {
            if bytes.len() < p + 4 {
                return Err(CompiledDescError::Truncated);
            }
            let mut offset_le = [0u8; 4];
            offset_le.copy_from_slice(&bytes[p..p + 4]);
            let offset = u32::from_le_bytes(offset_le);
            p += 4;
            if p >= bytes.len() {
                return Err(CompiledDescError::Truncated);
            }
            let name_len = bytes[p] as usize;
            p += 1;
            if name_len > 32 || bytes.len() < p + name_len {
                return Err(CompiledDescError::BadNameLen);
            }
            let name = ir::Atom::new(&bytes[p..p + name_len]).ok_or(CompiledDescError::BadNameLen)?;
            p += name_len;
            if bytes.len() < p + 5 {
                return Err(CompiledDescError::Truncated);
            }
            let width = bytes[p];
            p += 1;
            let access = bytes[p];
            p += 1;
            let write_kind = bytes[p];
            p += 1;
            let read_kind = bytes[p];
            p += 1;
            let atomic_max = bytes[p];
            p += 1;
            if bytes.len() < p + 16 {
                return Err(CompiledDescError::Truncated);
            }
            let mut mask_le = [0u8; 8];
            mask_le.copy_from_slice(&bytes[p..p + 8]);
            let mask = u64::from_le_bytes(mask_le);
            p += 8;
            let mut reset_le = [0u8; 8];
            reset_le.copy_from_slice(&bytes[p..p + 8]);
            let reset = u64::from_le_bytes(reset_le);
            p += 8;
            let barrier = bytes[p];
            p += 1;
            if bytes.len() < p + 4 {
                return Err(CompiledDescError::Truncated);
            }
            let interrupt = u16::from_le_bytes(bytes[p..p + 2].try_into().unwrap());
            p += 2;
            let irq = u16::from_le_bytes(bytes[p..p + 2].try_into().unwrap());
            p += 2;
            *rslot = CompiledRegister {
                offset,
                name,
                width,
                access,
                write_kind,
                read_kind,
                atomic_max,
                mask,
                reset,
                barrier,
                interrupt,
                irq,
            };
        }
        *slot = CompiledDevice {
            map,
            instance,
            aperture,
            base_offset,
            registers,
            register_count,
        };
    }

    if bytes.len() != p {
        return Err(CompiledDescError::TrailingBytes);
    }
    Ok(CompiledDescriptor {
        apertures,
        aperture_count,
        devices,
        device_count,
        platform_hash,
    })
}

/// Semantic validation of a decoded compiled descriptor: aperture size > 0,
/// ids unique and densely numbered from 0, no overlapping bus apertures, every
/// device referencing a declared aperture, and unique device identity.
pub fn validate_compiled_desc(cd: &CompiledDescriptor) -> Result<(), CompiledDescError> {
    let mut ids = [0u16; COMPILED_DESC_APERTURE_CAP];
    let mut id_count = 0usize;
    for w in cd.apertures() {
        ids[id_count] = w.id;
        id_count += 1;
    }
    sort_u16(&mut ids[..id_count]);
    for (index, &id) in ids[..id_count].iter().enumerate() {
        if id as usize != index {
            return Err(CompiledDescError::ApertureIdGap);
        }
    }

    for w in cd.apertures() {
        if w.size == 0 {
            return Err(CompiledDescError::ApertureSizeZero);
        }
    }

    let mut ranged = [(0u64, 0u64); COMPILED_DESC_APERTURE_CAP];
    let mut range_count = 0usize;
    for w in cd.apertures() {
        if let Some(b) = w.base {
            ranged[range_count] = (b, b.saturating_add(w.size as u64));
            range_count += 1;
        }
    }
    sort_ranges(&mut ranged[..range_count]);
    for pair in ranged[..range_count].windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(CompiledDescError::ApertureOverlap);
        }
    }

    let mut keys = [ir::AT_EMPTY; COMPILED_DESC_DEVICE_CAP];
    let mut key_count = 0usize;
    for d in cd.devices() {
        if cd.apertures().iter().all(|w| w.id != d.aperture) {
            return Err(CompiledDescError::DeviceUnknownAperture);
        }
        keys[key_count] = d.instance;
        key_count += 1;
    }
    sort_atoms(&mut keys[..key_count]);
    for pair in keys[..key_count].windows(2) {
        if pair[0] == pair[1] {
            return Err(CompiledDescError::DeviceDuplicate);
        }
    }

    Ok(())
}

fn serialized_len(cd: &CompiledDescriptor) -> Result<usize, CompiledDescError> {
    if cd.aperture_count > COMPILED_DESC_APERTURE_CAP {
        return Err(CompiledDescError::TooManyApertures);
    }
    if cd.device_count > COMPILED_DESC_DEVICE_CAP {
        return Err(CompiledDescError::TooManyDevices);
    }
    let mut len = 15usize; // magic 4 + ver 1 + hash 8 + aperture_count 1 + device_count 1
    for w in cd.apertures() {
        len += 2 + 1 + w.name.as_bytes().len() + 1 + 1 + 8 + 4;
    }
    for d in cd.devices() {
        if d.register_count > COMPILED_DESC_REGISTER_CAP {
            return Err(CompiledDescError::TooManyRegisters);
        }
        len += 1 + d.map.as_bytes().len() + 1 + d.instance.as_bytes().len() + 2 + 4 + 1;
        for r in d.registers() {
            len += 4 + 1 + r.name.as_bytes().len() + 1 + 1 + 1 + 1 + 1 + 8 + 8 + 1;
        }
    }
    Ok(len)
}

fn sort_u16(v: &mut [u16]) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && v[j - 1] > v[j] {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

fn sort_ranges(v: &mut [(u64, u64)]) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && v[j - 1].0 > v[j].0 {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

fn sort_atoms(v: &mut [ir::Atom]) {
    for i in 1..v.len() {
        let mut j = i;
        while j > 0 && v[j - 1].as_bytes() > v[j].as_bytes() {
            v.swap(j - 1, j);
            j -= 1;
        }
    }
}

fn reloc_isa_disc(r: Option<RelocIsa>) -> u8 {
    match r {
        None => 0,
        Some(RelocIsa::ArmThumbLdrLiteral) => 1,
        Some(RelocIsa::RiscVHi20Lo12) => 2,
    }
}

fn aperture_kind_disc(k: MmioApertureKind) -> u8 {
    match k {
        MmioApertureKind::Bus => 0,
        MmioApertureKind::Emulated => 1,
    }
}

/// The board-declared capability of a aperture (P6, design doc §5.2/§5.8): the
/// union of its devices' register access semantics. `tyu` encodes this into the
/// board aperture table (`lmod::board_table`) at build time; the loader checks a
/// module's fused `access_mask` is a subset of it (E5223). Host-side only —
/// the device loads the precomputed bytes.
pub fn aperture_capability(cd: &CompiledDescriptor, aperture_id: u16) -> u8 {
    use ir::{ACCESS_EFFECTFUL_READ, ACCESS_READ, ACCESS_W1C, ACCESS_W1S, ACCESS_WRITE};
    let mut cap = 0u8;
    for d in cd.devices() {
        if d.aperture != aperture_id {
            continue;
        }
        for r in d.registers() {
            match r.access {
                REG_ACCESS_RO => cap |= ACCESS_READ,
                REG_ACCESS_WO => cap |= ACCESS_WRITE,
                REG_ACCESS_RW => cap |= ACCESS_READ | ACCESS_WRITE,
                _ => {}
            }
            match r.write_kind {
                REG_WRITE_W1S => cap |= ACCESS_W1S,
                REG_WRITE_W1C => cap |= ACCESS_W1C,
                _ => {}
            }
            if r.read_kind == REG_READ_EFFECTFUL {
                cap |= ACCESS_EFFECTFUL_READ;
            }
        }
    }
    cap
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use crate::target::{MmioApertureKind, MmioApertureSpec};
    use ir::Atom;

    #[test]
    fn capability_derives_from_devices() {
        use ir::{ACCESS_EFFECTFUL_READ, ACCESS_READ, ACCESS_W1C, ACCESS_WRITE};
        let mut cd = CompiledDescriptor::default();
        cd.aperture_count = 1;
        cd.apertures[0] = MmioApertureSpec {
            id: 0,
            name: Atom::new(b"apb").unwrap(),
            kind: MmioApertureKind::Bus,
            base: Some(0x4000_0000),
            size: 0x1_0000,
            reloc_isa: Some(crate::target::RelocIsa::ArmThumbLdrLiteral),
        };
        let mut dev = CompiledDevice::EMPTY;
        dev.aperture = 0;
        dev.register_count = 2;
        dev.registers[0] = CompiledRegister {
            access: REG_ACCESS_RW,
            write_kind: REG_WRITE_W1C,
            read_kind: REG_READ_PLAIN,
            ..CompiledRegister::EMPTY
        };
        dev.registers[1] = CompiledRegister {
            access: REG_ACCESS_RO,
            write_kind: REG_WRITE_PLAIN,
            read_kind: REG_READ_EFFECTFUL,
            ..CompiledRegister::EMPTY
        };
        cd.devices[0] = dev;
        cd.device_count = 1;
        let cap = aperture_capability(&cd, 0);
        assert_eq!(cap, ACCESS_READ | ACCESS_WRITE | ACCESS_W1C | ACCESS_EFFECTFUL_READ);
        assert_eq!(aperture_capability(&cd, 1), 0);
    }
}
