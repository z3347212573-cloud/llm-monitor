//! Local reverse proxy: 127.0.0.1:8787/{endpoint-id}/{*path} -> real provider.
//!
//! Streams bytes through untouched while tapping the SSE flow to extract
//! model/usage (four token buckets), measure TTFT, and run the repetition
//! guard. Request rows are finalized exactly once — normally at stream end,
//! or via Drop when the client disconnects mid-stream.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use futures::{Stream, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::Instant,
};
use tauri::{AppHandle, Emitter};

use crate::{db, guard};

const PORT: u16 = 8787;
const SSE_BUF_CAP: usize = 512 * 1024;
const TAIL_KEEP_BYTES: usize = 4096;

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_uncached: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
}

#[derive(Clone)]
pub struct EndpointCfg {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    /// "anthropic" | "openai"
    pub protocol: String,
}

pub type EndpointMap = Arc<RwLock<HashMap<String, EndpointCfg>>>;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<db::Db>,
    pub endpoints: EndpointMap,
    pub app: AppHandle,
    pub http: reqwest::Client,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/{endpoint_id}/{*path}", any(handler))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                "LLM Monitor proxy: configure your agent's baseURL as http://127.0.0.1:8787/<endpoint-id>",
            )
                .into_response()
        })
        .with_state(state)
}

pub async fn serve(state: AppState) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await?;
    eprintln!("[llm-monitor] proxy listening on http://127.0.0.1:{PORT}");
    axum::serve(listener, router(state)).await
}

async fn handler(
    State(st): State<AppState>,
    Path((endpoint_id, rest)): Path<(String, String)>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
    body: Body,
) -> Response {
    let cfg = st.endpoints.read().unwrap().get(&endpoint_id).cloned();
    let Some(cfg) = cfg else {
        return (
            StatusCode::NOT_FOUND,
            format!("unknown endpoint-id: {endpoint_id}"),
        )
            .into_response();
    };

    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("{}/{rest}{query}", cfg.base_url.trim_end_matches('/'));

    let started_at = db::now_ms();
    let mut tap = TapState::new(st.clone(), cfg.clone());
    match st.db.insert_request(&db::InsertRequest {
        endpoint_id: endpoint_id.clone(),
        endpoint_name: cfg.name.clone(),
        base_url: cfg.base_url.clone(),
        protocol: cfg.protocol.clone(),
        path: Some(rest.clone()),
        started_at,
    }) {
        Ok(id) => tap.request_id = id,
        Err(e) => eprintln!("[llm-monitor] db insert_request: {e}"),
    }
    let _ = st.app.emit(
        "proxy:event",
        json!({
            "type": "start", "id": tap.request_id, "endpoint_id": endpoint_id,
            "endpoint_name": cfg.name, "path": rest, "started_at": started_at,
        }),
    );

    // Build the upstream request: pass through everything except hop-by-hop,
    // auth (we hold the real key) and content negotiation we must control.
    let mut req = st.http.request(method, &url);
    const SKIP: &[&str] = &[
        "host",
        "content-length",
        "connection",
        "transfer-encoding",
        "authorization",
        "x-api-key",
        "anthropic-version",
        "accept-encoding",
        "expect",
    ];
    for (k, v) in headers.iter() {
        let name = k.as_str().to_ascii_lowercase();
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        req = req.header(k, v);
    }
    match cfg.protocol.as_str() {
        "anthropic" => {
            req = req.header("x-api-key", &cfg.api_key);
            let ver = headers
                .get("anthropic-version")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("2023-06-01");
            req = req.header("anthropic-version", ver);
        }
        _ => {
            req = req.header("authorization", format!("Bearer {}", cfg.api_key));
        }
    }
    let body_stream = body
        .into_data_stream()
        .map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)));
    req = req.body(reqwest::Body::wrap_stream(body_stream));

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            tap.finalize("error", Some(format!("upstream: {e}")));
            return (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")).into_response();
        }
    };
    tap.http_status = Some(resp.status().as_u16() as i64);

    let mut builder = Response::builder().status(resp.status());
    for (k, v) in resp.headers().iter() {
        let name = k.as_str().to_ascii_lowercase();
        if matches!(name.as_str(), "content-length" | "transfer-encoding" | "connection") {
            continue;
        }
        builder = builder.header(k, v);
    }

    let tapped = tap_stream(tap, resp.bytes_stream());
    builder.body(Body::from_stream(tapped)).unwrap()
}

