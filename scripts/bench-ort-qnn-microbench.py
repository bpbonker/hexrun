"""ORT-QNN per-stage HTP-execution microbenchmark for npurun.

Times a single forward pass of each of the 8 QNN-EP-bound shards in the
`llmware/phi-3.5-onnx-qnn` pipeline (4 prompt-processors at ar128 +
4 token-generators at ar1), sums them to get a per-prefill and
per-decode-step lower bound, and compares against the libGenie numbers
this same hardware produces via `npurun bench`.

This is not a full LLM benchmark — there's no tokenizer, sampler, or
inter-stage data transfer modelled. What it measures is the quantity
specula's +19/+39% finding is about: how fast does the QNN HTP
*actually execute* a context binary, decoupled from the orchestration
runtime sitting on top.

Loads each shard as a separate `onnxruntime.InferenceSession` with
QNN EP and the same provider options the bundle's genai_config.json
declares. Synthesises zero-filled tensors of the shapes the session
declares on its inputs (HTP doesn't have data-dependent paths so
zeros are fine for timing). Runs warmup + measurement passes,
reports median latency per shard plus aggregate sums.

Output goes to stdout as a small report and (optionally) to a CSV the
same shape as `npurun bench --csv`. Run from `scripts\\dev-shell.bat`
so QnnHtp.dll resolves.
"""

from __future__ import annotations

import argparse
import csv
import os
import statistics
import sys
import time
from pathlib import Path

import numpy as np
import onnxruntime as ort
import onnxruntime_qnn as oq

ort.register_execution_provider_library(oq.EP_NAME, oq.get_library_path())

# These mirror the QNN provider options the bundle's genai_config.json
# declares for every QNN shard. We pass them on every session so the
# HTP runs in the same mode npurun's libGenie configures.
QNN_PROVIDER_OPTIONS = {
    "backend_path": "QnnHtp.dll",
    "htp_performance_mode": "burst",
    "enable_htp_shared_memory_allocator": "1",
    "qnn_context_priority": "high",
}

PROMPT_PROCESSORS = [
    "ar128_cl4096_1_of_4_qnn_ctx.onnx",
    "ar128_cl4096_2_of_4_qnn_ctx.onnx",
    "ar128_cl4096_3_of_4_qnn_ctx.onnx",
    "ar128_cl4096_4_of_4_qnn_ctx.onnx",
]
TOKEN_GENERATORS = [
    "ar1_cl4096_1_of_4_qnn_ctx.onnx",
    "ar1_cl4096_2_of_4_qnn_ctx.onnx",
    "ar1_cl4096_3_of_4_qnn_ctx.onnx",
    "ar1_cl4096_4_of_4_qnn_ctx.onnx",
]


_NPU_DEVICE_CACHE: list | None = None


def _qnn_npu_device():
    """The single QNN HTP NPU device on this machine. Cached after first probe.
    `onnxruntime-qnn` exposes three QNN devices (NPU/GPU/CPU); we pick NPU."""
    global _NPU_DEVICE_CACHE
    if _NPU_DEVICE_CACHE is None:
        _NPU_DEVICE_CACHE = [
            d
            for d in ort.get_ep_devices()
            if d.ep_name == "QNNExecutionProvider"
            and str(d.device.type).endswith("NPU")
        ]
        if not _NPU_DEVICE_CACHE:
            raise RuntimeError("no QNN HTP NPU device discovered on this machine")
    return _NPU_DEVICE_CACHE


def make_session(onnx_path: Path) -> ort.InferenceSession:
    """Load `onnx_path` as an InferenceSession bound to the QNN HTP NPU.
    Uses the v1.25 plugin-EP API (`add_provider_for_devices`) — the legacy
    `providers=[("QNNExecutionProvider", ...)]` pathway is rejected by
    `onnxruntime-qnn` 2.x because the EP is registered as a plugin."""
    opts = ort.SessionOptions()
    opts.add_provider_for_devices(_qnn_npu_device(), QNN_PROVIDER_OPTIONS)
    return ort.InferenceSession(str(onnx_path), sess_options=opts)


def synth_inputs(session: ort.InferenceSession) -> dict[str, np.ndarray]:
    """Build zero-filled tensors of every input's declared shape and dtype.
    QNN HTP has no data-dependent paths so zeros suffice for timing."""
    feeds = {}
    for inp in session.get_inputs():
        shape = []
        for d in inp.shape:
            if isinstance(d, int) and d > 0:
                shape.append(d)
            else:
                # Symbolic dim (None / "batch_size" / "seq_len"). Use 1 for
                # decode-step-style shards (ar1) and 128 for the prefill
                # tile (ar128). The session's filename tells us which.
                shape.append(1)
        dtype_map = {
            "tensor(float)": np.float32,
            "tensor(float16)": np.float16,
            "tensor(int64)": np.int64,
            "tensor(int32)": np.int32,
            "tensor(uint16)": np.uint16,
            "tensor(int8)": np.int8,
            "tensor(uint8)": np.uint8,
            "tensor(bool)": np.bool_,
        }
        np_dtype = dtype_map.get(inp.type, np.float32)
        feeds[inp.name] = np.zeros(shape, dtype=np_dtype)
    return feeds


