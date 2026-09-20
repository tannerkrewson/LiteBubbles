//! Adapter boundary for the pinned rustpush revision.
//!
//! This crate is the only workspace crate that depends directly on the pinned
//! rustpush revision. Upstream types stay behind this boundary.

pub mod hardware;
pub mod validation;

pub use hardware::{HardwareInputError, MacHardwareConfig, MacHardwareInput, MacSoftwareInfo};
pub use validation::{ValidationBackedMacOsConfig, production_validation_provider};

use thiserror::Error;

pub const RUSTPUSH_REVISION: &str = "f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c";

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("rustpush backend has not been configured")]
    NotConfigured,
}

#[derive(Debug, Default)]
pub struct Backend;

impl Backend {
    pub fn new() -> Result<Self, BackendError> {
        Ok(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::RUSTPUSH_REVISION;

    #[test]
    fn backend_records_the_audited_rustpush_revision() {
        assert_eq!(
            RUSTPUSH_REVISION,
            "f35c4ee062b3c3eae54dc96b89b90ee99f5e1d0c"
        );
    }

    #[cfg(feature = "development-dummy-fairplay")]
    #[test]
    fn dummy_fairplay_is_an_explicit_opt_in() {}

    #[cfg(not(feature = "development-dummy-fairplay"))]
    #[test]
    fn production_build_does_not_enable_dummy_fairplay() {}
}
