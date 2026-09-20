//! Adapter boundary for the pinned rustpush revision.
//!
//! This crate is intentionally the only workspace crate that will depend on
//! rustpush once the upstream revision has been audited and pinned.

use thiserror::Error;

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