def time_shard(
    onnx_path: Path,
    warmup: int,
    iters: int,
) -> tuple[float, float, float, list[float]]:
    """Time a single shard. Returns (median_ms, mean_ms, min_ms, all_samples)."""
    sess = make_session(onnx_path)
    feeds = synth_inputs(sess)
    output_names = [o.name for o in sess.get_outputs()]

    for _ in range(warmup):
        sess.run(output_names, feeds)

    samples: list[float] = []
    for _ in range(iters):
        t0 = time.perf_counter()
        sess.run(output_names, feeds)
        samples.append((time.perf_counter() - t0) * 1000.0)

    return (
        statistics.median(samples),
        statistics.mean(samples),
        min(samples),
        samples,
    )


def main() -> int:
    default_dir = (
        Path(os.environ["LOCALAPPDATA"]) / "npurun" / "models" / "phi-3.5-mini-ort"
    )
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--model-dir", type=Path, default=default_dir)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--iters", type=int, default=20)
    parser.add_argument("--csv", type=Path, default=None)
    args = parser.parse_args()

    if not args.model_dir.is_dir():
        print(f"error: {args.model_dir} does not exist", file=sys.stderr)
        return 2

    print(f"==  ort-qnn microbench: {args.model_dir.name}  ==")
    print(f"[warmup={args.warmup}  iters={args.iters}  HTP perf=burst  ctx priority=high]")
    print()

    csv_writer = None
    csv_file = None
    if args.csv is not None:
        new_file = not args.csv.exists()
        csv_file = open(args.csv, "a", newline="", encoding="utf-8")
        csv_writer = csv.writer(csv_file)
        if new_file:
            csv_writer.writerow(
                ["bundle", "shard", "stage", "median_ms", "mean_ms", "min_ms", "iters"]
            )

    pp_medians: list[float] = []
    print(f"--- prompt processors (ar128) ---")
    for shard in PROMPT_PROCESSORS:
        path = args.model_dir / shard
        median, mean, mn, _ = time_shard(path, args.warmup, args.iters)
        print(f"  {shard:55} median {median:7.2f} ms   mean {mean:7.2f} ms   min {mn:7.2f} ms")
        pp_medians.append(median)
        if csv_writer is not None:
            csv_writer.writerow(
                [args.model_dir.name, shard, "prompt-processor", f"{median:.3f}", f"{mean:.3f}", f"{mn:.3f}", args.iters]
            )
    print()

    tg_medians: list[float] = []
    print(f"--- token generators (ar1) ---")
    for shard in TOKEN_GENERATORS:
        path = args.model_dir / shard
        median, mean, mn, _ = time_shard(path, args.warmup, args.iters)
        print(f"  {shard:55} median {median:7.2f} ms   mean {mean:7.2f} ms   min {mn:7.2f} ms")
        tg_medians.append(median)
        if csv_writer is not None:
            csv_writer.writerow(
                [args.model_dir.name, shard, "token-generator", f"{median:.3f}", f"{mean:.3f}", f"{mn:.3f}", args.iters]
            )
    print()

    pp_total = sum(pp_medians)
    tg_total = sum(tg_medians)
    print(f"==  aggregate (sum of stage medians)  ==")
    print(f"    prefill (4 × ar128):        {pp_total:7.2f} ms  (= 1 prefill of up to 128 tokens)")
    print(f"    decode (4 × ar1):           {tg_total:7.2f} ms  (= 1 generated token)")
    if tg_total > 0:
        print(f"    implied steady-state:       {1000.0/tg_total:7.2f} tok/s  (raw HTP execution; no tokenizer/sampler)")
    print()

    print(f"==  comparison vs libGenie on this X1E (npurun bench)  ==")
    print( "    libGenie TTFT (ctx 4096):   ~110 ms  (full prefill)")
    print( "    libGenie per-token decode:  ~80 ms   (~12.5 tok/s post-TTFT)")
    print( "    libGenie reported tok/s:    ~12.7 tok/s post-TTFT (commit 9611c7b smoke)")
    print()
    print(f"    ORT-QNN per-token decode:   {tg_total:7.2f} ms")
    if tg_total > 0:
        print(f"    ratio (libGenie/ORT-QNN):   {80.0/tg_total:7.2f}x decode")
    if pp_total > 0:
        print(f"    ratio (libGenie/ORT-QNN):   {110.0/pp_total:7.2f}x prefill")

    if csv_file is not None:
        csv_file.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
