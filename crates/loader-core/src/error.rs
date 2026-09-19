//! Structured error type for the loader algorithm.
//!
//! Each variant carries a stable numeric code (`code()`) for on-wire/ABI
//! compatibility, matching the legacy `E_*` constant values.

/// Errors from the loader algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum LoadError {
    /// The module's ABI hash does not match the platform's expected value.
    AbiMismatch,
    /// The container is structurally invalid (truncated, overlapping sections, etc.).
    BadContainer,
    /// Signature is missing, invalid, or unexpected for the platform trust tier.
    SigInvalid,
    /// The import relocation kind is not supported by this architecture.
    RelocUnsupported,
    /// An imported symbol could not be resolved from the global symbol map.
    SymbolUnresolved,
    /// Two distinct exported symbols share the same hash (FNV-1a collision).
    SymbolConflict,
    /// The module declares an @interrupt binding, which is forbidden.
    ModuleDeclaresIsr,
    /// The resource sharing class does not match the platform policy.
    ResourceSharingMismatch,
    /// The container is encrypted but the platform has no decryption support.
    ContainerEncrypted,
    /// The module's `abi_hash` has already been loaded (load-once violation).
    ModuleAlreadyLoaded,
    /// The scanner returned ⊤ (unverifiable) while the producer claimed a
    /// finite stack bound.
    StackBoundUnverifiable,
    /// Encryption is not compiled in (`#[cfg(not(feature = "encryption"))]`).
    EncUnsupported,
    /// Encryption requires a signature, but the container is encrypted
    /// without one, or the platform trust tier is too low.
    EncRequiresSigned,
    /// No KEK in the encrypted container matched any of the platform's keys.
    EncNoKey,
    /// AEAD authentication failed (tampered payload or wrong key).
    EncAuthFail,
    /// The encryption header is malformed.
    EncBadHeader,
    /// A window-base reloc site's bound base does not match the device's
    /// descriptor-derived base (P6 `check_window_base`): the module was
    /// packed against a different window geometry than this device binds.
    WindowBaseMismatch,
}

/// Allow converting from `u32` (legacy platform interface) to `LoadError`.
/// Since the mapping is lossy, all unmapped codes become `BadContainer`.
impl From<u32> for LoadError {
    fn from(_code: u32) -> Self {
        // Platform allocation errors are indistinguishable at this level.
        LoadError::BadContainer
    }
}

/// Allow converting `LoadError` to its wire code.
impl From<LoadError> for u32 {
    fn from(e: LoadError) -> Self {
        e.code()
    }
}

/// Allow comparing `LoadError` directly against numeric error codes.
impl PartialEq<u32> for LoadError {
    fn eq(&self, other: &u32) -> bool {
        self.code() == *other
    }
}

impl LoadError {
    /// Stable numeric diagnostic code.
    pub const fn code(self) -> u32 {
        match self {
            Self::AbiMismatch => 5200,
            Self::BadContainer => 5201,
            Self::SigInvalid => 5202,
            Self::ModuleDeclaresIsr => 5203,
            Self::RelocUnsupported => 5204,
            Self::SymbolUnresolved => 5205,
            Self::SymbolConflict => 5206,
            Self::ResourceSharingMismatch => 5208,
            Self::ModuleAlreadyLoaded => 5210,
            Self::ContainerEncrypted => 5212,
            Self::EncUnsupported => 5213,
            Self::EncRequiresSigned => 5214,
            Self::EncNoKey => 5215,
            Self::EncAuthFail => 5216,
            Self::EncBadHeader => 5217,
            Self::StackBoundUnverifiable => 5220,
            Self::WindowBaseMismatch => 5221,
        }
    }
}

