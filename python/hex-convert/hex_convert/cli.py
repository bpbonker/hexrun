"""hex-convert CLI entry point.

Three subcommands:

- ``hex-convert manifest`` — emit a hexrun.json over an existing Genie
  bundle directory. The fast, immediate use: take a bundle you got
  from qai-hub-models or downloaded from Qualcomm's HuggingFace and
  make it ``hexrun pull``-equivalent on disk.
- ``hex-convert inspect`` — pretty-print a hexrun bundle or a raw
  Genie bundle, optionally verifying sha256s.
- ``hex-convert export`` — orchestrate a full HF -> ONNX -> AI-Hub
  cloud compile -> bundle pipeline by shelling out to qai-hub-models.
  Hours-of-cloud-compute heavy; requires an AI Hub token.

For day-to-day use ``manifest`` and ``inspect`` are what you reach for.
``export`` is for adding new models to the registry.
"""

from __future__ import annotations

import sys
from pathlib import Path

import click

from .export import ExportError, ExportOptions, export, known_slugs
from .inspect import inspect as inspect_target
from .manifest import ManifestOptions, write_manifest


@click.group()
@click.version_option()
def main() -> None:
    """Convert HuggingFace LLMs into hexrun-compatible bundles."""


@main.command("manifest")
@click.option(
    "--model-dir",
    type=click.Path(file_okay=False, dir_okay=True, path_type=Path),
    required=True,
    help="Directory the hexrun.json will be written into.",
)
@click.option(
    "--bundle-dir",
    type=click.Path(file_okay=False, dir_okay=True, exists=True, path_type=Path),
    required=True,
    help="Directory containing genie_config.json. Must be inside --model-dir.",
)
@click.option("--name", required=True, help="Registry slug, e.g. phi-3.5-mini")
@click.option("--arch", default=None, help="phi3 / llama / qwen2 (sniffed if omitted)")
@click.option(
    "--quant",
    default=None,
    help="w4a16 / w8a16 / int8-w-int16-a / int8 / int4 / fp16 (sniffed if omitted)",
)
@click.option("--qnn-sdk", default="2.45.0", show_default=True)
@click.option("--version", "manifest_version", default="0.1.0", show_default=True)
def manifest_cmd(
    model_dir: Path,
    bundle_dir: Path,
    name: str,
    arch: str | None,
    quant: str | None,
    qnn_sdk: str,
    manifest_version: str,
) -> None:
    """Emit hexrun.json over an existing Genie bundle."""
    model_dir.mkdir(parents=True, exist_ok=True)
    try:
        path = write_manifest(
            ManifestOptions(
                model_dir=model_dir,
                bundle_dir=bundle_dir,
                name=name,
                arch=arch,
                quant=quant,
                qnn_sdk=qnn_sdk,
                version=manifest_version,
            )
        )
    except (FileNotFoundError, ValueError) as e:
        click.echo(f"hex-convert manifest: {e}", err=True)
        sys.exit(2)
    click.echo(f"wrote {path}")


@main.command("inspect")
@click.argument(
    "target",
    type=click.Path(exists=True, path_type=Path),
)
@click.option(
    "--no-verify",
    is_flag=True,
    help="Skip sha256 verification (fast).",
)
def inspect_cmd(target: Path, no_verify: bool) -> None:
    """Pretty-print a hexrun bundle, hexrun.json, or raw Genie bundle."""
    code = inspect_target(target, verify=not no_verify)
    sys.exit(code)


@main.command("export")
@click.argument("slug")
@click.option(
    "--output",
    "-o",
    type=click.Path(file_okay=False, dir_okay=True, path_type=Path),
    required=True,
    help="Output directory; the hexrun.json + bundle land here.",
)
@click.option(
    "--quant",
    default=None,
    help="Override the recipe's default quant (e.g. w4a16, w8a16).",
)
@click.option("--qnn-sdk", default="2.45.0", show_default=True)
@click.option(
    "--skip-compile",
    is_flag=True,
    help=(
        "Don't shell out to qai-hub-models; assume the bundle is already under "
        "--output and just emit the manifest."
    ),
)
def export_cmd(
    slug: str, output: Path, quant: str | None, qnn_sdk: str, skip_compile: bool
) -> None:
    """Run qai-hub-models export for SLUG and emit a hexrun bundle.

    Heavy: requires QAI_HUB_API_TOKEN, x64 Python, network, and 30-90
    minutes of cloud compile time. Use --skip-compile to re-run only
    the manifest step against an existing bundle.

    Known slugs: phi-3.5-mini, llama-v3-1-8b-instruct, qwen-2-5-7b.
    """
    if slug == "list" or slug == "?":
        click.echo("known export recipes:")
        for s in known_slugs():
            click.echo(f"  {s}")
        return

    output.mkdir(parents=True, exist_ok=True)
    try:
        manifest_path = export(
            ExportOptions(
                slug=slug,
                output=output,
                quant=quant,
                qnn_sdk=qnn_sdk,
                skip_compile=skip_compile,
            )
        )
    except ExportError as e:
        click.echo(f"hex-convert export: {e}", err=True)
        sys.exit(2)
    click.echo(f"wrote {manifest_path}")


if __name__ == "__main__":
    main()
