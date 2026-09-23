# LLM Monitor

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Platform](https://img.shields.io/badge/platform-Windows-0078D6)](#)

> 本地 LLM 用量看板：一个反向代理，把「今天到底烧了多少 token」变成实时可见的数字。

**LLM Monitor** 是一个 Tauri 桌面应用。它在本机起一个反向代理，你把 agent 的 baseURL 指过来，
它在**原样转发 SSE 流**的同时解析 provider 返回的 usage，实时统计
**输入（未命中）/ 缓存命中 / 缓存写入 / 输出** 四桶 token，以及 TTFT、总耗时、tok/s，
并把每一次请求落进本地 SQLite 台账。附带一个**复读看门狗**：流式输出陷入死循环时弹系统通知报警。

**与 agent 无关** —— 任何能配 baseURL 的 agent（DSH、Claude Code、Codex、Cursor、自研脚本……）都能接入。

```
agent ──▶ http://127.0.0.1:8787/<endpoint-id> ──▶ LLM Monitor ──▶ 厂商 API
                    （key 在这里被替换成真 key，usage 在这里被记账）
```

## 特性

- **零侵入接入**：只改 agent 的 baseURL，不动 agent 代码、不装插件，代理挂了换回原地址即可。
- **四桶 token 口径**：区分「输入未命中 / 缓存命中 / 缓存写入 / 输出」，缓存省了多少一眼看清。
- **流式不打断**：SSE 逐 chunk 转发，统计在旁路解析，不给首字延迟加负担。
- **按厂商 / 模型 / Agent 分组**：厂商 = baseURL，模型 = usage 里的 model，Agent = endpoint 名。
- **Key 不出本机**：真实 API Key 只写本地 SQLite，代理出站时才注入。
- **复读看门狗**：句级 period 1–4 连续重复检测，命中就发 Windows 通知 + 界面红条（仅报警，不干预数据流）。
- **协议双支持**：`openai-completions` 与 `anthropic-messages`。

## 界面

- **总量卡片**：24h 请求数、输出 token、输入（未命中）、缓存命中、平均 TTFT、平均 tok/s
- **实时请求流水**：时间 / Agent / 模型 / 状态 / TTFT / 耗时 / tok/s / 输入 / 缓存命中 / 输出
- **按厂商分组表** + **端点管理**

## 安装

从 [Releases](https://github.com/z3347212573-cloud/llm-monitor/releases) 下载 `llm-monitor_x.y.z_x64-setup.exe`（NSIS 安装包，Windows x64），
装完直接运行。首次启动会在 `%APPDATA%\com.xwzhao9.llmmonitor\` 建库。

> 未签名安装包，SmartScreen 可能提示「未知发布者」，选「更多信息 → 仍要运行」。
> 介意的话请按下面的「从源码构建」自行编译。

## 使用

1. 启动应用，在「端点设置」里为**每个 agent** 建一个端点：
   - `endpoint-id`：URL 路径标识，如 `dsh`、`claude-code`（字母/数字/`-`/`_`）
   - `baseURL`：厂商真实地址（如 `https://open.bigmodel.cn/api/coding/paas/v4`）
   - `API Key`：真实 key（只存本地 SQLite，永不出代理）
   - `协议`：`openai-completions` 或 `anthropic-messages`
2. 在 agent 里把 baseURL 改成 `http://127.0.0.1:8787/<endpoint-id>`，API Key 随便填
   （代理会替换成真实 key）。
3. 正常用 agent —— 面板实时出现请求流水与统计。

## 架构

```
┌────────────────── Tauri 桌面应用（单进程）──────────────────┐
│  Rust 后端                                                   │
│   ├─ 反向代理 (axum): http://127.0.0.1:8787/<endpoint-id>/   │
│   │    ├─ SSE 流式转发（anthropic-messages + openai-completions）
│   │    ├─ 解析 usage：输入(未命中) / 缓存命中 / 缓存写入 / 输出
│   │    ├─ 计时：TTFT、总耗时、tok/s
│   │    └─ repetition guard：检测复读 → 通知 + 界面横幅（仅报警）
│   └─ SQLite 台账 (%APPDATA%/com.xwzhao9.llmmonitor/llm-monitor.db)
│  WebView2 前端
│   └─ 总量卡片 + 按厂商/模型/Agent 分组 + 实时流水 + 端点管理
└──────────────────────────────────────────────────────────────┘
```

源码结构：

| 路径 | 内容 |
|---|---|
| `src-tauri/src/proxy.rs` | axum 反向代理、SSE 转发、usage 解析、计时 |
| `src-tauri/src/guard.rs` | 复读（repetition）检测 |
| `src-tauri/src/db.rs` | SQLite 台账与聚合查询 |
| `src-tauri/src/lib.rs` | Tauri 命令、托盘、通知、单实例 |
| `src/main.ts` | 前端面板逻辑 |
| `e2e-seed.mjs` / `e2e-test.mjs` | 造数据 + 端到端冒烟脚本 |

## 统计口径

| 指标 | 来源 |
|---|---|
| 输入（未命中）/ 缓存命中 / 缓存写入 / 输出 | provider 响应中的 usage。anthropic：`input_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens`；openai：`prompt_tokens` / `prompt_tokens_details.cached_tokens`；deepseek 系：`prompt_cache_hit_tokens` |
| TTFT | 代理收到响应首字节 − 收到请求 |
| tok/s | `output_tokens / (duration − ttft)` |
| 分组 | 厂商 = base_url；模型 = usage 中的 model；Agent = endpoint 名 |

> 口径以 **provider 返回的 usage 为准**，本地不做 tokenizer 估算 —— 所以数字和你账单里的对得上。

## 复读看门狗

流式输出经代理时，按句级分段做 period 1–4 的连续重复检测（≥12 次，纯标点分隔线进白名单）。命中后：

- 界面顶部红色横幅 + Windows 系统通知，提示「去对应 agent 按 Esc」；
- **数据流不干预**（仅报警模式）；该请求行标 ⚠。

## 隐私

- 代理只监听 `127.0.0.1`，不对外暴露。
- API Key 只存本机 `%APPDATA%` 下的 SQLite，随应用数据一起删除。
- 请求/响应**只做旁路解析**，不落盘正文，只记 usage 与耗时元数据。
- 应用不含任何统计上报。

## 从源码构建

需要 Node.js 18+ 与 Rust 工具链（[rustup](https://rustup.rs)）：

```powershell
npm install
npm run tauri dev            # 开发
npm run tauri build          # 打包（产物在 src-tauri/target/release/bundle/）
```

只跑前端：`npm run build`；只做 Rust 类型检查：`cargo check --manifest-path src-tauri/Cargo.toml`。

本仓库的 `.npmrc` / Cargo 镜像配置是为了国内网络加速，**非必需**，删掉或改成官方源都能构建。

## 已知限制

- 仅打包/测试了 **Windows x64**（依赖 WebView2）；Tauri 理论上可跨平台，但未验证。
- 复读检测只覆盖流式文本，非流式响应与多模态内容不检测。
- 尚无成本（价格）估算功能 —— 见 Roadmap。

## Roadmap

- [ ] 按模型单价估算成本（各厂商价格表）
- [ ] 数据导出 CSV / JSON
- [ ] 时间范围筛选与趋势图
- [ ] macOS / Linux 构建验证
- [ ] i18n（English UI）

## 贡献

Issue 和 PR 都欢迎。改动代理/解析逻辑时，请说明用了哪家 provider 的哪种协议，方便回归。

## License

[MIT](LICENSE) © 2026 xwzhao9
