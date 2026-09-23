import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type GroupRow = {
  key: string;
  requests: number;
  input_uncached: number;
  cache_read: number;
  cache_write: number;
  output_tokens: number;
  avg_ttft_ms: number | null;
  tok_per_s: number | null;
};

type Totals = {
  requests: number;
  input_uncached: number;
  cache_read: number;
  cache_write: number;
  output_tokens: number;
  avg_ttft_ms: number | null;
  tok_per_s: number | null;
};

type Stats = {
  totals: Totals;
  by_provider: GroupRow[];
  by_model: GroupRow[];
  by_agent: GroupRow[];
};

type RequestRow = {
  id: number;
  endpoint_id: string;
  endpoint_name: string;
  base_url: string;
  model: string | null;
  protocol: string;
  path: string | null;
  started_at: number;
  ttft_ms: number | null;
  duration_ms: number | null;
  http_status: number | null;
  status: string;
  input_uncached: number;
  cache_read: number;
  cache_write: number;
  output_tokens: number;
  loop_alert: boolean;
  error: string | null;
};

type EndpointRow = {
  id: string;
  name: string;
  base_url: string;
  key_masked: string;
  protocol: string;
  created_at: number;
};

const ESC_MAP: Record<string, string> = {
  "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
};

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ESC_MAP[c]);
}

function fmtInt(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(2) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "k";
  return String(n);
}

function fmtMs(ms: number | null | undefined): string {
  if (ms == null) return "–";
  return ms >= 1000 ? (ms / 1000).toFixed(1) + "s" : ms + "ms";
}

function fmtTps(tps: number | null | undefined): string {
  return tps == null ? "–" : tps.toFixed(1);
}

