"""Stress-test the running npurun serve chat endpoint.

What it does:
  1. Long multi-turn conversation (15 turns), varied content + lengths.
  2. Memory-recall test (tells the model facts, asks them back later).
  3. Reset-on-error test (if a turn fails, can the next turn succeed?).
  4. Streaming + non-streaming round-trip.
  5. JSON mode round-trip.
  6. Concurrent-request test (should get one 200 + one 429).

Reports a clean PASS/FAIL summary at the end. Run while
`npurun serve --model phi-3.5-mini` is up on 127.0.0.1:11435.
"""

from __future__ import annotations

import json
import sys
import time
import urllib.request
import urllib.error
from concurrent.futures import ThreadPoolExecutor

BASE = "http://127.0.0.1:11435"
MODEL = "phi-3.5-mini"


def post_chat(messages, stream=False, max_tokens=128, response_format=None, timeout=120):
    body = {
        "model": MODEL,
        "messages": messages,
        "stream": stream,
        "max_tokens": max_tokens,
    }
    if response_format:
        body["response_format"] = response_format
    req = urllib.request.Request(
        f"{BASE}/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    started = time.perf_counter()
    if stream:
        # Read SSE, accumulate content
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                content = ""
                for line in resp:
                    line = line.decode("utf-8", errors="replace").rstrip("\r\n")
                    if not line.startswith("data:"):
                        continue
                    data = line[5:].strip()
                    if data == "[DONE]":
                        break
                    try:
                        chunk = json.loads(data)
                    except json.JSONDecodeError:
                        continue
                    delta = chunk.get("choices", [{}])[0].get("delta", {})
                    if delta.get("content"):
                        content += delta["content"]
                elapsed = time.perf_counter() - started
                return {"ok": True, "content": content, "elapsed": elapsed, "status": 200}
        except urllib.error.HTTPError as e:
            elapsed = time.perf_counter() - started
            body = e.read().decode("utf-8", errors="replace")
            return {"ok": False, "status": e.code, "body": body, "elapsed": elapsed}
        except Exception as e:
            elapsed = time.perf_counter() - started
            return {"ok": False, "status": 0, "body": str(e), "elapsed": elapsed}
    else:
        try:
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                body = json.loads(resp.read().decode("utf-8"))
                elapsed = time.perf_counter() - started
                content = body["choices"][0]["message"]["content"]
                return {"ok": True, "content": content, "elapsed": elapsed, "status": 200}
        except urllib.error.HTTPError as e:
            elapsed = time.perf_counter() - started
            body = e.read().decode("utf-8", errors="replace")
            return {"ok": False, "status": e.code, "body": body, "elapsed": elapsed}
        except Exception as e:
            elapsed = time.perf_counter() - started
            return {"ok": False, "status": 0, "body": str(e), "elapsed": elapsed}


def run_test(name, fn):
    print(f"\n[ TEST ] {name}", flush=True)
    started = time.perf_counter()
    try:
        result = fn()
        elapsed = time.perf_counter() - started
        if result is True or result is None:
            print(f"  PASS ({elapsed:.1f}s)")
            return True
        else:
            print(f"  FAIL: {result} ({elapsed:.1f}s)")
            return False
    except Exception as e:
        elapsed = time.perf_counter() - started
        print(f"  FAIL (exception): {e} ({elapsed:.1f}s)")
        return False


def test_single_turn():
    r = post_chat([{"role": "user", "content": "Reply with the single word OK and nothing else."}])
    if not r["ok"]:
        return f"status={r['status']} body={r.get('body', '')!r}"
    if "OK" not in r["content"]:
        return f"unexpected content: {r['content']!r}"
    print(f"    reply: {r['content']!r}  ({r['elapsed']:.2f}s)")
    return True


def test_streaming_turn():
    r = post_chat([{"role": "user", "content": "Count from 1 to 3, comma-separated, no other words."}], stream=True)
    if not r["ok"]:
        return f"status={r['status']} body={r.get('body', '')!r}"
    if not r["content"]:
        return "empty content from stream"
    print(f"    reply: {r['content'][:80]!r}  ({r['elapsed']:.2f}s)")
    return True


def test_multi_turn(turns=15):
    """Long conversation. After each turn, append assistant reply and ask a new question."""
    messages = [{"role": "system", "content": "You are concise. Answer in one short sentence."}]
    failures = []
    for i in range(turns):
        prompts = [
            "Pick a color, just the name.",
            "Pick a number from 1 to 100.",
            "Pick a fruit, just the name.",
            "What's 2+2?",
            "Name a city.",
            "Name an animal.",
            "Pick a programming language.",
            "Name a planet.",
            "Pick a vegetable.",
            "Name a sport.",
        ]
        user = prompts[i % len(prompts)] + f" (turn {i+1})"
        messages.append({"role": "user", "content": user})
        r = post_chat(messages, max_tokens=40)
        if not r["ok"]:
            failures.append(f"turn {i+1} ({len(messages)} msgs): status={r['status']} body={r.get('body', '')[:200]}")
            # Don't append a fake assistant reply; just keep going to test recovery
            messages.pop()  # drop the user that failed
            continue
        reply = r["content"].strip()
        print(f"    turn {i+1} (msgs={len(messages)}): {reply[:60]!r}  ({r['elapsed']:.2f}s)")
        messages.append({"role": "assistant", "content": reply})
    if failures:
        return f"{len(failures)}/{turns} turns failed:\n      " + "\n      ".join(failures)
    return True


def test_memory_recall():
    """Multi-turn where later turns must recall info from earlier turns."""
    messages = [
        {"role": "user", "content": "My favorite color is teal. Just acknowledge."},
    ]
    r = post_chat(messages, max_tokens=20)
    if not r["ok"]:
        return f"turn 1 failed: {r.get('body', '')!r}"
    messages.append({"role": "assistant", "content": r["content"].strip()})
    print(f"    turn 1: {r['content'].strip()!r}")

    messages.append({"role": "user", "content": "My favorite number is 47. Just acknowledge."})
    r = post_chat(messages, max_tokens=20)
    if not r["ok"]:
        return f"turn 2 failed: {r.get('body', '')!r}"
    messages.append({"role": "assistant", "content": r["content"].strip()})
    print(f"    turn 2: {r['content'].strip()!r}")

    messages.append({"role": "user", "content": "What is my favorite color? One word."})
    r = post_chat(messages, max_tokens=15)
    if not r["ok"]:
        return f"turn 3 (recall color) failed: {r.get('body', '')!r}"
    print(f"    turn 3 (recall color): {r['content'].strip()!r}  ({r['elapsed']:.2f}s)")
    if "teal" not in r["content"].lower():
        return f"didn't recall 'teal': {r['content']!r}"
    messages.append({"role": "assistant", "content": r["content"].strip()})

    messages.append({"role": "user", "content": "What is my favorite number? Just the digits."})
    r = post_chat(messages, max_tokens=15)
    if not r["ok"]:
        return f"turn 4 (recall number) failed: {r.get('body', '')!r}"
    print(f"    turn 4 (recall number): {r['content'].strip()!r}  ({r['elapsed']:.2f}s)")
    if "47" not in r["content"]:
        return f"didn't recall '47': {r['content']!r}"
    return True


def test_json_mode():
    r = post_chat(
        [{"role": "user", "content": "Output a JSON object with key \"answer\" and value 42. Nothing else."}],
        response_format={"type": "json_object"},
        max_tokens=30,
    )
    if not r["ok"]:
        return f"status={r['status']} body={r.get('body', '')!r}"
    print(f"    raw: {r['content']!r}")
    try:
        data = json.loads(r["content"].strip().lstrip("```json").rstrip("```"))
    except json.JSONDecodeError as e:
        return f"not parseable JSON: {e}"
    if data.get("answer") not in (42, "42"):
        return f"unexpected JSON content: {data}"
    return True


def test_concurrent_429():
    """Two concurrent requests; expect one 200 + one 429."""
    def hit():
        return post_chat(
            [{"role": "user", "content": "Reply with one word."}],
            max_tokens=20,
        )
    with ThreadPoolExecutor(max_workers=2) as ex:
        f1 = ex.submit(hit)
        f2 = ex.submit(hit)
        results = [f1.result(), f2.result()]
    statuses = sorted(r["status"] for r in results)
    print(f"    statuses: {statuses}")
    if statuses != [200, 429]:
        return f"expected [200, 429], got {statuses}"
    return True


def test_recovery_after_failure():
    """If a turn fails, the next turn should still succeed (dialog auto-reset)."""
    # Send something pathological that might trigger a failure: a very long
    # input. Phi 3.5 has 4096 ctx; pad close to it.
    big = "filler " * 700  # ~700 tokens
    r1 = post_chat(
        [{"role": "user", "content": big + " Now reply with the word HELLO."}],
        max_tokens=30,
    )
    print(f"    big request status={r1['status']}, elapsed={r1['elapsed']:.1f}s")
    # Whether it passed or failed, the next request should succeed.
    r2 = post_chat([{"role": "user", "content": "Reply with the single word RECOVERED."}], max_tokens=15)
    if not r2["ok"]:
        return f"recovery failed: status={r2['status']} body={r2.get('body', '')!r}"
    print(f"    recovery: {r2['content']!r}")
    return True


def main() -> int:
    print(f"==  npurun stress-chat  ==")
    print(f"  base:  {BASE}")
    print(f"  model: {MODEL}")

    # Probe health first.
    try:
        with urllib.request.urlopen(f"{BASE}/healthz", timeout=5) as resp:
            health = json.loads(resp.read().decode())
        print(f"  server: {health['status']}, model={health.get('model')}, uptime={health.get('uptime_seconds')}s")
    except Exception as e:
        print(f"FAIL: cannot reach {BASE}/healthz: {e}")
        return 2

    results = {
        "single_turn": run_test("single-turn blocking", test_single_turn),
        "streaming": run_test("streaming turn", test_streaming_turn),
        "memory_recall": run_test("memory recall (4 turns)", test_memory_recall),
        "multi_turn_15": run_test("multi-turn (15 turns, growing transcript)", lambda: test_multi_turn(15)),
        "json_mode": run_test("json mode", test_json_mode),
        "concurrent_429": run_test("concurrent -> 429", test_concurrent_429),
        "recovery": run_test("recovery after pathological input", test_recovery_after_failure),
    }

    print(f"\n==  summary  ==")
    passed = sum(1 for v in results.values() if v)
    failed = len(results) - passed
    for name, ok in results.items():
        marker = "PASS" if ok else "FAIL"
        print(f"  [{marker}] {name}")
    print(f"\n{passed}/{len(results)} tests passed")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