/// Forward upstream bytes to the client unchanged, tapping each chunk first.
fn tap_stream<S>(tap: TapState, upstream: S) -> impl Stream<Item = Result<bytes::Bytes, std::io::Error>>
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin,
{
    futures::stream::unfold((tap, upstream), |(mut tap, mut upstream)| async move {
        match upstream.next().await {
            Some(Ok(chunk)) => {
                tap.on_chunk(&chunk);
                Some((Ok(chunk), (tap, upstream)))
            }
            Some(Err(e)) => {
                tap.finalize("error", Some(format!("stream: {e}")));
                Some((
                    Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
                    (tap, upstream),
                ))
            }
            None => {
                tap.finalize("ok", None);
                None
            }
        }
    })
}

struct TapState {
    st: AppState,
    request_id: i64,
    cfg: EndpointCfg,
    started: Instant,
    http_status: Option<i64>,
    ttft_ms: Option<i64>,
    sse_buf: String,
    model: Option<String>,
    usage: Usage,
    got_usage: bool,
    text_tail: String,
    alerted: bool,
    finalized: bool,
}

impl TapState {
    fn new(st: AppState, cfg: EndpointCfg) -> Self {
        Self {
            st,
            request_id: -1,
            cfg,
            started: Instant::now(),
            http_status: None,
            ttft_ms: None,
            sse_buf: String::new(),
            model: None,
            usage: Usage::default(),
            got_usage: false,
            text_tail: String::new(),
            alerted: false,
            finalized: false,
        }
    }

    fn on_chunk(&mut self, chunk: &[u8]) {
        if self.ttft_ms.is_none() {
            self.ttft_ms = Some(self.started.elapsed().as_millis() as i64);
        }
        self.sse_buf.push_str(&String::from_utf8_lossy(chunk));
        while let Some(pos) = self.sse_buf.find("\n\n") {
            let event: String = self.sse_buf.drain(..pos + 2).collect();
            self.process_event(&event);
        }
        if self.sse_buf.len() > SSE_BUF_CAP {
            self.sse_buf.clear();
        }
    }

