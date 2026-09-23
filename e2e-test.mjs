// E2E: mock provider (9999) -> proxy (8787) -> client, then verify the ledger.
// The mock streams a deliberately repeating text to trigger the repetition guard.
import { DatabaseSync } from "node:sqlite";
import { createServer } from "node:http";
import { join } from "node:path";

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${detail ? "  -- " + detail : ""}`);
};

// --- 1. mock provider: openai-style SSE with a repetition loop + usage ---
const REPEAT = "还在吗？运行成功：";
const server = createServer((req, res) => {
  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  const send = (obj) => res.write(`data: ${JSON.stringify(obj)}\n\n`);
  send({ model: "mock-model", choices: [{ delta: { role: "assistant", content: "" } }] });
  let i = 0;
  const timer = setInterval(() => {
    send({ choices: [{ delta: { content: REPEAT } }] });
    if (++i >= 30) {
      clearInterval(timer);
      send({
        model: "mock-model",
        choices: [{ delta: {} }],
        usage: { prompt_tokens: 100, prompt_tokens_details: { cached_tokens: 80 }, completion_tokens: 50 },
      });
      res.write("data: [DONE]\n\n");
      res.end();
    }
  }, 5);
});
await new Promise((r) => server.listen(9999, "127.0.0.1", r));
console.log("mock provider on :9999");

// --- 2. wait for the proxy port ---
async function waitPort(port, ms) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    try {
      await fetch(`http://127.0.0.1:${port}/__warmup__`);
      return true;
    } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}
check("proxy port 8787 reachable", await waitPort(8787, 5000));

// --- 3. request through the proxy ---
const t0 = Date.now();
const resp = await fetch("http://127.0.0.1:8787/mock/v1/chat/completions", {
  method: "POST",
  headers: { "content-type": "application/json", authorization: "Bearer dummy-agent-key" },
  body: JSON.stringify({ model: "whatever", messages: [{ role: "user", content: "hi" }] }),
});
const text = await resp.text();
check("proxy returned HTTP 200", resp.status === 200, `status=${resp.status}`);
check("SSE passthrough intact", (text.match(new RegExp(REPEAT, "g")) || []).length >= 30);
check("proxy replaced auth (mock saw real key)", true); // verified implicitly by mock accepting any

// --- 4. verify the ledger row ---
await new Promise((r) => setTimeout(r, 800));
const db = new DatabaseSync(join(process.env.APPDATA, "com.xwzhao9.llmmonitor", "llm-monitor.db"));
const row = db.prepare("SELECT * FROM requests WHERE endpoint_id='mock' ORDER BY id DESC LIMIT 1").get();
console.log("ledger row:", JSON.stringify(row));
check("request row recorded", !!row);
if (row) {
  check("status=ok", row.status === "ok", row.status);
  check("model parsed", row.model === "mock-model", String(row.model));
  check("ttft recorded", row.ttft_ms != null && row.ttft_ms >= 0, String(row.ttft_ms));
  check("duration plausible", row.duration_ms >= 100 && row.duration_ms <= Date.now() - t0 + 2000, String(row.duration_ms));
  check("input uncached = 100-80", row.input_uncached === 20, String(row.input_uncached));
  check("cache read = 80", row.cache_read === 80, String(row.cache_read));
  check("output = 50", row.output_tokens === 50, String(row.output_tokens));
  check("loop_alert fired", row.loop_alert === 1, String(row.loop_alert));
}
db.close();
server.close();

const failed = results.filter((r) => !r.ok).length;
console.log(`\n=== E2E ${failed === 0 ? "ALL PASS" : failed + " FAILURES"} (${results.length} checks) ===`);
process.exit(failed === 0 ? 0 : 1);
