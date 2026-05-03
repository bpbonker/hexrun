#!/usr/bin/env bash
# One-shot setup for the qai-hub-models LLM export venv on Linux x86_64.
# Tested target: WSL2 Ubuntu 22.04 with python3.10 (the default on 22.04).
#
# Creates ./venv-export next to this script and pip-installs the pinned
# requirements. Re-run safely; existing venv is reused.
#
# After setup, configure AI Hub auth once:
#   ./venv-export/bin/qai-hub configure --api_token <YOUR_TOKEN>
# Token at https://app.aihub.qualcomm.com/account/api

set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="${VENV:-$HERE/venv-export}"

# AIMET-ONNX wheel only ships for python3.10. Pin to that.
PY="${PY:-python3.10}"

if ! command -v "$PY" >/dev/null; then
  cat >&2 <<EOF
ERROR: $PY not found.

On Ubuntu 22.04: it should be the default. If missing:
  sudo apt-get update
  sudo apt-get install -y python3.10 python3.10-venv python3.10-dev build-essential

On a non-22.04 Ubuntu:
  sudo add-apt-repository ppa:deadsnakes/ppa
  sudo apt-get install -y python3.10 python3.10-venv python3.10-dev
EOF
  exit 1
fi

if [ ! -d "$VENV" ]; then
  echo "[setup] Creating venv at $VENV"
  "$PY" -m venv "$VENV"
fi

echo "[setup] Upgrading pip"
"$VENV/bin/python" -m pip install --upgrade pip wheel setuptools

echo "[setup] Installing pinned requirements (this may take a few minutes)"
"$VENV/bin/python" -m pip install -r "$HERE/requirements.txt"

echo
echo "[setup] Verifying installed versions:"
"$VENV/bin/python" - <<'PY'
import importlib
mods = ["torch", "torchvision", "transformers", "sentencepiece",
        "accelerate", "onnx", "onnxruntime", "qai_hub", "qai_hub_models"]
for m in mods:
    try:
        mod = importlib.import_module(m)
        v = getattr(mod, "__version__", "(no __version__ attr)")
        print(f"  {m:18s} {v}")
    except Exception as e:
        print(f"  {m:18s} MISSING ({e.__class__.__name__})")
try:
    import aimet_onnx
    v = getattr(aimet_onnx, "__version__", "(no __version__ attr)")
    print(f"  {'aimet_onnx':18s} {v}")
except Exception:
    print(f"  {'aimet_onnx':18s} not installed (Linux+py3.10 only; needed for non-DEFAULT_W4A16)")
PY

echo
cat <<EOF
[setup] Done.

Next steps:
  1. Configure AI Hub auth:
       $VENV/bin/qai-hub configure --api_token <YOUR_TOKEN>
     Get a token at https://app.aihub.qualcomm.com/account/api

  2. (Optional) For HF-gated models like Llama, set HF_TOKEN:
       export HF_TOKEN=<your_hf_token>

  3. Run an export:
       bash $HERE/run-export.sh qwen3_4b_instruct_2507 4096,8192,16384,32768
EOF
