//! Built-in registry of known NPU-ready models.
//!
//! Each entry maps a npurun-side short name to a download URL plus the
//! metadata `npurun pull` needs to produce a working manifest. The
//! download URLs point at Qualcomm's public S3 release assets (the same
//! URLs surfaced in `release_assets.json` at the corresponding HuggingFace
//! repo, e.g. `https://huggingface.co/qualcomm/Phi-3.5-mini-instruct`).
//!
//! Only a small handful of models are included for v0.1.0; the registry
//! will grow as we verify each one on hardware. Unverified additions are
//! discouraged — the goal is "if it's in here, it works."

/// A known model entry.
pub struct KnownModel {
    /// Short name used at the CLI (`npurun pull <name>`).
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
    /// First-turn format with `{system}` and `{user}` placeholders.
    pub template: &'static str,
    /// Format for an assistant turn in a multi-turn transcript, with a
    /// single `{assistant}` placeholder.
    pub assistant_turn: &'static str,
    /// Format for a follow-up user turn in a multi-turn transcript,
    /// with a single `{user}` placeholder.
    pub next_user_turn: &'static str,
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
            assistant_turn: "{assistant}<|end|>\n",
            next_user_turn: "<|user|>\n{user}<|end|>\n<|assistant|>\n",
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
            assistant_turn: "{assistant}<|eot_id|>",
            next_user_turn: "<|start_header_id|>user<|end_header_id|>\n\n{user}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n",
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
            assistant_turn: "{assistant}<|im_end|>\n",
            next_user_turn: "<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
        },
    },
    // Qwen3 4B Instruct (July 2025 update). Verified end-to-end on
    // X1E at 11.7 tok/s post-TTFT — matches Phi 3.5 Mini's NPU
    // ceiling. Bundle uses the new multi-graph naming scheme
    // (`prompt_ar128_*` / `token_ar1_*`); pull_model auto-injects
    // `enable-graph-switching: true` after extract.
    KnownModel {
        name: "qwen3-4b-instruct-2507",
        url: "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/qwen3_4b_instruct_2507/releases/v0.52.0/qwen3_4b_instruct_2507-genie-w4a16-qualcomm_snapdragon_x_elite.zip",
        arch: "qwen3",
        quant: "w4a16",
        size_estimate_bytes: 2_530_000_000,
        qairt_version: "2.45.0",
        chat_template: ChatTemplateSpec {
            system_prompt: "You are a helpful AI assistant",
            template: "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
            assistant_turn: "{assistant}<|im_end|>\n",
            next_user_turn: "<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
        },
    },
    // Qwen3 4B base. Same multi-graph format as Instruct-2507; prefer
    // Instruct-2507 for chat. Bundled here so users can grab the base
    // for fine-tuning experiments.
    KnownModel {
        name: "qwen3-4b",
        url: "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/qwen3_4b/releases/v0.52.0/qwen3_4b-genie-w4a16-qualcomm_snapdragon_x_elite.zip",
        arch: "qwen3",
        quant: "w4a16",
        size_estimate_bytes: 2_530_000_000,
        qairt_version: "2.45.0",
        chat_template: ChatTemplateSpec {
            system_prompt: "You are a helpful AI assistant",
            template: "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
            assistant_turn: "{assistant}<|im_end|>\n",
            next_user_turn: "<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
        },
    },
    // Qwen2.5 VL 7B Instruct. Vision-language; npurun feeds text only,
    // vision tower stays unused. Multi-graph w4a16 bundle for X-Elite.
    KnownModel {
        name: "qwen-2-5-vl-7b-instruct",
        url: "https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/qwen2_5_vl_7b_instruct/releases/v0.52.0/qwen2_5_vl_7b_instruct-genie-w4a16-qualcomm_snapdragon_x_elite.zip",
        arch: "qwen2-vl",
        quant: "w4a16",
        size_estimate_bytes: 4_000_000_000,
        qairt_version: "2.45.0",
        chat_template: ChatTemplateSpec {
            system_prompt: "You are a helpful AI assistant",
            template: "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
            assistant_turn: "{assistant}<|im_end|>\n",
            next_user_turn: "<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n",
        },
    },
];

impl KnownModel {
    /// Look up a known model by name. Returns `None` if not registered.
    pub fn lookup(name: &str) -> Option<&'static KnownModel> {
        KNOWN_MODELS.iter().find(|m| m.name == name)
    }
}
