//! The `Peripheral` boundary (PLAN-WASM-1 I-1/§7.3): the sole
//! nondeterminism source in a run. P2 rejects MMIO modules at compile time,
//! so this trait is the *contract* (mirroring `verifier::mem`'s recorded-
//! history analogue); P4 ships the scripted device implementation and the
//! MMIO op arms. `Disabled` is the honest no-device implementation: it is an
//! error to reach it, and the engine reports E9152 when it happens.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeriphError {
    /// No device is registered for this aperture (diag E9152).
    UnboundAperture,
}

/// The device-access boundary (finding: `aperture` is a u16, `off` a u32,
/// `width` a byte count; §Q13 — a scripted device is a function of its own
/// access history, deterministic).
pub trait Peripheral {
    fn read(&mut self, aperture: u16, off: u32, width: u8) -> Result<u64, PeriphError>;
    fn write(&mut self, aperture: u16, off: u32, width: u8, val: u64) -> Result<(), PeriphError>;
}

/// The P2 device set: no apertures bound, every access is an error (E9152).
/// P4 replaces this with the aperture-keyed scripted device.
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
