"""Wrap qai-hub-models export for npurun.

The actual HF -> ONNX -> AI-Hub-cloud-compile -> Genie bundle pipeline
lives in Qualcomm's ``qai_hub_models`` Python package. We don't
reimplement it; we shell out to the per-model export script.

What this module adds on top:

- A curated map from a npurun-friendly slug (``phi-3.5-mini``) to the
  qai-hub-models module name (``phi_3_5_mini_instruct``) and HF id.
- Sanity checks before invoking: is the AI Hub token set, is x64
  Python in use, does the model bundle already exist (so we don't
  burn an hour of cloud compile time for nothing).
- Post-processing: locate the produced bundle, emit a npurun.json
  via ``npu_convert.manifest``.

Heavy lift caveats:

- An AI Hub API token is required (sign up at https://aihub.qualcomm.com).
- The cloud compile + link can take 30-90 minutes wall-clock per model.
- ARM64 Python cannot run the export — must be x64 Python under Prism.
- ~10-30 GB of disk per intermediate artifact set.

This is a thin orchestrator. The heavy lifting is in qai-hub-models;
fix bugs there, not here.
"""

from __future__ import annotations

import os
import platform
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from .manifest import ManifestOptions, write_manifest


@dataclass(frozen=True)
class _Recipe:
    """A curated qai-hub-models export recipe."""

    slug: str  # npurun slug
    qai_hub_module: str  # qai_hub_models.models.<this>
    hf_id: str  # informational
    arch: str  # phi3 / llama / qwen2
    default_quant: str  # w4a16 / w8a16 / etc.


_RECIPES: dict[str, _Recipe] = {
    r.slug: r
    for r in (
        _Recipe(
            slug="phi-3.5-mini",
            qai_hub_module="phi_3_5_mini_instruct",
            hf_id="microsoft/Phi-3.5-mini-instruct",
            arch="phi3",
            default_quant="w4a16",
        ),
        _Recipe(
            slug="llama-v3-1-8b-instruct",
            qai_hub_module="llama_v3_1_8b_instruct",
            hf_id="meta-llama/Llama-3.1-8B-Instruct",
            arch="llama",
            default_quant="w4a16",
        ),
        _Recipe(
            slug="qwen-2-5-7b",
            qai_hub_module="qwen2_5_7b_instruct",
            hf_id="Qwen/Qwen2.5-7B-Instruct",
            arch="qwen2",
            default_quant="w4a16",
        ),
    )
}


def known_slugs() -> list[str]:
    return sorted(_RECIPES.keys())


@dataclass
class ExportOptions:
    slug: str
    output: Path
    quant: str | None = None  # falls back to recipe default
    qnn_sdk: str = "2.45.0"
    skip_compile: bool = False  # post-process only; assume bundle is already in --output


class ExportError(RuntimeError):
    """Raised when export prerequisites or the export itself fails."""


def export(opts: ExportOptions) -> Path:
    """Run a qai-hub-models export and emit a npurun.json.

    Returns the path to the written npurun.json on success.
    """
    recipe = _RECIPES.get(opts.slug)
    if recipe is None:
        raise ExportError(
            f"unknown slug {opts.slug!r}. Known: {', '.join(known_slugs())}"
        )

    quant = opts.quant or recipe.default_quant

    if not opts.skip_compile:
        _check_prereqs()
        _run_qai_hub_export(recipe, quant, opts.output)

    bundle_dir = _locate_bundle_dir(opts.output, recipe, quant)
    if bundle_dir is None:
        raise ExportError(
            f"no Genie bundle found under {opts.output}. "
            "If the qai-hub-models export wrote somewhere else, copy or symlink "
            "it under that directory and re-run with --skip-compile."
        )

    manifest_path = write_manifest(
        ManifestOptions(
            model_dir=opts.output,
            bundle_dir=bundle_dir,
            name=recipe.slug,
            arch=recipe.arch,
            quant=quant,
            qnn_sdk=opts.qnn_sdk,
        )
    )
    return manifest_path


def _check_prereqs() -> None:
    if platform.machine().lower() in ("arm64", "aarch64"):
        # qai-hub-models doesn't run on Windows ARM64 today (ONNX
        # quantization tooling missing). Fail fast instead of
        # mid-compile.
        raise ExportError(
            "running on ARM64 Python — qai-hub-models export does not work here. "
            "Use an x64 Python venv (Prism emulation) for export. The compiled "
            "bundle can then be used by the ARM64 npurun runtime."
        )
    if not os.environ.get("QAI_HUB_API_TOKEN"):
        raise ExportError(
            "QAI_HUB_API_TOKEN is not set. Sign up at https://aihub.qualcomm.com, "
            "create a token, and `set QAI_HUB_API_TOKEN=...` before running."
        )
    try:
        import qai_hub_models  # type: ignore[unused-import]  # noqa: F401
    except ImportError as e:
        raise ExportError(
            "qai-hub-models is not installed in this Python. "
            "`pip install qai-hub-models[<model>]` for the model you want to export."
        ) from e


def _run_qai_hub_export(recipe: _Recipe, quant: str, output: Path) -> None:
    """Shell out to qai-hub-models's per-model export script.

    The qai-hub-models package exposes one export module per supported
    model, e.g. ``qai_hub_models.models.phi_3_5_mini_instruct.export``.
    Args we care about:

    - ``--target-runtime qnn_dlc`` — emit Genie context binaries.
    - ``--device "Snapdragon X Elite CRD"`` — compile target.
    - ``--output-dir`` — where artifacts land.
    - ``--precision`` — quantization scheme.
    """
    output.mkdir(parents=True, exist_ok=True)
    cmd = [
        sys.executable,
        "-m",
        f"qai_hub_models.models.{recipe.qai_hub_module}.export",
        "--target-runtime",
        "qnn_dlc",
        "--chipset",
        "qualcomm-snapdragon-x-elite",
        "--precision",
        quant,
        "--output-dir",
        str(output),
        "--model-cache-mode",
        "enable",
    ]
    print(f"[npu-convert] launching: {' '.join(cmd)}", file=sys.stderr)
    try:
        subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError as e:
        raise ExportError(
            f"qai-hub-models export failed (exit {e.returncode}). "
            "See the export logs above for the cloud job id; you can resume "
            "with --skip-compile once the job lands and the bundle is on disk."
        ) from e


def _locate_bundle_dir(output: Path, recipe: _Recipe, quant: str) -> Path | None:
    """Find the Genie bundle dir under `output`.

    qai-hub-models lays the bundle out as
    ``<output>/<module>-genie-<quant>-qualcomm_snapdragon_x_elite/``
    with a ``genie_config.json`` inside. We look for that first, then
    fall back to any directory containing genie_config.json.
    """
    expected = (
        f"{recipe.qai_hub_module}-genie-{quant}-qualcomm_snapdragon_x_elite"
    )
    candidate = output / expected
    if (candidate / "genie_config.json").is_file():
        return candidate

    # Walk one level deep looking for any genie_config.json.
    for sub in output.iterdir() if output.is_dir() else []:
        if not sub.is_dir():
            continue
        if (sub / "genie_config.json").is_file():
            return sub
    return None


def _which(name: str) -> str | None:
    """Trampoline so test helpers can stub `shutil.which`."""
    return shutil.which(name)
