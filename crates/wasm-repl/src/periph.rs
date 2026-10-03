//! The `Peripheral` boundary (PLAN-WASM-1 I-1/§7.3): the sole
//! nondeterminism source in a run. A scripted device is a function of its own
//! access history — the concrete analogue of `ApertureMem`'s recorded stores
//! (`verifier/src/mem.rs`, §Q13).
//!
//! P2 shipped the *contract* ([`Peripheral`]) plus [`Disabled`] (the honest
//! no-device implementation: any access reports E9152). P4 adds the device
//! implementations:
//!
//! - [`ScriptedPeripheral`] — aperture-keyed, **ordered** action scripts
//!   (`reply_read`, `expect_write`); a wrong access order/value is a script
//!   failure (fail-fast), never an engine diagnostic (§P4); + an access log
//!   for traces.
//! - [`RamDevice`] — the emulated-aperture RAM device: writes persist, reads
//!   return them. The engine performs the `WriteKind` RMW against it, so the
//!   `mmio_*` fixture self-checks pin the engine's bit math end-to-end.

use alloc::vec::Vec;

/// Device-access failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeriphError {
    /// No device is registered for this aperture (diag E9152).
    UnboundAperture,
}

/// The device-access boundary (finding: `aperture` is a u16, `off` a u32,
/// `width` a byte count).
pub trait Peripheral {
    fn read(&mut self, aperture: u16, off: u32, width: u8) -> Result<u64, PeriphError>;
    fn write(&mut self, aperture: u16, off: u32, width: u8, val: u64) -> Result<(), PeriphError>;
}

/// The no-device set: no apertures bound; every access is an error (E9152).
pub struct Disabled;

impl Peripheral for Disabled {
    fn read(&mut self, _aperture: u16, _off: u32, _width: u8) -> Result<u64, PeriphError> {
        Err(PeriphError::UnboundAperture)
    }
    fn write(
        &mut self,
        _aperture: u16,
        _off: u32,
        _width: u8,
        _val: u64,
    ) -> Result<(), PeriphError> {
        Err(PeriphError::UnboundAperture)
    }
}

/// One recorded device access (the §7.4 periph event payload and the
/// harness's access-log pin).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Access {
    pub aperture: u16,
    pub off: u32,
    pub width: u8,
    pub val: u64,
    pub is_write: bool,
}

impl Access {
    /// The normalized periph-event token (`W{ap:x}:{off:x}:{width}:{val:x}`).
    pub fn token(&self) -> alloc::string::String {
        use alloc::format;
        format!(
            "{}{:x}:{:x}:{}:{:x}",
            if self.is_write { 'W' } else { 'R' },
            self.aperture,
            self.off,
            self.width,
            self.val
        )
    }
}

/// One ordered script step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptStep {
    /// The next read at `(aperture, off, width)` replies `reply`.
    Read {
        aperture: u16,
        off: u32,
        width: u8,
        reply: u64,
    },
    /// The next write at `(aperture, off, width)` MUST equal `val`.
    Write {
        aperture: u16,
        off: u32,
        width: u8,
        val: u64,
    },
}

/// Aperture-keyed, ordered action script + access log (P4 §value).
///
/// Reads reply the next [`ScriptStep::Read`]'s value; writes must match the
/// next [`ScriptStep::Write`] exactly. A mismatch (wrong aperture/off/width,
/// wrong order, wrong value) **panics** — a wrong script is a test/harness
/// failure, never an engine diagnostic. Accesses to apertures outside
/// `bound` report [`PeriphError::UnboundAperture`] (E9152).
pub struct ScriptedPeripheral {
    script: Vec<ScriptStep>,
    cursor: usize,
    bound: Vec<u16>,
    log: Vec<Access>,
}

impl ScriptedPeripheral {
    pub fn new(bound: &[u16], script: Vec<ScriptStep>) -> ScriptedPeripheral {
        ScriptedPeripheral {
            script,
            cursor: 0,
            bound: bound.to_vec(),
            log: Vec::new(),
        }
    }

    /// The recorded access history (trace payload; harness pin).
    pub fn log(&self) -> &[Access] {
        &self.log
    }

    /// The steps remaining (0 when the script was fully consumed).
    pub fn remaining(&self) -> usize {
        self.script.len() - self.cursor
    }

    fn check_bound(&self, aperture: u16) -> Result<(), PeriphError> {
        if self.bound.contains(&aperture) {
            Ok(())
        } else {
            Err(PeriphError::UnboundAperture)
        }
    }

