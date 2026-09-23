# LLM Monitor

**English** • [简体中文](README.zh-CN.md)

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6)](#)

> A local LLM usage dashboard: one reverse proxy that turns "how many tokens did I burn today?" into a number you can watch in real time.

**LLM Monitor** is a Tauri desktop app. It runs a reverse proxy on your machine; you point your agent's baseURL at it, and while it **forwards the SSE stream untouched** it parses the `usage` the provider returns and tracks the four token buckets — **uncached input / cache read / cache write / output** — plus TTFT, total duration and tok/s. Every request lands in a local SQLite ledger. It also ships a **repetition watchdog** that fires a system notification when a streaming response gets stuck in a loop.

**Agent-agnostic** — anything that lets you set a baseURL works (DSH, Claude Code, Codex, Cursor, your own scripts…).

```
agent ──▶ http://127.0.0.1:8787/<endpoint-id> ──▶ LLM Monitor ──▶ provider API
                 (your key is swapped in here, usage is accounted here)
```

## Features

- **Zero-touch integration** — change one baseURL. No agent code changes, no plugin. If the proxy misbehaves, point the baseURL back and you are done.
- **Four token buckets** — uncached input, cache read, cache write and output are tracked separately, so you can see exactly how much caching is saving you.
- **Streaming stays streaming** — SSE chunks are forwarded as they arrive and statistics are parsed on the side; first-token latency takes no detour.
- **Grouped by provider / model / agent** — provider = baseURL, model = the `model` in `usage`, agent = endpoint name.
- **Your key never leaves the machine** — the real API key is stored only in local SQLite and injected on the outbound request.
- **Repetition watchdog** — sentence-level detection of period 1–4 repetition (≥12 repeats); on a hit you get a Windows notification and a red banner. Alert-only: the data path is never modified.
- **Two protocols** — `openai-completions` and `anthropic-messages`.

## UI

- **Summary cards** — requests (24h), output tokens, uncached input, cache read, avg TTFT, avg tok/s
- **Live request feed** — time / agent / model / status / TTFT / duration / tok/s / input / cache read / output
- **Per-provider breakdown table** + **endpoint management**

## Install

Download `llm-monitor_x.y.z_x64-setup.exe` from [Releases](https://github.com/z3347212573-cloud/llm-monitor/releases) (NSIS installer, Windows x64) and run it. On first launch the app creates its database under `%APPDATA%\com.xwzhao9.llmmonitor\`.

> The installer is unsigned, so SmartScreen may warn about an unknown publisher — choose "More info → Run anyway".
> If that bothers you, build it yourself (see [Build from source](#build-from-source)).

## Usage

1. Start the app and, under **Endpoints**, create one endpoint **per agent**:
   - `endpoint-id` — the URL path segment, e.g. `dsh`, `claude-code` (letters, digits, `-`, `_`)
   - `baseURL` — the provider's real address (e.g. `https://open.bigmodel.cn/api/coding/paas/v4`)
   - `API Key` — the real key (stored only in local SQLite, never leaves the proxy)
   - `protocol` — `openai-completions` or `anthropic-messages`
2. In your agent, set the baseURL to `http://127.0.0.1:8787/<endpoint-id>` and put anything in the API key field — the proxy swaps in the real key.
3. Use your agent as usual; the request feed and statistics show up live.

## Architecture

```
┌──────────────── Tauri desktop app (single process) ─────────────────┐
│  Rust backend                                                        │
│   ├─ Reverse proxy (axum): http://127.0.0.1:8787/<endpoint-id>/      │
│   │    ├─ SSE streaming passthrough (anthropic-messages + openai-completions)
│   │    ├─ usage parsing: uncached input / cache read / cache write / output
│   │    ├─ timing: TTFT, total duration, tok/s
│   │    └─ repetition guard: loop detected → notification + banner (alert only)
│   └─ SQLite ledger (%APPDATA%/com.xwzhao9.llmmonitor/llm-monitor.db)
│  WebView2 frontend
│   └─ summary cards + provider/model/agent breakdown + live feed + endpoints
└──────────────────────────────────────────────────────────────────────┘
```

Source layout:

| Path | What it does |
|---|---|
| `src-tauri/src/proxy.rs` | axum reverse proxy, SSE passthrough, usage parsing, timing |
| `src-tauri/src/guard.rs` | repetition detection |
| `src-tauri/src/db.rs` | SQLite ledger and aggregate queries |
| `src-tauri/src/lib.rs` | Tauri commands, tray, notifications, single instance |
| `src/main.ts` | frontend dashboard logic |
| `e2e-seed.mjs` / `e2e-test.mjs` | seed data + end-to-end smoke scripts |

## How the numbers are counted

| Metric | Source |
|---|---|
| Uncached input / cache read / cache write / output | `usage` in the provider response. anthropic: `input_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens`; openai: `prompt_tokens` / `prompt_tokens_details.cached_tokens`; deepseek-style: `prompt_cache_hit_tokens` |
| TTFT | first byte of the upstream response − request received |
| tok/s | `output_tokens / (duration − ttft)` |
| Grouping | provider = base_url; model = `model` from `usage`; agent = endpoint name |

> Figures come **straight from the provider's own `usage`** — there is no local tokenizer estimate, so they match what you are billed for.

## Repetition watchdog

While a stream passes through the proxy, output is split into sentences and checked for period 1–4 repetition (≥12 repeats; pure-punctuation ruler lines are whitelisted). On a hit:

- a red banner at the top of the window plus a Windows notification, telling you to press Esc in the offending agent;
- **the data path is untouched** (alert-only mode); the request row is flagged ⚠.

## Privacy

- The proxy binds to `127.0.0.1` only and is never exposed to the network.
- API keys live in a SQLite file under `%APPDATA%` and go away with the app's data.
- Request and response bodies are **parsed on the side and never persisted** — only usage counts and timing metadata are stored.
- The app contains no telemetry of any kind.

## Build from source

Requires Node.js 18+ and a Rust toolchain ([rustup](https://rustup.rs)):

```powershell
npm install
npm run tauri dev            # dev
npm run tauri build          # bundle (output under src-tauri/target/release/bundle/)
```

Frontend only: `npm run build`. Rust type-check only: `cargo check --manifest-path src-tauri/Cargo.toml`.

The `.npmrc` / Cargo mirror settings in this repo exist for faster downloads in mainland China. They are **optional** — delete them or switch to the official registries and the build still works.

## Known limitations

- Only **Windows x64** is built and tested (it depends on WebView2). Tauri is cross-platform in principle, but that is unverified here.
- Repetition detection only covers streaming text — non-streaming responses and multimodal content are not checked.
- No cost estimation yet — see the roadmap.

## Roadmap

- [ ] Cost estimation from per-model pricing
- [ ] Export to CSV / JSON
- [ ] Time-range filtering and trend charts
- [ ] Verify macOS / Linux builds
- [ ] i18n — English UI (the UI is currently Chinese)

## Contributing

Issues and PRs are welcome. When you change the proxy or the usage parsing, please say which provider and which protocol you tested against, so it can be regression-checked.

## License

[MIT](LICENSE) © 2026 xwzhao9
