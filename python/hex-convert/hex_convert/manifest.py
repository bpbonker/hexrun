"""Emit a hexrun.json manifest from a Genie bundle directory.

A Genie bundle is the on-disk artifact that Qualcomm AI Hub produces when
you compile an LLM for Snapdragon X Elite — typically a directory containing:

- ``genie_config.json``
- One or more compiled context-binary shards (``*.bin``)
- ``tokenizer.json`` (HuggingFace tokenizer JSON)
- Optional ``htp_backend_ext_config.json``

The hexrun runtime needs a ``hexrun.json`` manifest on top of that bundle
so it can validate the SDK version, look up the chat template, and verify
file integrity. ``hex-convert manifest`` writes that file.

This module is the canonical place for "what does a hexrun manifest look
like for a Genie bundle" — the Rust ``hexrun-registry`` crate does the
same job for the registry's pre-built models.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path

# Per-architecture chat templates. Mirrors hexrun-registry/src/known.rs.
# When `--arch` is provided we use that; otherwise we sniff from the
# bundle name (Phi/Llama/Qwen). If sniffing fails, we leave chat_template
# empty and the user is told to fill it in by hand.
_CHAT_TEMPLATES: dict[str, dict[str, str]] = {
    "phi3": {
        "system_prompt": "You are a concise assistant. Answer in 1-2 sentences.",
        "template": "<|system|>\n{system}<|end|>\n<|user|>\n{user}<|end|>\n<|assistant|>\n",
        "assistant_turn": "{assistant}<|end|>\n",
        "next_user_turn": "<|user|>\n{user}<|end|>\n<|assistant|>\n",
    },
    "llama": {
        "system_prompt": "You are a concise assistant.",
        "template": (
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n"
            "{system}<|eot_id|>"
            "<|start_header_id|>user<|end_header_id|>\n\n"
            "{user}<|eot_id|>"
            "<|start_header_id|>assistant<|end_header_id|>\n\n"
        ),
        "assistant_turn": "{assistant}<|eot_id|>",
        "next_user_turn": (
            "<|start_header_id|>user<|end_header_id|>\n\n"
            "{user}<|eot_id|>"
            "<|start_header_id|>assistant<|end_header_id|>\n\n"
        ),
    },
    "qwen2": {
        "system_prompt": "You are a concise assistant. Answer in 1-2 sentences.",
        "template": (
            "<|im_start|>system\n{system}<|im_end|>\n"
            "<|im_start|>user\n{user}<|im_end|>\n"
            "<|im_start|>assistant\n"
        ),
        "assistant_turn": "{assistant}<|im_end|>\n",
        "next_user_turn": (
            "<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n"
        ),
    },
}


def _sniff_arch(bundle_name: str) -> str | None:
    """Best-effort arch inference from a bundle directory name.

    The qai-hub-models export naming is very consistent (e.g.
    ``phi_3_5_mini_instruct-genie-w4a16-qualcomm_snapdragon_x_elite``)
    so a few substring checks cover the supported set.
    """
    n = bundle_name.lower()
    if "phi" in n:
        return "phi3"
    if "llama" in n or "llama_v3" in n or "llama-v3" in n:
        return "llama"
    if "qwen" in n:
        return "qwen2"
    return None


def _sniff_quant(bundle_name: str) -> str | None:
    n = bundle_name.lower()
    for q in ("w4a16", "w8a16", "int8-w-int16-a", "int8", "int4", "fp16"):
        if q in n:
            return q
    return None


@dataclass
class ManifestOptions:
    """All inputs needed to write a hexrun.json over a Genie bundle."""

    # The model directory we're going to write `hexrun.json` into. The
    # `bundle` directory is somewhere underneath (we record its location
    # relative to this in the manifest).
    model_dir: Path
    # The directory containing genie_config.json. Must be inside model_dir.
    bundle_dir: Path
    # The user-facing model name (registry slug — e.g. ``phi-3.5-mini``).
    name: str
    # Architecture identifier. If None, sniffed from the bundle name.
    arch: str | None = None
    # Quantization label. If None, sniffed from the bundle name.
    quant: str | None = None
    # QAIRT SDK version the bundle was compiled against.
    qnn_sdk: str = "2.45.0"
    # Manifest version.
    version: str = "0.1.0"
    # Optional chat template override. If None, looked up by arch.
    chat_template_override: dict[str, str] | None = None
    # Files (relative to bundle_dir) to record sha256 for. If empty we
    # default to all files under bundle_dir.
    sha256_files: list[Path] = field(default_factory=list)


def write_manifest(opts: ManifestOptions) -> Path:
    """Write a hexrun.json into ``opts.model_dir``. Returns its path."""
    if not opts.bundle_dir.is_dir():
        raise FileNotFoundError(f"bundle_dir does not exist: {opts.bundle_dir}")
    genie_cfg = opts.bundle_dir / "genie_config.json"
    if not genie_cfg.is_file():
        raise FileNotFoundError(
            f"no genie_config.json under {opts.bundle_dir} — is this really a Genie bundle?"
        )

    with genie_cfg.open("r", encoding="utf-8") as fh:
        cfg = json.load(fh)
    try:
        ctx_block = cfg["dialog"]["context"]
        context = int(ctx_block["size"])
        vocab = int(ctx_block["n-vocab"]) if "n-vocab" in ctx_block else int(ctx_block["n_vocab"])
    except KeyError as e:
        raise ValueError(
            f"genie_config.json at {genie_cfg} is missing dialog.context.{e.args[0]}"
        ) from e

    arch = opts.arch or _sniff_arch(opts.bundle_dir.name) or _sniff_arch(opts.name)
    if arch is None:
        raise ValueError(
            f"could not infer arch from bundle name {opts.bundle_dir.name!r} or "
            f"model name {opts.name!r} — pass --arch explicitly"
        )

    quant = opts.quant or _sniff_quant(opts.bundle_dir.name) or "w8a16"
    chat_template = opts.chat_template_override or _CHAT_TEMPLATES.get(arch)

    # bundle_dir-relative-to-model_dir, with forward slashes for portability.
    try:
        bundle_rel = opts.bundle_dir.relative_to(opts.model_dir)
    except ValueError as e:
        raise ValueError(
            f"bundle_dir {opts.bundle_dir} is not inside model_dir {opts.model_dir}"
        ) from e
    bundle_rel_str = bundle_rel.as_posix()

    # Locate tokenizer + genie_config.
    tokenizer_path = opts.bundle_dir / "tokenizer.json"
    if not tokenizer_path.is_file():
        raise FileNotFoundError(
            f"no tokenizer.json under {opts.bundle_dir} — Genie bundles must include one"
        )

    files = {
        "tokenizer": f"{bundle_rel_str}/tokenizer.json",
        "genie_config": f"{bundle_rel_str}/genie_config.json",
    }

    # sha256s. Default = every file in the bundle directory.
    if opts.sha256_files:
        sha_targets: Iterable[Path] = opts.sha256_files
    else:
        sha_targets = sorted(p for p in opts.bundle_dir.rglob("*") if p.is_file())

    sha256: dict[str, str] = {}
    for f in sha_targets:
        rel = f.relative_to(opts.model_dir).as_posix()
        sha256[rel] = _hash_file(f)

    manifest = {
        "name": opts.name,
        "version": opts.version,
        "arch": arch,
        "vocab": vocab,
        "context": context,
        "quant": quant,
        "qnn_sdk": opts.qnn_sdk,
        "files": files,
        "sha256": sha256,
    }
    if chat_template is not None:
        manifest["chat_template"] = chat_template

    manifest_path = opts.model_dir / "hexrun.json"
    with manifest_path.open("w", encoding="utf-8") as fh:
        json.dump(manifest, fh, indent=2, sort_keys=False)
        fh.write("\n")
    return manifest_path


def _hash_file(path: Path, *, chunk_size: int = 1024 * 1024) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            buf = fh.read(chunk_size)
            if not buf:
                break
            h.update(buf)
    return h.hexdigest()