function fmtTime(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const p = (x: number) => String(x).padStart(2, "0");
  const hm = `${p(d.getHours())}:${p(d.getMinutes())}`;
  if (d.toDateString() === now.toDateString()) {
    return `${hm}:${p(d.getSeconds())}`;
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (d.toDateString() === yesterday.toDateString()) {
    return `昨天 ${hm}`;
  }
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${hm}`;
}

function statusBadge(r: RequestRow): string {
  const map: Record<string, string> = {
    running: "st-run", ok: "st-ok", error: "st-err", client_abort: "st-abort", aborted: "st-abort",
  };
  const label: Record<string, string> = {
    running: "流式中", ok: "完成", error: "错误", client_abort: "中止", aborted: "中止",
  };
  return `<span class="badge ${map[r.status] ?? "st-err"}">${label[r.status] ?? esc(r.status)}</span>`;
}

function requestRowHtml(r: RequestRow): string {
  const tps =
    r.output_tokens > 0 && r.duration_ms && r.ttft_ms != null && r.duration_ms - r.ttft_ms > 0
      ? (r.output_tokens * 1000) / (r.duration_ms - r.ttft_ms)
      : null;
  return `<tr class="${r.loop_alert ? "row-alert" : ""}">
    <td>${fmtTime(r.started_at)}</td>
    <td>${esc(r.endpoint_name)}</td>
    <td class="mono">${esc(r.model ?? "…")}</td>
    <td>${statusBadge(r)}</td>
    <td>${fmtMs(r.ttft_ms)}</td>
    <td>${fmtMs(r.duration_ms)}</td>
    <td>${fmtTps(tps)}</td>
    <td>${fmtInt(r.input_uncached)}</td>
    <td>${fmtInt(r.cache_read)}</td>
    <td>${fmtInt(r.output_tokens)}</td>
    <td>${r.loop_alert ? "⚠" : ""}${r.error ? `<span class="err" title="${esc(r.error)}">✕</span>` : ""}</td>
  </tr>`;
}

function renderRequests(rows: RequestRow[]): void {
  const body = document.querySelector<HTMLTableSectionElement>("#req-body")!;
  body.innerHTML = rows.map(requestRowHtml).join("");
}

function groupTableHtml(rows: GroupRow[]): string {
  if (rows.length === 0) return `<tbody><tr><td class="empty">暂无数据</td></tr></tbody>`;
  return `<thead><tr><th>名称</th><th>请求</th><th>入</th><th>命中</th><th>出</th><th>tok/s</th></tr></thead><tbody>` +
    rows.map((g) => `<tr>
      <td class="mono" title="${esc(g.key)}">${esc(g.key.length > 28 ? g.key.slice(0, 28) + "…" : g.key)}</td>
      <td>${g.requests}</td>
      <td>${fmtInt(g.input_uncached)}</td>
      <td>${fmtInt(g.cache_read)}</td>
      <td>${fmtInt(g.output_tokens)}</td>
      <td>${fmtTps(g.tok_per_s)}</td>
    </tr>`).join("") + `</tbody>`;
}

async function refreshStats(): Promise<void> {
  const s = await invoke<Stats>("stats_summary", { hours: 24 });
  (document.querySelector("#c-requests") as HTMLElement).textContent = fmtInt(s.totals.requests);
  (document.querySelector("#c-output") as HTMLElement).textContent = fmtInt(s.totals.output_tokens);
  (document.querySelector("#c-input") as HTMLElement).textContent = fmtInt(s.totals.input_uncached);
  (document.querySelector("#c-cache") as HTMLElement).textContent = fmtInt(s.totals.cache_read);
  (document.querySelector("#c-ttft") as HTMLElement).textContent = fmtMs(s.totals.avg_ttft_ms);
  (document.querySelector("#c-tps") as HTMLElement).textContent = fmtTps(s.totals.tok_per_s);
  document.querySelector("#g-provider")!.innerHTML = groupTableHtml(s.by_provider);
}

async function refreshRequests(): Promise<void> {
  const rows = await invoke<RequestRow[]>("recent_requests", { limit: 100 });
  renderRequests(rows);
}

async function refreshEndpoints(): Promise<void> {
  const rows = await invoke<EndpointRow[]>("list_endpoints");
  const table = document.querySelector<HTMLTableElement>("#ep-table")!;
  if (rows.length === 0) {
    table.innerHTML = `<tbody><tr><td class="empty">暂无端点，先在上方表单里添加一个</td></tr></tbody>`;
    return;
  }
  table.innerHTML = `<thead><tr><th>endpoint-id</th><th>名称</th><th>baseURL</th><th>Key</th><th>协议</th><th></th></tr></thead><tbody>` +
    rows.map((e) => `<tr>
      <td class="mono">/${esc(e.id)}/</td>
      <td>${esc(e.name)}</td>
      <td class="mono" title="${esc(e.base_url)}">${esc(e.base_url.length > 40 ? e.base_url.slice(0, 40) + "…" : e.base_url)}</td>
      <td class="mono">${esc(e.key_masked)}</td>
      <td>${esc(e.protocol)}</td>
      <td><button class="link-danger" data-del="${esc(e.id)}">删除</button></td>
    </tr>`).join("") + `</tbody>`;
  table.querySelectorAll<HTMLButtonElement>("button[data-del]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      await invoke("delete_endpoint", { id: btn.dataset.del });
      await refreshEndpoints();
    });
  });
}

function showAlert(p: { endpoint_name: string; id: number; unit: string; run: number }): void {
  const banner = document.querySelector("#alert-banner")!;
  const text = document.querySelector("#alert-text")!;
  text.textContent =
    `⚠ 复读疑似：${p.endpoint_name} 的请求 #${p.id} 正在重复「${p.unit}」×${p.run} —— 请到对应 agent 按 Esc 停止`;
  banner.classList.remove("hidden");
}

type ProxyEvent =
  | { type: "start"; id: number; endpoint_name: string; path: string; started_at: number }
  | {
      type: "end"; id: number; status: string; http_status: number | null;
      ttft_ms: number | null; duration_ms: number; model: string | null;
      usage: { input_uncached: number; cache_read: number; cache_write: number; output_tokens: number };
      loop_alert: boolean;
    }
  | { type: "guard-alert"; id: number; endpoint_name: string; unit: string; run: number };

// streaming rows currently shown (start events), replaced wholesale on end events
const streamingRows = new Map<number, RequestRow>();

window.addEventListener("DOMContentLoaded", async () => {
  document.querySelector("#alert-close")?.addEventListener("click", () => {
    document.querySelector("#alert-banner")!.classList.add("hidden");
  });

  document.querySelector<HTMLFormElement>("#ep-form")!.addEventListener("submit", async (e) => {
    e.preventDefault();
    const form = e.currentTarget as HTMLFormElement;
    try {
      await invoke("save_endpoint", {
        input: {
          id: (form.querySelector("#ep-id") as HTMLInputElement).value.trim(),
          name: (form.querySelector("#ep-name") as HTMLInputElement).value.trim(),
          base_url: (form.querySelector("#ep-url") as HTMLInputElement).value.trim(),
          api_key: (form.querySelector("#ep-key") as HTMLInputElement).value.trim(),
          protocol: (form.querySelector("#ep-protocol") as HTMLSelectElement).value,
        },
      });
      form.reset();
      await refreshEndpoints();
    } catch (err) {
      alert(String(err));
    }
  });

  await listen<ProxyEvent>("proxy:event", (e) => {
    const p = e.payload;
    if (p.type === "start") {
      streamingRows.set(p.id, {
        id: p.id, endpoint_id: "", endpoint_name: p.endpoint_name, base_url: "",
        model: null, protocol: "", path: p.path, started_at: p.started_at,
        ttft_ms: null, duration_ms: null, http_status: null, status: "running",
        input_uncached: 0, cache_read: 0, cache_write: 0, output_tokens: 0,
        loop_alert: false, error: null,
      });
      const done = Array.from(document.querySelectorAll("#req-body tr")).length;
      if (done < 100) {
        const body = document.querySelector<HTMLTableSectionElement>("#req-body")!;
        body.insertAdjacentHTML("afterbegin", requestRowHtml(streamingRows.get(p.id)!));
      }
    } else if (p.type === "end") {
      streamingRows.delete(p.id);
      void refreshRequests();
      void refreshStats();
    } else if (p.type === "guard-alert") {
      showAlert(p);
    }
  });

  await Promise.all([refreshStats(), refreshRequests(), refreshEndpoints()]);
  // periodic stats refresh keeps "24h" cards honest for aborts made elsewhere
  setInterval(() => void refreshStats(), 30_000);
});
