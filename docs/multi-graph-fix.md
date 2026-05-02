# Multi-graph bundle fix: 20× decode speedup

## Summary

Newer Qualcomm Genie LLM bundles (Qwen3 onwards) ship with multi-graph
context binaries that contain both a prefill graph (`prompt_ar128_*`)
and a decode graph (`token_ar1_*`). libGenie 1.17.0's auto-switch
heuristic was written for the older naming convention (`ar128_*`,
`ar1_*`) and doesn't match these prefixed names. Without the explicit
`enable-graph-switching` flag in `genie_config.json`, Genie executes
the prefill graph for every output token. That's roughly 128× more
compute per decode token than the decode graph would do, and on
Snapdragon X1E it dropped Qwen3-4B's throughput to **5% of the
hardware ceiling**.

The fix is one line of JSON. `npurun pull` injects it automatically.

## Numbers

Bench: `cargo run --release -p qnn --example qwen-bench` against
Qwen3-4B-Instruct-2507 (w4a16) on Snapdragon X1E. Three warm
queries, first one discarded:

| Config | Avg gen time | tok/s post-TTFT |
|---|---:|---:|
| Default genie_config (broken) | 44.18 s | 0.6 |
| `+ enable-graph-switching: true` | 2.28 s | **11.7** |

11.7 tok/s matches Phi 3.5 Mini's measured NPU throughput on the same
hardware — the bandwidth-bound HMX ceiling. So the fix doesn't merely
unstick the runtime; it gets a 4 B model running at full hardware
speed, which is the ceiling we'd expect.

## Diagnosis

Watch Task Manager → Performance → NPU during decode. Symptoms of
the bug:

- Brief 100 % spike per output token, then drop to ~5 %
- 0.5–2 s wall-clock per token
- TTFT looks fine (~500 ms)
- Loaded bundle size and config look correct

What's actually happening: each decode step is running the 128-token
prefill graph (just to advance one position), so the NPU does a full
prefill burst, then sits idle while the host re-tokenises and
resubmits.

To verify the bundle has the multi-graph structure, dump the context
binary:

```powershell
qnn-context-binary-utility.exe `
    --context_binary qwen3_4b_instruct_2507_w4a16_part_1_of_4.bin `
    --json_file ctx1.json
```

Look for `info.graphs[].info.graphName`. If you see entries with
`prompt_ar128_*` and `token_ar1_*` prefixes, the bundle needs the
flag. If you see plain `ar128_*` / `ar1_*` (Phi-style), the flag is
already implicit.

## The fix

Add to the bundle's `genie_config.json` (and `text-generator.json`
if present), inside `engine.backend.QnnHtp`:

```json
"enable-graph-switching": true
```

The flag is documented in the QAIRT 2.45 SDK docs at
`Genie/general/library/dialog/json.html` under
`dialog::engine::backend::QnnHtp::enable-graph-switching`.

`npurun pull` runs the patcher
(`patch_genie_config_for_graph_switching`) after extraction. It walks
both `genie_config.json` and `text-generator.json`, locates the
`QnnHtp` block under any top-level wrapper, and inserts the flag if
it's missing. Idempotent — if the bundle author has already set the
flag (either way) the patcher leaves it alone.

If you're side-loading a bundle (manually placing files in
`%LOCALAPPDATA%\npurun\models\<name>\bundle\`), set the flag yourself.

## Why this matters

Qualcomm publishes precompiled bundles for a small set of models on
their public S3 bucket. Before this fix, only Phi 3.5 Mini was usable
end-to-end on npurun — every other Qwen3-family bundle was
bottlenecked at <1 tok/s even though the hardware was fully capable.

After the fix, `npurun pull qwen3-4b-instruct-2507` produces a
working chat bundle that runs at NPU ceiling speed.

## Caveat: not every bundle needs the flag

Measured on Qwen 2.5 VL-7B w4a16 (a multi-node bundle with
imageEncoder + lutEncoder + textGenerator):

| Config | tok/s post-TTFT | TTFT |
|---|---:|---:|
| flag OFF | 9.1 | 156 ms |
| flag ON  | 9.2 | 551 ms |

Decode rate is identical; flag-on adds ~400 ms to every prefill (the
graph swap penalty). VL-7B's LM is either single-graph or names its
graphs in a way libGenie's auto-switch heuristic recognises.

The current patcher injects the flag unconditionally. That's a small
TTFT regression on VL-7B and any future bundle in the same shape. A
follow-up should make injection conditional on actually finding
`prompt_*` / `token_*` in the bundle's graph names.
