"""hex-convert CLI entry point. Phase 5 implementation."""

from __future__ import annotations

import sys

import click

from .pipeline import ConvertOptions, convert


@click.group()
@click.version_option()
def main() -> None:
    """Convert HuggingFace models for hexrun."""


@main.command()
@click.argument("hf_id")
@click.option("--output", "-o", required=True, type=click.Path(), help="Output directory")
@click.option(
    "--quant",
    type=click.Choice(["int8", "int8-w-int16-a", "int4"]),
    default="int8-w-int16-a",
    show_default=True,
)
@click.option("--calibration", default="wikitext-2", show_default=True)
@click.option("--seq-len", type=int, default=2048, show_default=True)
@click.option("--samples", type=int, default=128, show_default=True)
def convert_cmd(
    hf_id: str,
    output: str,
    quant: str,
    calibration: str,
    seq_len: int,
    samples: int,
) -> None:
    """Convert HF_ID (e.g. microsoft/Phi-3.5-mini-instruct) to a hexrun model."""
    opts = ConvertOptions(
        hf_id=hf_id,
        output=output,
        quant=quant,
        calibration=calibration,
        seq_len=seq_len,
        samples=samples,
    )
    try:
        convert(opts)
    except NotImplementedError as e:
        click.echo(f"hex-convert: {e}", err=True)
        sys.exit(2)


main.add_command(convert_cmd, name="convert")


if __name__ == "__main__":
    main()
