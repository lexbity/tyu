//! Set-payload cryptography (P7): the key roster, the set signing-key
//! registry, and the region-with-rollback protocol host-side implementation.
//!
//! This is the Rust side of the over-the-air set-payload loader path. It
//! implements the versioned wire format, the key roster, the set-table that
//! routes per-key and per-bit to different slots, and the region state machine
//! (`naked` / `provisioned` / `committing` / `rollback`) that the on-device
//! loader mirrors.
//!
//! Modules:
//! - [`registry`]: the set signing-key registry (`Roster`, `KeyRoster`,
//!   `sign_key_for_payload`, `sniff`/`peek`).
//! - [`keys`]: key selection (`register_key`, `select_key`,
//!   `select_best_sk0`) and the keys-section protocol.
//! - [`region`]: the region protocol (`RegionState`, `RegionLoader`) and the
//!   set-table routing.

pub mod keys;
pub mod region;
pub mod registry;