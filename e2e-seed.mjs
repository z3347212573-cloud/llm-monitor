// Seed a mock endpoint into the monitor DB before the app launches.
// Schema mirrors src-tauri/src/db.rs (all CREATEs are IF NOT EXISTS).
import { DatabaseSync } from "node:sqlite";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const dir = join(process.env.APPDATA, "com.xwzhao9.llmmonitor");
mkdirSync(dir, { recursive: true });
const db = new DatabaseSync(join(dir, "llm-monitor.db"));

db.exec(`
CREATE TABLE IF NOT EXISTS endpoints(
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    base_url   TEXT NOT NULL,
    api_key    TEXT NOT NULL,
    protocol   TEXT NOT NULL CHECK(protocol IN ('anthropic','openai')),
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS requests(
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    endpoint_id    TEXT NOT NULL,
    endpoint_name  TEXT NOT NULL,
    base_url       TEXT NOT NULL,
    model          TEXT,
    protocol       TEXT NOT NULL,
    path           TEXT,
    started_at     INTEGER NOT NULL,
    ttft_ms        INTEGER,
    duration_ms    INTEGER,
    http_status    INTEGER,
    status         TEXT NOT NULL DEFAULT 'running',
    input_uncached INTEGER NOT NULL DEFAULT 0,
    cache_read     INTEGER NOT NULL DEFAULT 0,
    cache_write    INTEGER NOT NULL DEFAULT 0,
    output_tokens  INTEGER NOT NULL DEFAULT 0,
    loop_alert     INTEGER NOT NULL DEFAULT 0,
    error          TEXT
);
`);
db.prepare(
  `INSERT INTO endpoints(id, name, base_url, api_key, protocol, created_at)
   VALUES('mock', 'Mock Provider', 'http://127.0.0.1:9999', 'test-key-12345678', 'openai', ?)
   ON CONFLICT(id) DO UPDATE SET name=excluded.name, base_url=excluded.base_url,
                                 api_key=excluded.api_key, protocol=excluded.protocol`
).run(Date.now());
const row = db.prepare("SELECT id, base_url FROM endpoints WHERE id='mock'").get();
console.log("seeded endpoint:", JSON.stringify(row));
db.close();
