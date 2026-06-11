//! Trap-code to human-readable claim-text mapping.
//!
//! Covers the three error bands:
//! - Runtime trap codes (10, 20–24)
//! - Effect/context `50xx` band (5001–5040)
//! - Static stack-depth `51xx` band (5100, 5101, 5103)

/// Return the human-readable claim name for a trap code.
///
/// Returns `"UNKNOWN_TRAP_CODE"` for any code not in the registry.
pub fn claim_text(trap_code: u16) -> &'static str {
    match trap_code {
        // Runtime trap codes (crates/ir/src/lib.rs)
        10 => "STACK_OVERFLOW",
        20 => "CONTRACT_FAIL",
        21 => "SUBTYPE_FAIL",
        22 => "ASSERT_FAIL",
        23 => "UNREACHABLE",
        24 => "TASK_QUEUE_OVERFLOW",
        // Effect / context model 50xx band
        5001 => "E_SUSPEND_FORBIDDEN",
        5002 => "E_LOCK_NEST",
        5003 => "E_LOCK_STACK",
        5004 => "E_CAP_MISSING",
        5010 => "E_ISO_DUP",
        5011 => "E_ISO_DROP",
        5012 => "E_ISO_USE_AFTER_MOVE",
        5020 => "E_BORROW_ESCAPE",
        5030 => "E_ISR_STACK",
        5031 => "E_RESOURCE_SHARED_UNLOCKED",
        5040 => "E_DIVERGE_IN_BOUNDED",
        // Static stack-depth 51xx band
        5100 => "E_STACK_UNBOUNDED",
        5101 => "E_STACK_EXCEEDS_BUDGET",
        5103 => "E_STACK_QUOT_ERASED",
        _ => "UNKNOWN_TRAP_CODE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_trap_codes() {
        assert_eq!(claim_text(10), "STACK_OVERFLOW");
        assert_eq!(claim_text(20), "CONTRACT_FAIL");
        assert_eq!(claim_text(21), "SUBTYPE_FAIL");
        assert_eq!(claim_text(22), "ASSERT_FAIL");
        assert_eq!(claim_text(23), "UNREACHABLE");
        assert_eq!(claim_text(24), "TASK_QUEUE_OVERFLOW");
    }

    #[test]
    fn effect_band_codes() {
        assert_eq!(claim_text(5001), "E_SUSPEND_FORBIDDEN");
        assert_eq!(claim_text(5002), "E_LOCK_NEST");
        assert_eq!(claim_text(5003), "E_LOCK_STACK");
        assert_eq!(claim_text(5004), "E_CAP_MISSING");
        assert_eq!(claim_text(5010), "E_ISO_DUP");
        assert_eq!(claim_text(5011), "E_ISO_DROP");
        assert_eq!(claim_text(5012), "E_ISO_USE_AFTER_MOVE");
        assert_eq!(claim_text(5020), "E_BORROW_ESCAPE");
        assert_eq!(claim_text(5030), "E_ISR_STACK");
        assert_eq!(claim_text(5031), "E_RESOURCE_SHARED_UNLOCKED");
        assert_eq!(claim_text(5040), "E_DIVERGE_IN_BOUNDED");
    }

    #[test]
    fn stack_band_codes() {
        assert_eq!(claim_text(5100), "E_STACK_UNBOUNDED");
        assert_eq!(claim_text(5101), "E_STACK_EXCEEDS_BUDGET");
        assert_eq!(claim_text(5103), "E_STACK_QUOT_ERASED");
    }

    #[test]
    fn unknown_code_returns_fallback() {
        assert_eq!(claim_text(0), "UNKNOWN_TRAP_CODE");
        assert_eq!(claim_text(9999), "UNKNOWN_TRAP_CODE");
        assert_eq!(claim_text(65535), "UNKNOWN_TRAP_CODE");
    }

    #[test]
    fn every_registered_code_has_text() {
        let known = [
            10u16, 20, 21, 22, 23, 24, 5001, 5002, 5003, 5004, 5010, 5011, 5012,
            5020, 5030, 5031, 5040, 5100, 5101, 5103,
        ];
        for &code in &known {
            assert_ne!(
                claim_text(code),
                "UNKNOWN_TRAP_CODE",
                "trap_code {} is in the registry but claim_text returns fallback",
                code,
            );
        }
    }
}
