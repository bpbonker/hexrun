# JavaScript / TypeScript (`openai` package)

```bash
npm install openai
```

```ts
import OpenAI from "openai";

const client = new OpenAI({
  baseURL: "http://127.0.0.1:11435/v1",
  apiKey: "dummy", // any non-empty string; npurun ignores it
                   // unless --auth-token is set
});

// Blocking
const resp = await client.chat.completions.create({
  model: "phi-3.5-mini",
  messages: [{ role: "user", content: "Three reasons to learn Rust:" }],
});
console.log(resp.choices[0].message.content);

// Streaming
const stream = await client.chat.completions.create({
  model: "phi-3.5-mini",
  messages: [{ role: "user", content: "Count to ten." }],
  stream: true,
});
for await (const chunk of stream) {
  const delta = chunk.choices[0]?.delta?.content;
  if (delta) process.stdout.write(delta);
}
process.stdout.write("\n");
```

## Browser usage

The `openai` package supports browser builds. CORS is permissive on
`npurun serve`, so you can hit it directly from a `localhost`-served
page. For auth in the browser, prefer a proxy — embedding the bearer
token in JS shipped to a user is rarely what you want.

## JSON mode

```ts
const resp = await client.chat.completions.create({
  model: "phi-3.5-mini",
  messages: [
    { role: "user", content: "Return a JSON object with name='npurun'." },
  ],
  response_format: { type: "json_object" },
});
let data;
try {
  data = JSON.parse(resp.choices[0].message.content!);
} catch {
  // retry on parse failure — JSON mode is a prompt hint, not
  // constrained sampling
}
```
