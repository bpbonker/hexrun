"""ORT-QNN-vs-Genie full-pipeline probe harness for npurun.

**Currently blocked.** `onnxruntime-genai`'s published wheels (PyPI stable
0.13.1 and ORT-Nightly 0.13.0.dev20260402 as of 2026-05-01) are not
compiled with `--use_qnn`; the runtime parses the bundle's
`genai_config.json`, sees the `qnn` provider request, and raises
`RuntimeError: QNN execution provider is not supported in this build`.
Getting QNN-enabled ort-genai requires building from source on Windows
ARM64. Until that wheel exists, this harness cannot run end-to-end.

Use [`bench-ort-qnn-microbench.py`](bench-ort-qnn-microbench.py) for the
working per-stage HTP latency probe. Results in
[`docs/findings_ort_vs_genie.md`](../docs/findings_ort_vs_genie.md).

---

Mirrors `npurun bench` shape so the two output CSVs diff cleanly:
    model,prompt,repeat,ctx,ttft_ms,total_ms,gen_ms,tokens,tps_post_ttft

Loads `llmware/phi-3.5-onnx-qnn` (or whatever bundle is at
`%LOCALAPPDATA%\\npurun\\models\\phi-3.5-mini-ort`) via onnxruntime-genai
with the QNN execution provider plugin registered. Runs the same four
built-in prompts as `BENCH_PROMPTS` in `crates/npurun-cli/src/main.rs`,
applies the same Phi-3 chat template, and times per-query TTFT + total
+ post-TTFT tok/s. Approximate token count uses the same `words * 1.3`
fudge as the Rust harness so the columns line up.

This is a feasibility probe, not a production benchmark — see
`docs/specula_followups.md` Item 5 for the broader context.
"""

from __future__ import annotations

import argparse
import csv
import os
import sys
import time
from pathlib import Path

# Register the QNN EP plugin before importing ort-genai so the Model
# loader can pick it up. ort-genai resolves the EP from the model's
# genai_config.json (`provider: QNN`) at construction time.
import onnxruntime as ort
import onnxruntime_qnn as oq

ort.register_execution_provider_library(oq.EP_NAME, oq.get_library_path())

import onnxruntime_genai as og  # noqa: E402  (must follow EP registration)


BENCH_PROMPTS = [
    "Write a one-line joke about Snapdragon laptops.",
    "Briefly explain why an NPU is more energy-efficient than a CPU for matrix multiplication.",
    "List three reasons running language models locally on a laptop is useful.",
    "What is 17 multiplied by 23? Just the number.",
]

# Matches `crates/npurun-cli/src/main.rs::approx_token_count`.
def approx_token_count(s: str) -> int:
    words = len(s.split())
    return round(words * 1.3)


# Same wrap shape as the Phi 3.5 Mini Genie bundle's chat template.
def wrap_phi3(prompt: str) -> str:
    return (
        "<|system|>\nYou are a helpful assistant.<|end|>\n"
        f"<|user|>\n{prompt}<|end|>\n"
        "<|assistant|>\n"
    )


