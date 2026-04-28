"""Phase 0 smoke test: prove ONNX Runtime QNN EP runs a real LLM on the NPU.

Run this **after** installing the QNN SDK and pulling a pre-converted
Qualcomm AI Hub model. Requires x64 Python with onnxruntime-qnn and
onnxruntime-genai installed.

Usage:
    python scripts/smoke_phase0.py --model C:\\path\\to\\Phi-3.5-mini-instruct-onnx \\
                                   --prompt "Tell me a joke."

DoD (per the plan):
    - Task Manager NPU column shows >0% sustained during inference.
    - Tokens/sec is reported and is >3x what CPU would do on the same model.
    - QnnHtp.log (with QNN_LOG_LEVEL=PROFILE) shows ops on HTP, not CPU.
"""

from __future__ import annotations

import argparse
import os
import sys
import time
from pathlib import Path


def _eprint(*a: object) -> None:
    print(*a, file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--model",
        required=True,
        type=Path,
        help="Directory containing the genai_builder ONNX bundle (genai_config.json + *.onnx)",
    )
    parser.add_argument("--prompt", default="Tell me a joke.")
    parser.add_argument("--max-tokens", type=int, default=128)
    parser.add_argument(
        "--ep",
        choices=["qnn", "cpu"],
        default="qnn",
        help="Execution provider. Use cpu for the >3x sanity-check baseline.",
    )
    args = parser.parse_args()

    if not args.model.is_dir():
        _eprint(f"--model is not a directory: {args.model}")
        return 2
    if not (args.model / "genai_config.json").is_file():
        _eprint(f"missing genai_config.json under {args.model}")
        _eprint("This script expects a Phi-3.5-mini-style genai_builder bundle.")
        return 2

    # Surface QNN profile logs so the user can confirm HTP execution.
    os.environ.setdefault("QNN_LOG_LEVEL", "PROFILE")

    try:
        import onnxruntime_genai as og  # type: ignore
    except ModuleNotFoundError as e:
        _eprint(f"missing dependency: {e}. Try: pip install onnxruntime-genai onnxruntime-qnn")
        return 2

    print(f"hexrun smoke test  ep={args.ep}  model={args.model}")

    config = og.Config(str(args.model))
    if args.ep == "qnn":
        try:
            config.append_provider("QNN")
        except Exception as e:  # noqa: BLE001
            _eprint(f"failed to register QNN execution provider: {e}")
            _eprint("Verify QNN_SDK_ROOT, the HTP driver, and onnxruntime-qnn install.")
            return 1

    model = og.Model(config)
    tokenizer = og.Tokenizer(model)
    tokenizer_stream = tokenizer.create_stream()

    chat_template = "<|user|>\n{prompt}<|end|>\n<|assistant|>\n"
    input_text = chat_template.format(prompt=args.prompt)
    input_tokens = tokenizer.encode(input_text)

    params = og.GeneratorParams(model)
    params.set_search_options(max_length=args.max_tokens, do_sample=False)
    generator = og.Generator(model, params)
    generator.append_tokens(input_tokens)

    print()
    print(f"prompt: {args.prompt}")
    print("response:")

    t0 = time.perf_counter()
    n_tokens = 0
    try:
        while not generator.is_done():
            generator.generate_next_token()
            new_token = generator.get_next_tokens()[0]
            sys.stdout.write(tokenizer_stream.decode(new_token))
            sys.stdout.flush()
            n_tokens += 1
    except KeyboardInterrupt:
        _eprint("\ninterrupted")
    elapsed = time.perf_counter() - t0
    print()
    print()
    if elapsed > 0 and n_tokens > 0:
        tps = n_tokens / elapsed
        print(f"generated {n_tokens} tokens in {elapsed:.2f}s  ->  {tps:.2f} tok/s")
    else:
        print("no tokens generated")
        return 1

    print()
    print("Verification checklist (do these manually):")
    print("  1. Task Manager > Performance > NPU should have shown sustained utilization.")
    print("  2. Look at QnnHtp.log near the working dir — ops should be running on HTP, not CPU.")
    print("  3. Re-run with --ep cpu and compare tokens/sec; NPU should be >=3x CPU baseline.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
