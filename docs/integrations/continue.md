# Continue.dev / VS Code

[Continue.dev](https://www.continue.dev/) is a VS Code / JetBrains
extension for in-editor code chat and inline completion. It speaks
the OpenAI protocol when configured against a custom provider, so
`npurun serve` works as the backend.

## Setup

1. Install the [Continue extension](https://marketplace.visualstudio.com/items?itemName=Continue.continue).
2. Open the Continue config (Cmd/Ctrl-Shift-P → "Continue: Open
   config.json"), and add a model entry pointing at npurun:

```jsonc
{
  "models": [
    {
      "title": "npurun (Phi 3.5 Mini, NPU)",
      "provider": "openai",
      "model": "phi-3.5-mini",
      "apiBase": "http://127.0.0.1:11435/v1",
      "apiKey": "dummy",
      "completionOptions": {
        "temperature": 0.2,
        "maxTokens": 1024
      }
    }
  ]
}
```

3. Reload Continue. Pick "npurun (Phi 3.5 Mini, NPU)" in the model
   picker.

## Use cases that work well

- Inline chat about selected code ("explain this function", "rewrite
  with better names").
- Quick refactor proposals on small selections.
- Conversational debugging — paste an error, ask for the likely cause.

## Use cases to skip on a 3.8B model

- Whole-file generation — quality drops fast past ~500 tokens.
- Reasoning-heavy code review — Phi 3.5 will guess where a 70B
  model would think.
- Codebase-wide search/refactor with autonomous edits — needs a
  much larger model than fits on this generation of NPU silicon.

## Inline completion

Continue's tab-autocomplete needs a fast model with very low TTFT.
Phi 3.5 Mini's ~200 ms TTFT on the NPU is borderline — usable, but
you'll feel it. A 1B-class model would be a better fit; see
[`roadmap.md`](../roadmap.md) for tracking smaller models.
