#!/usr/bin/env bash
# Run a qai-hub-models LLM export inside the WSL Linux x86 venv.
# Mirrors the export flags we use on Windows ARM, but runs on hardware
# that actually has the RAM headroom.
#
# Args:
#   $1 = model_id      e.g. qwen3_4b_instruct_2507
#   $2 = ctx_lengths   e.g. 4096,8192,16384,32768  (default: 4096)
#   $3 = output_dir    (default: ./build/<model_id>)
#
# Required: AI Hub token configured via `qai-hub configure` (run once).
# For HF-gated models (Llama, etc.), also: export HF_TOKEN=<token>
#
# Examples:
#   bash run-export.sh qwen3_4b_instruct_2507 32768
#   bash run-export.sh qwen3_4b_instruct_2507 4096,8192,16384,32768
#   bash run-export.sh llama_v3_2_1b_instruct 4096

set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VENV="${VENV:-$HERE/venv-export}"

MODEL="${1:?model_id required, e.g. qwen3_4b_instruct_2507}"
CTX="${2:-4096}"
OUT="${3:-./build/$MODEL}"

if [ ! -x "$VENV/bin/python" ]; then
  echo "ERROR: venv not found at $VENV. Run setup.sh first." >&2
  exit 1
fi

mkdir -p "$OUT"
LOG="$OUT/export.log"

# Sanity: AI Hub auth must be configured.
if ! "$VENV/bin/qai-hub" config 2>/dev/null | grep -q api_token; then
  cat >&2 <<EOF
ERROR: AI Hub not configured. Run once:
  $VENV/bin/qai-hub configure --api_token <YOUR_TOKEN>
Token at https://app.aihub.qualcomm.com/account/api
EOF
  exit 1
fi

echo "[export] model        : $MODEL"
echo "[export] ctx lengths  : $CTX"
echo "[export] output dir   : $OUT"
echo "[export] log          : $LOG"
echo "[export] starting     : $(date -Is)"

# UTF-8 IO so stdout/stderr don't choke on tqdm or model card emoji.
PYTHONIOENCODING=utf-8 PYTHONUTF8=1 \
  "$VENV/bin/python" -u -m "qai_hub_models.models.$MODEL.export" \
    --target-runtime genie \
    --chipset qualcomm-snapdragon-x-elite \
    --context-length "$CTX" \
    --output-dir "$OUT" \
    --model-cache-mode enable \
    --synchronous \
    --skip-profiling \
    --skip-inferencing \
    --zip-assets \
    2>&1 | tee "$LOG"

rc="${PIPESTATUS[0]}"
echo "[export] finished     : $(date -Is) rc=$rc"
exit "$rc"
