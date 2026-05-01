# Python (`openai` SDK)

```bash
pip install openai
```

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://127.0.0.1:11435/v1",
    api_key="dummy",  # any non-empty string; npurun ignores it
                      # unless --auth-token is set
)

# Blocking
resp = client.chat.completions.create(
    model="phi-3.5-mini",
    messages=[{"role": "user", "content": "Three reasons to learn Rust:"}],
)
print(resp.choices[0].message.content)

# Streaming
stream = client.chat.completions.create(
    model="phi-3.5-mini",
    messages=[{"role": "user", "content": "Count to ten."}],
    stream=True,
)
for chunk in stream:
    delta = chunk.choices[0].delta.content
    if delta:
        print(delta, end="", flush=True)
print()
```

## JSON mode

```python
import json

resp = client.chat.completions.create(
    model="phi-3.5-mini",
    messages=[{
        "role": "user",
        "content": "Return a JSON object with name='npurun' and version='0.1.0'.",
    }],
    response_format={"type": "json_object"},
)
data = json.loads(resp.choices[0].message.content)  # may need retry on bad JSON
print(data)
```

This is a prompt hint, not constrained sampling. Wrap the
`json.loads` call in a retry on invalid JSON, same as you would
against OpenAI's own JSON mode.

## Auth

If `npurun serve --auth-token <TOKEN>` is set, pass the token as
`api_key`:

```python
client = OpenAI(base_url=..., api_key=TOKEN)
```

## LangChain / LlamaIndex

Both use the `openai` package under the hood. Set
`OPENAI_API_BASE=http://127.0.0.1:11435/v1` in the environment and
they'll route through npurun automatically. Embeddings are not yet
served — point those at a separate provider.
