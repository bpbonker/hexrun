//! hexrun inference engine core.
//!
//! Two parallel inference paths picked at model-load time:
//! - `Backend::Ort`: ONNX Runtime with the QNN Execution Provider (default).
//! - `Backend::QnnDirect`: load a pre-built QNN context binary directly via
//!   the `qnn` crate. Gated behind the `qnn-direct` feature.
//!
//! Library-side code uses concrete `thiserror` error types; the CLI binary
//! is the only place `anyhow` lives.

#![warn(missing_docs, clippy::all)]

pub mod engine;
pub mod manifest;
pub mod sampler;

pub use engine::{Backend, Engine, EngineConfig, EngineError, GenerationStream};
pub use manifest::{Manifest, ManifestError, ManifestFiles, Quant};
pub use sampler::{sample, SamplerConfig, SamplerError};
