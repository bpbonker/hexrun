#!/usr/bin/env bash
# Resilient downloader for the Qwen3-4B w4a16 prequantized asset.
# Mirrors scripts/vl7b-download-loop.sh: keeps issuing curl -C - resumes
# until the file hits the expected Content-Length. The qai-hub-models
# export step will see the cached file at this exact path and skip its
# own (non-resumable) download.

set -u
mkdir -p logs
exec >> logs/qwen3-4b-download.log 2>&1

ZIP="${ZIP:-$HOME/.qaihm/qai-hub-models/models/qwen3_4b_instruct_2507/v1/qwen34instruct2507_w4a16_adascale.zip}"
URL="https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/qwen3_4b_instruct_2507/v1/qwen34instruct2507_w4a16_adascale.zip"
TARGET_SIZE=17729097123  # exact Content-Length from S3 HEAD on 2026-05-02
MAX_ATTEMPTS=60
ATTEMPT_TIMEOUT=900      # 15 min per curl attempt

mkdir -p "$(dirname "$ZIP")"

attempt=1
while [ $attempt -le $MAX_ATTEMPTS ]; do
  size=$(stat -c%s "$ZIP" 2>/dev/null || echo 0)
  if [ "$size" -ge "$TARGET_SIZE" ]; then
    echo "[$(date +%H:%M:%S)] [download-OK] size=$size >= target=$TARGET_SIZE"
    exit 0
  fi
  remaining=$((TARGET_SIZE - size))
  echo "[$(date +%H:%M:%S)] [attempt $attempt/$MAX_ATTEMPTS] size=$size remaining=$remaining"
  curl -L -C - -m "$ATTEMPT_TIMEOUT" --connect-timeout 30 --speed-time 60 --speed-limit 50000 \
    -o "$ZIP" "$URL" \
    -w "\n[curl-w] http=%{http_code} downloaded=%{size_download} speed=%{speed_download} time=%{time_total}\n" \
    >> "logs/qwen3-4b-curl-$attempt.log" 2>&1
  rc=$?
  echo "[$(date +%H:%M:%S)] [attempt $attempt rc=$rc] (28=timeout, 56=conn-reset, 18=partial; retrying)"
  attempt=$((attempt + 1))
  sleep 5
done

echo "[$(date +%H:%M:%S)] [download-FAIL] gave up after $MAX_ATTEMPTS attempts; size=$(stat -c%s "$ZIP" 2>/dev/null || echo 0)"
exit 1
