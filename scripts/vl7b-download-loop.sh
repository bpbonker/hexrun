#!/usr/bin/env bash
# Resilient downloader for VL-7B. Keeps issuing curl -C - resumes until
# the file hits the expected size. Logs every attempt to logs/vl7b-download.log.

set -u
exec >> logs/vl7b-download.log 2>&1

ZIP="${ZIP:-${TMPDIR:-/tmp}/qwen2_5_vl_7b.zip}"
URL="https://qaihub-public-assets.s3.us-west-2.amazonaws.com/qai-hub-models/models/qwen2_5_vl_7b_instruct/releases/v0.52.0/qwen2_5_vl_7b_instruct-genie-w4a16-qualcomm_snapdragon_x_elite.zip"
TARGET_SIZE=4934152901   # exact full size from earlier curl line
MAX_ATTEMPTS=30
ATTEMPT_TIMEOUT=900      # 15 min per curl attempt — short enough to recover quickly from a stall

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
    >> "logs/vl7b-curl-$attempt.log" 2>&1
  rc=$?
  echo "[$(date +%H:%M:%S)] [attempt $attempt rc=$rc] (28=timeout, 56=conn-reset, 18=partial; retrying)"
  attempt=$((attempt + 1))
  sleep 5   # brief pause before retry
done

echo "[$(date +%H:%M:%S)] [download-FAIL] gave up after $MAX_ATTEMPTS attempts; size=$(stat -c%s "$ZIP" 2>/dev/null || echo 0)"
exit 1
