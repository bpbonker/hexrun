//! Safe, ergonomic wrappers around the Qualcomm AI Engine Direct (QNN) runtime.
//!
//! Phase 1 surface (planned): `Backend`, `Context`, `Graph`, `Tensor`, plus a
//! `Capabilities` snapshot used by hexrun to refuse loading context binaries
//! whose embedded SDK version mismatches by minor — directly addressing the
//! version-pinning trap that hit Nexa SDK #1060.
//!
//! All public types are RAII; drop runs the matching `Qnn*_free` call.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum QnnError {
    #[error("QNN SDK not present at build time (QNN_SDK_ROOT was unset)")]
    SdkMissing,
    #[error("QNN runtime returned status {0}")]
    Status(i32),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

/// Snapshot of the runtime environment used for version compatibility checks.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub sdk_version: String,
    pub htp_driver_version: Option<String>,
}

impl Capabilities {
    /// Probe the loaded QNN runtime for its version. Phase 1 stub.
    pub fn probe() -> Result<Self, QnnError> {
        Err(QnnError::SdkMissing)
    }
}
