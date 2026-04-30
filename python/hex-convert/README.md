# hex-convert

Sidecar tool that turns HuggingFace LLMs into hexrun-compatible
bundles. Three subcommands; you'll mostly use `manifest` and `inspect`.

```
hex-convert manifest --model-dir <dir> --bundle-dir <bundle> --name <slug>
hex-convert inspect <bundle-or-manifest>
hex-convert export <slug> --output <dir>      # heavy: requires AI Hub token
```

## When to use which

- **You already have a Genie bundle** (downloaded from Qualcomm's
  HuggingFace, or built earlier with `qai-hub-models`) and just want
  the runtime to load it. Run `hex-convert manifest` to drop a
  `hexrun.json` next to it. Done.
- **You want to verify a hexrun bundle's integrity** or see what's in
  it without parsing JSON by hand. Run `hex-convert inspect`.
- **You want to add a new model that's not in the built-in registry**
  and you have an AI Hub API token + an x64 Python venv + a few
  spare hours. Run `hex-convert export`.

## Install

`manifest` and `inspect` are lightweight (just `click` + `rich`) and
work fine on Windows ARM64:

```powershell
py -3.11 -m venv .venv
.\.venv\Scripts\Activate.ps1
pip install -e .
```

`export` pulls in `qai-hub-models` and the ONNX/transformers stack,
which doesn't have ARM64 wheels today. Use **x64 Python under
Prism emulation**:

```powershell
# in a separate venv
py -3.11-64 -m venv .venv-x64
.\.venv-x64\Scripts\Activate.ps1
pip install -e ".[export]"
```

## `hex-convert manifest`

```powershell
hex-convert manifest `
    --model-dir C:\AAA\Personal\AI\models\phi-3.5-mini `
    --bundle-dir C:\AAA\Personal\AI\models\phi-3.5-mini\bundle\phi_3_5_mini_instruct-genie-w4a16-qualcomm_snapdragon_x_elite `
    --name phi-3.5-mini
```

What it does:

1. Reads `genie_config.json` to pick up `dialog.context.size` (context
   length) and `dialog.context.n-vocab` (tokenizer vocab size).
2. Sniffs `arch` from the bundle directory name (`phi`, `llama`, `qwen`)
   or accepts an explicit `--arch`.
3. Sniffs `quant` similarly (`w4a16`, `w8a16`, `int8-w-int16-a`, ...) or
   accepts `--quant`.
4. Looks up a chat template by `arch` (Phi 3, Llama 3, Qwen 2.5
   patterns ship as defaults).
5. Walks the bundle directory and computes sha256 of every file.
6. Writes `<model-dir>/hexrun.json`.

The Rust runtime then validates this manifest and loads the bundle.

## `hex-convert inspect`

```powershell
hex-convert inspect C:\AAA\Personal\AI\models\phi-3.5-mini
hex-convert inspect C:\path\to\unmanaged-genie-bundle\
hex-convert inspect path\to\hexrun.json --no-verify
```

If the path contains a `hexrun.json`, it pretty-prints the manifest,
lists the files, and verifies every sha256 (skip with `--no-verify`).
If it only contains a `genie_config.json`, it prints what a manifest
*would* say and tells you the command to write one.

## `hex-convert export`

```powershell
$env:QAI_HUB_API_TOKEN = "..."   # https://aihub.qualcomm.com -> Settings -> API token
hex-convert export phi-3.5-mini --output C:\AAA\Personal\AI\models\phi-3.5-mini-fresh
```

Shells out to `python -m qai_hub_models.models.<model>.export`, which:

- Submits the model to Qualcomm AI Hub's cloud compile + link
  pipeline (real Snapdragon X Elite hardware in their farm).
- Waits for the job to finish (typically 30-90 minutes, depending on
  load and model size).
- Downloads the resulting Genie bundle to `--output`.
- Then `hex-convert export` chains into `manifest` to drop a
  `hexrun.json` on top.

Use `--skip-compile` to re-run only the manifest step against a bundle
the cloud compile already produced - useful if the network died after
the upload completed.

Known slugs (curated to match the runtime's built-in registry):
`phi-3.5-mini`, `llama-v3-1-8b-instruct`, `qwen-2-5-7b`. Adding new
ones is a matter of adding a recipe to `hex_convert/export.py`; see
the existing entries.

### Caveats

- An AI Hub token is mandatory. We don't have a fallback.
- Cloud compile fails sometimes; the AI Hub UI shows the job logs.
- The `qai-hub-models` model-cache (under `~/.qaihm/`) is your friend
  - once a model has been uploaded once, subsequent compiles skip the
  multi-GB upload step.
- Downstream of `export`, the bundle is identical in shape to what
  `hexrun pull` lands. The two flows produce interchangeable
  on-disk layouts.

## Tests

```powershell
pip install -e ".[test]"
pytest
```

The tests synthesize fake Genie bundles in `tmp_path` and verify
manifest emission. They don't touch the network or AI Hub.
