"""Pretty-print and verify hexrun bundles and Genie bundles.

``hex-convert inspect`` is the diagnostic counterpart to ``manifest``:
read what a bundle / manifest claims to contain, and check that what's
on disk matches.

Three modes, picked from the path you point at:

- A directory containing ``hexrun.json``: full manifest read,
  sha256 verify, list of files with sizes.
- A directory containing ``genie_config.json`` but no
  ``hexrun.json``: report what we'd write a manifest with.
- A file path ending in ``hexrun.json``: same as the first mode.
"""

from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable
from pathlib import Path

from rich.console import Console
from rich.table import Table


def inspect(target: Path, *, verify: bool = True) -> int:
    """Inspect a bundle or manifest. Returns exit code (0 on success)."""
    console = Console()

    if target.is_file() and target.name == "hexrun.json":
        return _inspect_manifest(target.parent, target, console=console, verify=verify)

    if target.is_dir():
        manifest = target / "hexrun.json"
        if manifest.is_file():
            return _inspect_manifest(target, manifest, console=console, verify=verify)
        # Walk one or two levels for a genie_config.json — the typical
        # qai-hub-models output lives one level below the model dir.
        cfg = _find_first(target, "genie_config.json", max_depth=3)
        if cfg is None:
            console.print(
                f"[red]no hexrun.json or genie_config.json found under {target}[/red]"
            )
            return 1
        return _inspect_genie_only(cfg, console=console)

    console.print(f"[red]not a file or directory: {target}[/red]")
    return 1


def _find_first(root: Path, name: str, *, max_depth: int = 3) -> Path | None:
    """Breadth-first search for the first file named `name` under root."""
    queue: list[tuple[Path, int]] = [(root, 0)]
    while queue:
        d, depth = queue.pop(0)
        if depth > max_depth:
            continue
        try:
            entries = list(d.iterdir())
        except OSError:
            continue
        for e in entries:
            if e.is_file() and e.name == name:
                return e
        for e in entries:
            if e.is_dir():
                queue.append((e, depth + 1))
    return None


def _inspect_manifest(
    model_dir: Path,
    manifest_path: Path,
    *,
    console: Console,
    verify: bool,
) -> int:
    with manifest_path.open("r", encoding="utf-8") as fh:
        m = json.load(fh)

    console.print(f"[bold]manifest:[/bold]   {manifest_path}")
    _print_kv(console, "name",       m.get("name"))
    _print_kv(console, "version",    m.get("version"))
    _print_kv(console, "arch",       m.get("arch"))
    _print_kv(console, "vocab",      m.get("vocab"))
    _print_kv(console, "context",    m.get("context"))
    _print_kv(console, "quant",      m.get("quant"))
    _print_kv(console, "qnn_sdk",    m.get("qnn_sdk"))

    files = m.get("files", {})
    if files:
        console.print()
        console.print("[bold]files:[/bold]")
        t = Table(show_header=True, header_style="bold")
        t.add_column("key")
        t.add_column("path")
        t.add_column("size", justify="right")
        for k, rel in files.items():
            full = model_dir / rel
            size = _fmt_size(full.stat().st_size) if full.is_file() else "[red]MISSING[/red]"
            t.add_row(k, rel, size)
        console.print(t)

    chat = m.get("chat_template")
    if chat:
        console.print()
        console.print("[bold]chat_template:[/bold]")
        _print_kv(console, "  system_prompt",   _ellipsis(chat.get("system_prompt", "")))
        _print_kv(console, "  template",        _ellipsis(chat.get("template", "")))
        _print_kv(console, "  assistant_turn",  _ellipsis(chat.get("assistant_turn", "")))
        _print_kv(console, "  next_user_turn",  _ellipsis(chat.get("next_user_turn", "")))

    sha = m.get("sha256", {}) or {}
    if not sha:
        console.print()
        console.print("[yellow]no sha256 entries — manifest cannot be integrity-verified[/yellow]")
        return 0

    if not verify:
        console.print()
        console.print(f"[dim]{len(sha)} sha256 entries; skipped verification[/dim]")
        return 0

    console.print()
    console.print(f"[bold]verifying sha256 of {len(sha)} files...[/bold]")
    bad = 0
    for rel, expected in sha.items():
        full = model_dir / rel
        if not full.is_file():
            console.print(f"  [red]MISSING[/red]  {rel}")
            bad += 1
            continue
        actual = _hash_file(full)
        if actual != expected:
            console.print(f"  [red]BAD    [/red]  {rel}")
            console.print(f"           expected {expected}")
            console.print(f"           got      {actual}")
            bad += 1
    if bad:
        console.print(f"[red]{bad} of {len(sha)} files failed verification[/red]")
        return 2
    console.print(f"[green]all {len(sha)} files verified[/green]")
    return 0


def _inspect_genie_only(cfg_path: Path, *, console: Console) -> int:
    """Show what we'd put in a manifest if we generated one for this bundle."""
    with cfg_path.open("r", encoding="utf-8") as fh:
        cfg = json.load(fh)
    bundle_dir = cfg_path.parent

    console.print(
        f"[yellow]no hexrun.json — showing inferred manifest for the Genie bundle at "
        f"{bundle_dir}[/yellow]"
    )
    console.print()
    try:
        ctx = cfg["dialog"]["context"]
        context = int(ctx["size"])
        vocab = int(ctx.get("n-vocab", ctx.get("n_vocab", 0)))
    except (KeyError, TypeError):
        console.print("[red]genie_config.json is missing dialog.context.size / n-vocab[/red]")
        return 1

    _print_kv(console, "bundle dir",     str(bundle_dir))
    _print_kv(console, "context",        context)
    _print_kv(console, "vocab",          vocab)

    files = sorted(p for p in bundle_dir.rglob("*") if p.is_file())
    if files:
        console.print()
        console.print("[bold]files in bundle:[/bold]")
        t = Table(show_header=True, header_style="bold")
        t.add_column("path")
        t.add_column("size", justify="right")
        for f in files:
            t.add_row(str(f.relative_to(bundle_dir)), _fmt_size(f.stat().st_size))
        console.print(t)

    console.print()
    console.print(
        "[dim]write a hexrun.json with: hex-convert manifest --bundle-dir "
        f"{bundle_dir} --name <slug>[/dim]"
    )
    return 0


def _print_kv(console: Console, k: str, v: object) -> None:
    console.print(f"  [bold]{k:<14}[/bold] {v}")


def _ellipsis(s: str, *, width: int = 80) -> str:
    s = s.replace("\n", "\\n")
    return s if len(s) <= width else s[: width - 1] + "..."


def _fmt_size(b: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if b < 1024 or unit == "GB":
            return f"{b:.1f} {unit}" if unit != "B" else f"{b} B"
        b /= 1024
    return f"{b:.1f} GB"  # unreachable, satisfies the type checker


def _hash_file(path: Path, *, chunk_size: int = 1024 * 1024) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        while True:
            buf = fh.read(chunk_size)
            if not buf:
                break
            h.update(buf)
    return h.hexdigest()


__all__: Iterable[str] = ("inspect",)