    fn process_event(&mut self, event: &str) {
        let mut data = String::new();
        for line in event.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data.push_str(rest.trim_start());
            }
        }
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        if self.cfg.protocol == "anthropic" {
            self.process_anthropic(&v);
        } else {
            self.process_openai(&v);
        }
    }

    fn process_openai(&mut self, v: &Value) {
        if self.model.is_none() {
            if let Some(m) = v["model"].as_str() {
                self.model = Some(m.to_string());
            }
        }
        if let Some(t) = v["choices"][0]["delta"]["content"].as_str() {
            self.feed_text(t);
        }
        if let Some(t) = v["choices"][0]["delta"]["reasoning_content"].as_str() {
            self.feed_text(t);
        }
        if v["usage"].is_object() {
            let u = &v["usage"];
            let prompt = u["prompt_tokens"].as_i64().unwrap_or(0);
            let cached = u["prompt_tokens_details"]["cached_tokens"]
                .as_i64()
                .or_else(|| u["prompt_cache_hit_tokens"].as_i64())
                .unwrap_or(0);
            self.usage = Usage {
                input_uncached: (prompt - cached).max(0),
                cache_read: cached,
                cache_write: 0,
                output_tokens: u["completion_tokens"].as_i64().unwrap_or(0),
            };
            self.got_usage = true;
        }
    }

    fn process_anthropic(&mut self, v: &Value) {
        match v["type"].as_str() {
            Some("message_start") => {
                if let Some(m) = v["message"]["model"].as_str() {
                    self.model = Some(m.to_string());
                }
                let u = &v["message"]["usage"];
                self.usage.input_uncached = u["input_tokens"].as_i64().unwrap_or(0);
                self.usage.cache_read = u["cache_read_input_tokens"].as_i64().unwrap_or(0);
                self.usage.cache_write = u["cache_creation_input_tokens"].as_i64().unwrap_or(0);
                self.got_usage = true;
            }
            Some("content_block_delta") => {
                if let Some(t) = v["delta"]["text"].as_str() {
                    self.feed_text(t);
                }
                if let Some(t) = v["delta"]["thinking"].as_str() {
                    self.feed_text(t);
                }
            }
            Some("message_delta") => {
                if let Some(o) = v["usage"]["output_tokens"].as_i64() {
                    self.usage.output_tokens = o;
                    self.got_usage = true;
                }
            }
            _ => {}
        }
    }

    fn feed_text(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.text_tail.push_str(s);
        if self.text_tail.len() > TAIL_KEEP_BYTES {
            let cut = self.text_tail.len() - TAIL_KEEP_BYTES / 2;
            let mut idx = cut;
            while !self.text_tail.is_char_boundary(idx) {
                idx += 1;
            }
            self.text_tail.drain(..idx);
        }
        if self.alerted {
            return;
        }
        if let Some((unit, run)) = guard::detect_loop(&self.text_tail) {
            self.alerted = true;
            let _ = self.st.db.mark_loop_alert(self.request_id);
            let short = guard::shorten_unit(&unit);
            let _ = self.st.app.emit(
                "proxy:event",
                json!({
                    "type": "guard-alert", "id": self.request_id,
                    "endpoint_name": self.cfg.name, "unit": short, "run": run,
                }),
            );
            use tauri_plugin_notification::NotificationExt;
            let _ = self
                .st
                .app
                .notification()
                .builder()
                .title("LLM Monitor: 检测到复读")
                .body(format!(
                    "{} 的请求正在重复「{short}」×{run}，请按 Esc 停止该会话",
                    self.cfg.name
                ))
                .show();
        }
    }

    fn finalize(&mut self, status: &str, error: Option<String>) {
        if self.finalized {
            return;
        }
        self.finalized = true;
        let duration_ms = self.started.elapsed().as_millis() as i64;
        let mut status = status.to_string();
        if status == "ok" && self.http_status.map_or(false, |c| c >= 400) {
            status = "error".into();
        }
        let _ = self.st.db.finish_request(&db::FinishRequest {
            id: self.request_id,
            status: status.clone(),
            http_status: self.http_status,
            ttft_ms: self.ttft_ms,
            duration_ms: Some(duration_ms),
            model: self.model.clone(),
            input_uncached: self.usage.input_uncached,
            cache_read: self.usage.cache_read,
            cache_write: self.usage.cache_write,
            output_tokens: self.usage.output_tokens,
            loop_alert: self.alerted,
            error,
        });
        let _ = self.st.app.emit(
            "proxy:event",
            json!({
                "type": "end", "id": self.request_id, "status": status,
                "http_status": self.http_status, "ttft_ms": self.ttft_ms,
                "duration_ms": duration_ms, "model": self.model,
                "usage": {
                    "input_uncached": self.usage.input_uncached,
                    "cache_read": self.usage.cache_read,
                    "cache_write": self.usage.cache_write,
                    "output_tokens": self.usage.output_tokens,
                },
                "loop_alert": self.alerted,
            }),
        );
    }
}

impl Drop for TapState {
    fn drop(&mut self) {
        if !self.finalized {
            self.finalize("client_abort", None);
        }
    }
}
