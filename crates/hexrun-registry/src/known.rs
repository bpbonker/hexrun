//! Built-in registry of known NPU-ready models.
//!
//! Each entry maps a hexrun-side short name to a download URL plus the
//! metadata `hexrun pull` needs to produce a working manifest. The
//! download URLs point at Qualcomm's public S3 release assets (the same
//! URLs surfaced in `release_assets.json` at the corresponding HuggingFace
//! repo, e.g. `https://huggingface.co/qualcomm/Phi-3.5-mini-instruct`).
//!
//! Only a small handful of models are included for v0.1.0; the registry
//! will grow as we verify each one on hardware. Unverified additions are
//! discouraged — the goal is "if it's in here, it works."

/// A known model entry.
pub struct KnownModel {
    /// Short name used at the CLI (`hexrun pull <name>`).
    pub name: &'static str,
    /// Direct HTTPS URL to the model bundle zip.
    pub url: &'static str,
    /// Architecture identifier copied verbatim into the manifest.
    pub arch: &'static str,
    /// Quantization scheme as shipped by Qualcomm.
    pub quant: &'static str,
    /// Approximate compressed download size, for progress reporting.
    pub size_estimate_bytes: u64,
    /// QAIRT version the bundle was compiled against.
    pub qairt_version: &'static str,
    /// Default chat template (matches the model family's prompt format).
    pub chat_template: ChatTemplateSpec,
}

/// Compile-time chat-template spec embedded in the registry.
pub struct ChatTemplateSpec {
    /// System prompt to use when none is supplied.
    pub system_prompt: &'static str,
    /// Format string with `{system}` and `{user}` placeholders.
    pub template: &'static str,
}

/// Built-in registry. Public so the CLI's `pull` command can list them.
pub const KNOWN_MODELS: &[KnownModel] = &[
    KnownModel {
        name: "phi-3.5-mini",
        url: "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/phi_3_5_mini_instruct/releases/v0.52.0/phi_3_5_mini_instruct-genie-w4a16-qualcomm_snapdragon_x_elite.zip",
        arch: "phi3",
        quant: "w4a16",
        size_estimate_bytes: 2_080_000_000,
        qairt_version: "2.43.1",
        chat_template: ChatTemplateSpec {
            system_prompt: "You are a concise assistant. Answer in 1-2 sentences.",
            template: "<|system|>\n{system}<|end|>\n<|user|>\n{user}<|end|>\n<|assistant|>\n",
        },
    },
    KnownModel {
        name: "llama-v3-1-8b-instruct",
        url: "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/llama_v3_1_8b_instruct/releases/v0.52.0/llama_v3_1_8b_instruct-genie-w4a16-qualcomm_snapdragon_x_elite.zip",
        arch: "llama",
        quant: "w4a16",
        size_estimate_bytes: 4_500_000_000,
        qairt_version: "2.43.1",
        chat_template: ChatTemplateSpec {
            system_prompt: "You are a concise assistant.",
            template: "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n{system}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n{user}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n",
        },
    },
    KnownModel {
        name: "qwen-2-5-7b",
        url: "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/qwen2_5_7b_instruct/releases/v0.52.0/qwen2_5_7b_instruct-genie-w4a16-qualcomm_snapdragon_x_elite.zip",
        arch: "qwen2",
        quant: "w4a16",
        size_estimate_bytes: 4_300_000_000,
        qairt_version: "2.43.1",
        chat_template: ChatTemplateSpec {
            system_prompt: "You are a concise assistant. Answer in 1-2 sentences.",
            template: "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
        },
    },
];

impl KnownModel {
    /// Look up a known model by name. Returns `None` if not registered.
    pub fn lookup(name: &str) -> Option<&'static KnownModel> {
        KNOWN_MODELS.iter().find(|m| m.name == name)
    }
}
