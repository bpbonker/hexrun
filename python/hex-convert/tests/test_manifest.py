"""Tests for hex_convert.manifest.

Synthesize a minimal Genie bundle on disk (genie_config.json, a fake
tokenizer.json, a placeholder .bin shard) and verify ``write_manifest``
produces a hexrun.json that:

- has the right top-level fields
- locates files at the right relative paths
- computes correct sha256s
- picks up the chat template from `arch`
- sniffs `arch` and `quant` from a Phi 3.5 Mini-style bundle name when
  not given explicitly
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

from hex_convert.manifest import ManifestOptions, write_manifest


def _make_bundle(model_dir: Path, bundle_subdir: str) -> Path:
    """Synthesize a minimal Genie-bundle layout under model_dir/bundle_subdir."""
    bundle = model_dir / bundle_subdir
    bundle.mkdir(parents=True)
    (bundle / "genie_config.json").write_text(
        json.dumps(
            {
                "dialog": {
                    "context": {
                        "size": 4096,
                        "n-vocab": 32064,
                    }
                }
            }
        ),
        encoding="utf-8",
    )
    (bundle / "tokenizer.json").write_text('{"model":{"type":"BPE"}}', encoding="utf-8")
    (bundle / "weights_1_of_4.bin").write_bytes(b"\x01\x02\x03\x04")
    (bundle / "weights_2_of_4.bin").write_bytes(b"\x05\x06\x07\x08")
    return bundle


def test_writes_manifest_with_expected_fields(tmp_path: Path) -> None:
    bundle = _make_bundle(
        tmp_path,
        "bundle/phi_3_5_mini_instruct-genie-w4a16-qualcomm_snapdragon_x_elite",
    )
    path = write_manifest(
        ManifestOptions(
            model_dir=tmp_path,
            bundle_dir=bundle,
            name="phi-3.5-mini",
        )
    )
    assert path == tmp_path / "hexrun.json"

    m = json.loads(path.read_text(encoding="utf-8"))
    assert m["name"] == "phi-3.5-mini"
    assert m["arch"] == "phi3"
    assert m["quant"] == "w4a16"  # sniffed from bundle dir name
    assert m["context"] == 4096
    assert m["vocab"] == 32064
    # files dict references the bundle relative path
    assert m["files"]["genie_config"].endswith("/genie_config.json")
    assert m["files"]["tokenizer"].endswith("/tokenizer.json")
    # chat_template is auto-populated for phi3
    assert m["chat_template"]["assistant_turn"].endswith("<|end|>\n")


def test_sha256_matches_actual_file_content(tmp_path: Path) -> None:
    bundle = _make_bundle(tmp_path, "bundle/qwen2_5_7b-genie-w8a16")
    write_manifest(
        ManifestOptions(
            model_dir=tmp_path,
            bundle_dir=bundle,
            name="qwen-2-5-7b",
        )
    )
    m = json.loads((tmp_path / "hexrun.json").read_text(encoding="utf-8"))
    sha = m["sha256"]
    assert sha, "expected at least one sha256 entry"
    for rel, expected in sha.items():
        actual = hashlib.sha256((tmp_path / rel).read_bytes()).hexdigest()
        assert actual == expected, f"{rel}: manifest claims {expected}, file is {actual}"


def test_sniffs_qwen_arch_and_w8a16(tmp_path: Path) -> None:
    bundle = _make_bundle(
        tmp_path, "bundle/qwen2_5_7b_instruct-genie-w8a16-qualcomm_snapdragon_x_elite"
    )
    write_manifest(
        ManifestOptions(
            model_dir=tmp_path,
            bundle_dir=bundle,
            name="qwen-2-5-7b",
        )
    )
    m = json.loads((tmp_path / "hexrun.json").read_text(encoding="utf-8"))
    assert m["arch"] == "qwen2"
    assert m["quant"] == "w8a16"
    assert m["chat_template"]["template"].startswith("<|im_start|>")


def test_explicit_arch_and_quant_override_sniff(tmp_path: Path) -> None:
    bundle = _make_bundle(tmp_path, "bundle/anonymized-bundle-name")
    write_manifest(
        ManifestOptions(
            model_dir=tmp_path,
            bundle_dir=bundle,
            name="my-model",
            arch="llama",
            quant="int8-w-int16-a",
        )
    )
    m = json.loads((tmp_path / "hexrun.json").read_text(encoding="utf-8"))
    assert m["arch"] == "llama"
    assert m["quant"] == "int8-w-int16-a"
    assert m["chat_template"]["template"].startswith("<|begin_of_text|>")


def test_rejects_bundle_outside_model_dir(tmp_path: Path) -> None:
    other = tmp_path / "other"
    other.mkdir()
    bundle = _make_bundle(other, "bundle")
    with pytest.raises(ValueError, match="not inside model_dir"):
        write_manifest(
            ManifestOptions(
                model_dir=tmp_path / "model",
                bundle_dir=bundle,
                name="phi-3.5-mini",
            )
        )


def test_rejects_missing_genie_config(tmp_path: Path) -> None:
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "tokenizer.json").write_text("{}", encoding="utf-8")
    with pytest.raises(FileNotFoundError, match="no genie_config.json"):
        write_manifest(
            ManifestOptions(
                model_dir=tmp_path,
                bundle_dir=bundle,
                name="phi-3.5-mini",
            )
        )


def test_rejects_missing_tokenizer(tmp_path: Path) -> None:
    bundle = tmp_path / "bundle"
    bundle.mkdir()
    (bundle / "genie_config.json").write_text(
        json.dumps({"dialog": {"context": {"size": 4096, "n-vocab": 32000}}}),
        encoding="utf-8",
    )
    with pytest.raises(FileNotFoundError, match="no tokenizer.json"):
        write_manifest(
            ManifestOptions(
                model_dir=tmp_path,
                bundle_dir=bundle,
                name="phi-3.5-mini",
            )
        )


def test_unknown_arch_without_override_errors(tmp_path: Path) -> None:
    bundle = _make_bundle(tmp_path, "bundle/totally-anonymous")
    with pytest.raises(ValueError, match="could not infer arch"):
        write_manifest(
            ManifestOptions(
                model_dir=tmp_path,
                bundle_dir=bundle,
                name="not-a-known-family",
            )
        )