def run_bench(
    model_dir: Path,
    prompts: list[str],
    repeats: int,
    skip_first: bool,
    csv_path: Path | None,
    max_new_tokens: int,
) -> None:
    print(f"==  ort-qnn bench: {model_dir.name}  ==", file=sys.stderr)
    load_started = time.perf_counter()
    model = og.Model(str(model_dir))
    tokenizer = og.Tokenizer(model)
    load_elapsed = time.perf_counter() - load_started
    print(f"[bundle loaded in {load_elapsed:.2f}s]", file=sys.stderr)

    csv_writer = None
    csv_file = None
    if csv_path is not None:
        new_file = not csv_path.exists()
        csv_file = open(csv_path, "a", newline="", encoding="utf-8")
        csv_writer = csv.writer(csv_file)
        if new_file:
            csv_writer.writerow(
                [
                    "model",
                    "prompt",
                    "repeat",
                    "ctx",
                    "ttft_ms",
                    "total_ms",
                    "gen_ms",
                    "tokens",
                    "tps_post_ttft",
                ]
            )

    runs: list[dict] = []
    total_runs = len(prompts) * max(repeats, 1)
    print(f"[running {total_runs} queries]\n", file=sys.stderr)

    idx = 0
    for repeat_idx in range(max(repeats, 1)):
        for prompt in prompts:
            idx += 1
            wrapped = wrap_phi3(prompt)
            input_tokens = tokenizer.encode(wrapped)

            params = og.GeneratorParams(model)
            params.set_search_options(
                max_length=len(input_tokens) + max_new_tokens,
                do_sample=False,
            )
            generator = og.Generator(model, params)
            generator.append_tokens(input_tokens)

            tokenizer_stream = tokenizer.create_stream()
            output = ""
            ttft_at = None
            tokens_generated = 0
            started = time.perf_counter()

            while not generator.is_done():
                generator.generate_next_token()
                new_token = generator.get_next_tokens()[0]
                if ttft_at is None:
                    ttft_at = time.perf_counter() - started
                piece = tokenizer_stream.decode(new_token)
                output += piece
                tokens_generated += 1
                if tokens_generated >= max_new_tokens:
                    break

            total = time.perf_counter() - started
            ttft = ttft_at if ttft_at is not None else total
            gen_time = max(total - ttft, 0.0)
            approx_tokens = approx_token_count(output)
            tps_post = approx_tokens / gen_time if gen_time > 0 else 0.0

            print(f"--- query {idx} ---")
            print(f"    prompt: {prompt}")
            print(f"    response ({approx_tokens} approx tokens): {output.strip()}")
            print(
                f"    total: {total*1000:.2f}ms   ttft: {ttft*1000:.2f}ms   "
                f"gen: {gen_time*1000:.2f}ms   tok/s post-ttft: {tps_post:.1f}"
            )
            print()

            runs.append({"total": total, "ttft": ttft, "gen": gen_time, "tokens": approx_tokens})

            if csv_writer is not None:
                csv_writer.writerow(
                    [
                        model_dir.name,
                        prompt,
                        repeat_idx + 1,
                        4096,  # llmware bundle ships single ctx tier
                        f"{ttft*1000:.3f}",
                        f"{total*1000:.3f}",
                        f"{gen_time*1000:.3f}",
                        approx_tokens,
                        f"{tps_post:.3f}",
                    ]
                )
                csv_file.flush()

            del generator

    if csv_file is not None:
        csv_file.close()

    warm = runs[1:] if skip_first and len(runs) > 1 else runs
    if not warm:
        return
    n = len(warm)
    avg_total = sum(r["total"] for r in warm) / n
    avg_ttft = sum(r["ttft"] for r in warm) / n
    avg_gen = sum(r["gen"] for r in warm) / n
    total_tokens = sum(r["tokens"] for r in warm)
    total_secs = sum(r["total"] for r in warm)
    total_gen_secs = sum(r["gen"] for r in warm)

    label = "warm summary (skipping first query)" if skip_first else "summary"
    print(f"==  {label}  ==")
    print(f"    queries:                  {n}")
    print(f"    avg total per query:      {avg_total*1000:.2f}ms")
    print(f"    avg time-to-first-token:  {avg_ttft*1000:.2f}ms")
    print(f"    avg generation time:      {avg_gen*1000:.2f}ms")
    print(f"    aggregate tok/s (incl ttft): {total_tokens/total_secs:.1f}")
    print(
        f"    aggregate tok/s (post ttft): "
        f"{total_tokens/total_gen_secs if total_gen_secs > 0 else 0:.1f}"
    )


def main() -> int:
    default_dir = (
        Path(os.environ["LOCALAPPDATA"]) / "npurun" / "models" / "phi-3.5-mini-ort"
    )
    parser = argparse.ArgumentParser(description="ORT-QNN-vs-Genie probe harness.")
    parser.add_argument("--model-dir", type=Path, default=default_dir)
    parser.add_argument("--prompt", type=str, default=None)
    parser.add_argument("--repeats", type=int, default=1)
    parser.add_argument(
        "--no-skip-first", dest="skip_first", action="store_false", default=True
    )
    parser.add_argument("--csv", type=Path, default=None)
    parser.add_argument("--max-new-tokens", type=int, default=128)
    args = parser.parse_args()

    if not args.model_dir.is_dir():
        print(f"error: {args.model_dir} does not exist", file=sys.stderr)
        return 2

    prompts = [args.prompt] if args.prompt else BENCH_PROMPTS
    run_bench(
        args.model_dir,
        prompts,
        args.repeats,
        args.skip_first,
        args.csv,
        args.max_new_tokens,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