    fn step_for(&mut self, is_write: bool) -> ScriptStep {
        // Panic-fail on exhaustion (a scripted run that stops early is a
        // wrong script — nothing left to confirm the remaining accesses).
        match self.script.get(self.cursor).copied() {
            Some(s) => {
                let ok_matches = if is_write {
                    matches!(s, ScriptStep::Write { .. })
                } else {
                    matches!(s, ScriptStep::Read { .. })
                };
                if !ok_matches {
                    panic!(
                        "script mismatch at step {}: expected a {}, got {:?}",
                        self.cursor,
                        if is_write { "write" } else { "read" },
                        s
                    );
                }
                self.cursor += 1;
                s
            }
            None => panic!(
                "script exhausted at {} but the engine issued a {}",
                self.cursor,
                if is_write { "write" } else { "read" }
            ),
        }
    }
}

impl Peripheral for ScriptedPeripheral {
    fn read(&mut self, aperture: u16, off: u32, width: u8) -> Result<u64, PeriphError> {
        self.check_bound(aperture)?;
        let ScriptStep::Read {
            aperture: ap,
            off: o,
            width: w,
            reply,
        } = self.step_for(false)
        else {
            unreachable!("step_for matched a Read")
        };
        if ap != aperture || o != off || w != width {
            panic!(
                "script mismatch at step {}: read ({ap},{o:#x},{w}) but the engine asked \
                 ({aperture},{off:#x},{width})",
                self.cursor - 1
            );
        }
        self.log.push(Access {
            aperture,
            off,
            width,
            val: reply,
            is_write: false,
        });
        Ok(reply)
    }

    fn write(&mut self, aperture: u16, off: u32, width: u8, val: u64) -> Result<(), PeriphError> {
        self.check_bound(aperture)?;
        let ScriptStep::Write {
            aperture: ap,
            off: o,
            width: w,
            val: expected,
        } = self.step_for(true)
        else {
            unreachable!("step_for matched a Write")
        };
        if ap != aperture || o != off || w != width || expected != val {
            panic!(
                "script mismatch at step {}: write ({ap},{o:#x},{w})={expected:#x} but the engine \
                 wrote ({aperture},{off:#x},{width})={val:#x}",
                self.cursor - 1
            );
        }
        self.log.push(Access {
            aperture,
            off,
            width,
            val,
            is_write: true,
        });
        Ok(())
    }
}

/// The emulated-aperture RAM device (P4 fixture side): an aperture-keyed,
/// sparse register file — writes persist, reads return the last written
/// value (zero before any write). This is the deterministic, memory-backed
/// device behind an `emulated` aperture; the engine's `WriteKind` RMW is
/// performed *against* this device (the engine computes the new value, the
/// device stores it), so the mmio fixture self-checks pin the full path.
pub struct RamDevice {
    bound: Vec<u16>,
    cells: Vec<(u16, u32, u8, u64)>,
    log: Vec<Access>,
}

impl RamDevice {
    pub fn new(bound: &[u16]) -> RamDevice {
        RamDevice {
            bound: bound.to_vec(),
            cells: Vec::new(),
            log: Vec::new(),
        }
    }

    /// The recorded access history (trace payload; harness pin).
    pub fn log(&self) -> &[Access] {
        &self.log
    }

    fn check_bound(&self, aperture: u16) -> Result<(), PeriphError> {
        if self.bound.contains(&aperture) {
            Ok(())
        } else {
            Err(PeriphError::UnboundAperture)
        }
    }
}

/// The register-cell width mask for a byte width.
pub fn width_mask(width: u8) -> u64 {
    match width {
        1 => 0xff,
        2 => 0xffff,
        4 => 0xffff_ffff,
        8 => u64::MAX,
        _ => 0,
    }
}

impl Peripheral for RamDevice {
    fn read(&mut self, aperture: u16, off: u32, width: u8) -> Result<u64, PeriphError> {
        self.check_bound(aperture)?;
        // A read reports the cell's stored value (masked to the read width).
        let val = self
            .cells
            .iter()
            .find(|(ap, o, _, _)| *ap == aperture && *o == off)
            .map(|&(_, _, _, v)| v & width_mask(width))
            .unwrap_or(0);
        self.log.push(Access {
            aperture,
            off,
            width,
            val,
            is_write: false,
        });
        Ok(val)
    }

    fn write(&mut self, aperture: u16, off: u32, width: u8, val: u64) -> Result<(), PeriphError> {
        self.check_bound(aperture)?;
        let masked = val & width_mask(width);
        match self
            .cells
            .iter_mut()
            .find(|(ap, o, _, _)| *ap == aperture && *o == off)
        {
            Some(cell) => cell.3 = masked,
            None => self.cells.push((aperture, off, width, masked)),
        }
        self.log.push(Access {
            aperture,
            off,
            width,
            val: masked,
            is_write: true,
        });
        Ok(())
    }
}
