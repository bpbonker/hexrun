"""Inspect what's actually on the user's Qualcomm AI Hub workspace.

Lists recent jobs and uploaded models so we can see what we can salvage
without re-uploading 30 GB of model shards.
"""

import os
import sys

os.environ.setdefault("PYTHONIOENCODING", "utf-8")
os.environ.setdefault("PYTHONUTF8", "1")

import qai_hub as hub

print("=== Recent jobs (most recent first) ===")
try:
    jobs = hub.get_job_summaries(limit=30)
    for j in jobs:
        # Different attribute names depending on version
        jid = getattr(j, "job_id", None) or getattr(j, "id", None) or "?"
        name = getattr(j, "name", None) or "(no name)"
        status = getattr(j, "status", None)
        status_str = (
            getattr(status, "code", None)
            or getattr(status, "value", None)
            or str(status)
        )
        jtype = type(j).__name__
        print(f"  {jid:14s}  {jtype:18s}  {status_str:14s}  {name}")
except Exception as e:
    print(f"  (failed to list jobs: {e})", file=sys.stderr)

print()
print("=== Available qai-hub list functions ===")
print([n for n in dir(hub) if "model" in n.lower() or "summar" in n.lower()])

print()
print("=== Recent uploaded models (try a few names) ===")
for fn_name in ("get_models", "get_model", "list_models"):
    fn = getattr(hub, fn_name, None)
    if fn is not None:
        print(f"  trying {fn_name}()...")
        try:
            ms = fn()
            print(f"    -> {type(ms).__name__}")
            for m in (ms if hasattr(ms, "__iter__") else [ms]):
                mid = getattr(m, "model_id", None) or getattr(m, "id", None) or "?"
                name = getattr(m, "name", None) or "(no name)"
                print(f"    {mid}  {name}")
        except Exception as e:
            print(f"    error: {e}")

print()
print("=== Detail on first job (if any) ===")
try:
    jobs = hub.get_job_summaries(limit=1)
    if jobs:
        first = hub.get_job(getattr(jobs[0], "job_id", None) or getattr(jobs[0], "id", None))
        print(f"  type: {type(first).__name__}")
        print(f"  status: {first.get_status()}")
        for attr in ("device", "target_runtime", "shape", "options"):
            v = getattr(first, attr, None)
            if v is not None:
                print(f"  {attr}: {v}")
except Exception as e:
    print(f"  (failed to get job detail: {e})", file=sys.stderr)
