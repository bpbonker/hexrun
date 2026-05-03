#!/usr/bin/env bash
# One-shot: wait for VL-7B download → integrity-check → unzip →
# patch graph-switching → run qwen-bench. Logs to logs/vl7b-verify.log
# and writes a single-line summary to logs/vl7b-summary.txt at end.

set -u
exec >> logs/vl7b-verify.log 2>&1
echo "========== $(date) =========="

ZIP="${ZIP:-${TMPDIR:-/tmp}/qwen2_5_vl_7b.zip}"
DEST_ROOT="${DEST_ROOT:-$(pwd)/models/qwen-2-5-vl-7b-instruct}"
BUNDLE_PARENT="$DEST_ROOT/bundle"
SUMMARY="logs/vl7b-summary.txt"
TARGET_SIZE=4934152901   # exact full size from S3 Content-Length

# 1. Wait for the download-loop to bring the file up to TARGET_SIZE.
echo "[wait] watching file size of $ZIP (target $TARGET_SIZE)"
while :; do
  size=$(stat -c%s "$ZIP" 2>/dev/null || echo 0)
  if [ "$size" -ge "$TARGET_SIZE" ]; then
    echo "[wait] file reached $size >= $TARGET_SIZE"
    break
  fi
  # If the download-loop process has died and we're still short, fail
  # rather than spin forever. Git Bash has no pgrep, so use ps + grep -F.
  if ! ps -ef 2>/dev/null | grep -F "vl7b-download-loop.sh" | grep -v grep > /dev/null; then
    echo "[wait] download-loop is gone and file still short ($size); aborting"
    echo "FAIL: download-loop died at $size bytes" > "$SUMMARY"
    exit 8
  fi
  sleep 30
done

final_size=$(stat -c%s "$ZIP" 2>/dev/null || echo 0)
echo "[wait] final size: $final_size bytes"
if [ "$final_size" -lt 4900000000 ]; then
  echo "[FAIL] zip is short ($final_size bytes < 4.9 GB); aborting."
  echo "FAIL: short zip ($final_size bytes)" > "$SUMMARY"
  exit 2
fi

# 2. Integrity test.
echo "[test] running unzip -tq"
if ! unzip -tq "$ZIP" > /tmp/vl7b-unzip-test.log 2>&1; then
  echo "[FAIL] zip integrity check failed:"
  tail -20 /tmp/vl7b-unzip-test.log
  echo "FAIL: integrity test" > "$SUMMARY"
  exit 3
fi
echo "[test] integrity OK"

# 3. Unzip.
mkdir -p "$BUNDLE_PARENT"
echo "[unzip] extracting to $BUNDLE_PARENT"
rm -rf "$BUNDLE_PARENT"/*
if ! unzip -q "$ZIP" -d "$BUNDLE_PARENT"; then
  echo "[FAIL] unzip extract failed"
  echo "FAIL: unzip extract" > "$SUMMARY"
  exit 4
fi

# Locate the bundle dir (the zip extracts a top-level folder containing
# genie_config.json).
BUNDLE_DIR=$(find "$BUNDLE_PARENT" -maxdepth 2 -name genie_config.json -printf '%h\n' 2>/dev/null | head -1)
if [ -z "$BUNDLE_DIR" ]; then
  # Genie 1.17 bundles use text-generator.json as the wrapper config.
  BUNDLE_DIR=$(find "$BUNDLE_PARENT" -maxdepth 2 -name text-generator.json -printf '%h\n' 2>/dev/null | head -1)
fi
if [ -z "$BUNDLE_DIR" ]; then
  echo "[FAIL] no genie_config.json or text-generator.json found under $BUNDLE_PARENT"
  echo "FAIL: bundle dir not found" > "$SUMMARY"
  exit 5
fi
echo "[unzip] bundle: $BUNDLE_DIR"

# 4. Patch graph-switching via inline Python (json walks).
python_inline=$(cat <<'PY'
import json, os, sys
bundle = os.environ["BUNDLE_DIR"]
patched = []
for fname in ("genie_config.json", "text-generator.json"):
    p = os.path.join(bundle, fname)
    if not os.path.isfile(p):
        continue
    with open(p, "r", encoding="utf-8") as f:
        data = json.load(f)
    if not isinstance(data, dict):
        continue
    # Walk: root -> first wrapper -> engine -> backend -> QnnHtp
    wrap = next(iter(data.values())) if data else None
    if not isinstance(wrap, dict):
        continue
    eng = wrap.get("engine")
    be = eng.get("backend") if isinstance(eng, dict) else None
    htp = be.get("QnnHtp") if isinstance(be, dict) else None
    if isinstance(htp, dict):
        if htp.get("enable-graph-switching") is True:
            patched.append(f"{fname}: already-set")
            continue
        htp["enable-graph-switching"] = True
        with open(p, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
        patched.append(f"{fname}: injected")
    else:
        patched.append(f"{fname}: no QnnHtp block")
print("\n".join(patched))
PY
)
echo "[patch] enable-graph-switching: true"
BUNDLE_DIR="$BUNDLE_DIR" python/.venv-x64/Scripts/python.exe -c "$python_inline"
patch_rc=$?
if [ $patch_rc -ne 0 ]; then
  echo "[FAIL] patch script exit $patch_rc"
  echo "FAIL: patch script exit $patch_rc" > "$SUMMARY"
  exit 6
fi

# 5. Run qwen-bench against the bundle.
echo "[bench] launching qwen-bench"
NPURUN_BUNDLE="$BUNDLE_DIR" scripts/dev-shell.bat cargo run --release -p qnn --example qwen-bench > logs/vl7b-bench.log 2>&1
bench_rc=$?
echo "[bench] exit $bench_rc"

# Extract a tok/s line from the bench log if present, else last 30 lines.
if [ $bench_rc -ne 0 ]; then
  echo "FAIL: bench rc=$bench_rc; tail:" > "$SUMMARY"
  tail -30 logs/vl7b-bench.log >> "$SUMMARY"
  exit 7
fi

tps_line=$(grep -E -i "tok/s|tps|tokens/s" logs/vl7b-bench.log | tail -5)
echo "OK: VL-7B bench complete" > "$SUMMARY"
echo "bundle: $BUNDLE_DIR" >> "$SUMMARY"
echo "tok/s lines:" >> "$SUMMARY"
echo "$tps_line" >> "$SUMMARY"
echo "[done] summary written to $SUMMARY"
