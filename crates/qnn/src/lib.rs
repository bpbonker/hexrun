//! Safe, ergonomic wrappers around the Qualcomm AI Engine Direct (QNN) and
//! Genie LLM runtime C APIs.
//!
//! Two layers:
//! - The **Genie** module ([`genie`]) wraps `libGenie` (the higher-level LLM
//!   runtime). This is the path used for inference of pre-built Snapdragon
//!   X Elite NPU bundles like the Qwen 2.5 7B context binaries produced by
//!   Qualcomm AI Hub. Genie ships an import library, so the bindings are
//!   statically linked.
//! - The **QNN core** wrappers (planned, future work) will wrap raw QNN
//!   contexts/graphs/tensors for non-LLM workloads. QAIRT does not ship a
//!   `QnnSystem.lib` import library on Windows ARM64, so that path will use
//!   `libloading` for dynamic dispatch.
//!
//! All public types are RAII; their `Drop` impls call the matching
//! `*_free` C function.

#![warn(missing_docs, clippy::all)]

pub mod genie;

pub use genie::{api_version, ApiVersion, Dialog, GenieError, SentenceCode};
