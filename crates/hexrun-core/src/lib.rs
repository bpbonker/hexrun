//! hexrun inference engine core.
//!
//! Two parallel inference paths picked at model-load time:
//! - `Backend::Genie`: load a Genie LLM bundle (compiled context-binary
//!   shards) and run inference via the Genie C runtime. This is the
//!   default and currently-supported LLM path on Snapdragon X Elite NPU.
//!   Gated behind the `genie` feature.
//! - `Backend::Ort`: ONNX Runtime with the QNN Execution Provider. Reserved
//!   for non-LLM models and future work.
//!
//! Library-side code uses concrete `thiserror` error types; the CLI binary
//! is the only place `anyhow` lives.

#![warn(missing_docs, clippy::all)]

pub mod engine;
pub mod manifest;
pub mod sampler;

pub use engine::{Backend, Engine, EngineConfig, EngineError};
pub use manifest::{
    ChatMessage, ChatRole, ChatTemplate, Manifest, ManifestError, ManifestFiles, Quant,
};
pub use sampler::{sample, SamplerConfig, SamplerError};
