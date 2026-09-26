//! Discoverable manual HIL targets for platform packs.
//!
//! These tests are ignored by default. They exist as named targets that can be
//! referenced from platform manifests and backed by recorded evidence.

/// Manual RP2350 HIL evidence target.
///
/// The platform pack references this target name in `[test].target`.
/// Placeholder target: real assertions come from recorded HIL evidence runs.
#[ignore = "manual HIL evidence target"]
#[test]
#[allow(clippy::assertions_on_constants)]
fn rp2350_manual_hil() {
    assert!(true);
}