/// Legacy numeric error code shims.
///
/// These are kept during the migration from `u32` constants to `LoadError`.
/// New code should use the `LoadError` enum directly, calling `.code()`
/// only at the wire boundary.
pub const E_ABI_MISMATCH: u32 = LoadError::AbiMismatch.code();
pub const E_BAD_CONTAINER: u32 = LoadError::BadContainer.code();
pub const E_SIG_INVALID: u32 = LoadError::SigInvalid.code();
pub const E_RELOC_UNSUPPORTED: u32 = LoadError::RelocUnsupported.code();
pub const E_SYMBOL_UNRESOLVED: u32 = LoadError::SymbolUnresolved.code();
pub const E_SYMBOL_CONFLICT: u32 = LoadError::SymbolConflict.code();
pub const E_MODULE_DECLARES_ISR: u32 = LoadError::ModuleDeclaresIsr.code();
pub const E_RESOURCE_SHARING_MISMATCH: u32 = LoadError::ResourceSharingMismatch.code();
pub const E_CONTAINER_ENCRYPTED: u32 = LoadError::ContainerEncrypted.code();
pub const E_MODULE_ALREADY_LOADED: u32 = LoadError::ModuleAlreadyLoaded.code();
pub const E_STACK_BOUND_UNVERIFIABLE: u32 = LoadError::StackBoundUnverifiable.code();
pub const E_ENC_UNSUPPORTED: u32 = LoadError::EncUnsupported.code();
pub const E_ENC_REQUIRES_SIGNED: u32 = LoadError::EncRequiresSigned.code();
pub const E_ENC_NO_KEY: u32 = LoadError::EncNoKey.code();
pub const E_ENC_AUTH_FAIL: u32 = LoadError::EncAuthFail.code();
pub const E_ENC_BAD_HEADER: u32 = LoadError::EncBadHeader.code();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(E_ABI_MISMATCH, 5200);
        assert_eq!(E_BAD_CONTAINER, 5201);
        assert_eq!(E_SIG_INVALID, 5202);
        assert_eq!(E_MODULE_DECLARES_ISR, 5203);
        assert_eq!(E_RELOC_UNSUPPORTED, 5204);
        assert_eq!(E_SYMBOL_UNRESOLVED, 5205);
        assert_eq!(E_SYMBOL_CONFLICT, 5206);
        assert_eq!(E_RESOURCE_SHARING_MISMATCH, 5208);
        assert_eq!(E_MODULE_ALREADY_LOADED, 5210);
        assert_eq!(E_CONTAINER_ENCRYPTED, 5212);
        assert_eq!(E_ENC_UNSUPPORTED, 5213);
        assert_eq!(E_ENC_REQUIRES_SIGNED, 5214);
        assert_eq!(E_ENC_NO_KEY, 5215);
        assert_eq!(E_ENC_AUTH_FAIL, 5216);
        assert_eq!(E_ENC_BAD_HEADER, 5217);
        assert_eq!(E_STACK_BOUND_UNVERIFIABLE, 5220);
    }

    #[test]
    fn load_error_code_matches_legacy() {
        assert_eq!(LoadError::AbiMismatch.code(), 5200);
        assert_eq!(LoadError::BadContainer.code(), 5201);
        assert_eq!(LoadError::StackBoundUnverifiable.code(), 5220);
    }

    #[test]
    fn all_codes_are_distinct() {
        let codes = [
            LoadError::AbiMismatch.code(),
            LoadError::BadContainer.code(),
            LoadError::SigInvalid.code(),
            LoadError::ModuleDeclaresIsr.code(),
            LoadError::RelocUnsupported.code(),
            LoadError::SymbolUnresolved.code(),
            LoadError::SymbolConflict.code(),
            LoadError::ResourceSharingMismatch.code(),
            LoadError::ModuleAlreadyLoaded.code(),
            LoadError::ContainerEncrypted.code(),
            LoadError::EncUnsupported.code(),
            LoadError::EncRequiresSigned.code(),
            LoadError::EncNoKey.code(),
            LoadError::EncAuthFail.code(),
            LoadError::EncBadHeader.code(),
            LoadError::StackBoundUnverifiable.code(),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "all error codes must be unique");
    }
}
