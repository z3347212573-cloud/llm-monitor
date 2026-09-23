//! SQLite ledger: endpoint config + per-request usage records + grouped stats.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

pub struct Db(pub Mutex<Connection>);

#[derive(Debug, Clone, Serialize)]
pub struct EndpointRow {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub key_masked: String,
    pub protocol: String,
    pub created_at: i64,
}

/// Internal variant carrying the raw API key (never sent to the frontend).
#[derive(Debug, Clone)]
pub struct EndpointFull {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RequestRow {
    pub id: i64,
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub base_url: String,
    pub model: Option<String>,
    pub protocol: String,
    pub path: Option<String>,
    pub started_at: i64,
    pub ttft_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub http_status: Option<i64>,
    pub status: String,
    pub input_uncached: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
    pub loop_alert: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupRow {
    pub key: String,
    pub requests: i64,
    pub input_uncached: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
    pub avg_ttft_ms: Option<f64>,
    pub tok_per_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Totals {
    pub requests: i64,
    pub input_uncached: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
    pub avg_ttft_ms: Option<f64>,
    pub tok_per_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatsSummary {
    pub totals: Totals,
    pub by_provider: Vec<GroupRow>,
    pub by_model: Vec<GroupRow>,
    pub by_agent: Vec<GroupRow>,
}

pub struct InsertRequest {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub base_url: String,
    pub protocol: String,
    pub path: Option<String>,
    pub started_at: i64,
}

pub struct FinishRequest {
    pub id: i64,
    pub status: String,
    pub http_status: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub model: Option<String>,
    pub input_uncached: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub output_tokens: i64,
    pub loop_alert: bool,
    pub error: Option<String>,
}

fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "***".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}***{tail}")
}

fn row_to_endpoint(r: &rusqlite::Row) -> rusqlite::Result<EndpointRow> {
    let api_key: String = r.get(3)?;
    Ok(EndpointRow {
        id: r.get(0)?,
        name: r.get(1)?,
        base_url: r.get(2)?,
        key_masked: mask_key(&api_key),
        protocol: r.get(4)?,
        created_at: r.get(5)?,
    })
}

fn row_to_request(r: &rusqlite::Row) -> rusqlite::Result<RequestRow> {
    Ok(RequestRow {
        id: r.get(0)?,
        endpoint_id: r.get(1)?,
        endpoint_name: r.get(2)?,
        base_url: r.get(3)?,
        model: r.get(4)?,
        protocol: r.get(5)?,
        path: r.get(6)?,
        started_at: r.get(7)?,
        ttft_ms: r.get(8)?,
        duration_ms: r.get(9)?,
        http_status: r.get(10)?,
        status: r.get(11)?,
        input_uncached: r.get(12)?,
        cache_read: r.get(13)?,
        cache_write: r.get(14)?,
        output_tokens: r.get(15)?,
        loop_alert: r.get::<_, i64>(16)? != 0,
        error: r.get(17)?,
    })
}

pub fn init(path: &Path) -> rusqlite::Result<Db> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode=WAL;
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
        CREATE INDEX IF NOT EXISTS idx_requests_started ON requests(started_at);
        "#,
    )?;
    // Rows left 'running' by a crash/restart can never finish: close them out.
    conn.execute(
        "UPDATE requests SET status='aborted', error='app restarted mid-stream' WHERE status='running'",
        [],
    )?;
    Ok(Db(Mutex::new(conn)))
}

impl Db {
    pub fn list_endpoints(&self) -> rusqlite::Result<Vec<EndpointRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, base_url, api_key, protocol, created_at FROM endpoints ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], row_to_endpoint)?;
        rows.collect()
    }

    pub fn list_endpoints_raw(&self) -> rusqlite::Result<Vec<EndpointFull>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, base_url, api_key, protocol FROM endpoints ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(EndpointFull {
                id: r.get(0)?,
                name: r.get(1)?,
                base_url: r.get(2)?,
                api_key: r.get(3)?,
                protocol: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn upsert_endpoint(
        &self,
        id: &str,
        name: &str,
        base_url: &str,
        api_key: &str,
        protocol: &str,
    ) -> rusqlite::Result<EndpointRow> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO endpoints(id, name, base_url, api_key, protocol, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, base_url=excluded.base_url,
                                           api_key=excluded.api_key, protocol=excluded.protocol",
            params![id, name, base_url, api_key, protocol, now_ms()],
        )?;
        let mut stmt = conn.prepare(
            "SELECT id, name, base_url, api_key, protocol, created_at FROM endpoints WHERE id = ?1",
        )?;
        stmt.query_row(params![id], row_to_endpoint)
    }

    pub fn delete_endpoint(&self, id: &str) -> rusqlite::Result<usize> {
        let conn = self.0.lock().unwrap();
        conn.execute("DELETE FROM endpoints WHERE id = ?1", params![id])
    }

    pub fn insert_request(&self, r: &InsertRequest) -> rusqlite::Result<i64> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO requests(endpoint_id, endpoint_name, base_url, protocol, path, started_at, status)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'running')",
            params![r.endpoint_id, r.endpoint_name, r.base_url, r.protocol, r.path, r.started_at],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn finish_request(&self, f: &FinishRequest) -> rusqlite::Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE requests SET status=?2, http_status=?3, ttft_ms=?4, duration_ms=?5, model=?6,
                    input_uncached=?7, cache_read=?8, cache_write=?9, output_tokens=?10,
                    loop_alert=?11, error=?12
             WHERE id=?1",
            params![
                f.id,
                f.status,
                f.http_status,
                f.ttft_ms,
                f.duration_ms,
                f.model,
                f.input_uncached,
                f.cache_read,
                f.cache_write,
                f.output_tokens,
                f.loop_alert as i64,
                f.error
            ],
        )?;
        Ok(())
    }

    pub fn mark_loop_alert(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.0.lock().unwrap();
        conn.execute("UPDATE requests SET loop_alert=1 WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn recent_requests(&self, limit: i64) -> rusqlite::Result<Vec<RequestRow>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, endpoint_id, endpoint_name, base_url, model, protocol, path, started_at,
                    ttft_ms, duration_ms, http_status, status, input_uncached, cache_read,
                    cache_write, output_tokens, loop_alert, error
             FROM requests ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], row_to_request)?;
        rows.collect()
    }

    pub fn stats_since(&self, since_ms: i64) -> rusqlite::Result<StatsSummary> {
        let conn = self.0.lock().unwrap();
        let totals = query_totals(&conn, since_ms)?;
        let by_provider = query_group(&conn, since_ms, "base_url")?;
        let by_model = query_group(&conn, since_ms, "COALESCE(model, '(unknown)')")?;
        let by_agent = query_group(&conn, since_ms, "endpoint_name")?;
        Ok(StatsSummary { totals, by_provider, by_model, by_agent })
    }
}

fn query_totals(conn: &Connection, since: i64) -> rusqlite::Result<Totals> {
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(input_uncached),0), COALESCE(SUM(cache_read),0),
                COALESCE(SUM(cache_write),0),   COALESCE(SUM(output_tokens),0),
                AVG(ttft_ms),
                CAST(SUM(output_tokens) AS REAL) * 1000.0
                    / NULLIF(SUM(MAX(COALESCE(duration_ms,0) - COALESCE(ttft_ms,0), 0)), 0)
         FROM requests
         WHERE started_at >= ?1 AND status != 'running'",
        params![since],
        |r| {
            Ok(Totals {
                requests: r.get(0)?,
                input_uncached: r.get(1)?,
                cache_read: r.get(2)?,
                cache_write: r.get(3)?,
                output_tokens: r.get(4)?,
                avg_ttft_ms: r.get(5)?,
                tok_per_s: r.get(6)?,
            })
        },
    )
}

fn query_group(conn: &Connection, since: i64, key_expr: &str) -> rusqlite::Result<Vec<GroupRow>> {
    let sql = format!(
        "SELECT {key_expr} AS k, COUNT(*),
                COALESCE(SUM(input_uncached),0), COALESCE(SUM(cache_read),0),
                COALESCE(SUM(cache_write),0),   COALESCE(SUM(output_tokens),0),
                AVG(ttft_ms),
                CAST(SUM(output_tokens) AS REAL) * 1000.0
                    / NULLIF(SUM(MAX(COALESCE(duration_ms,0) - COALESCE(ttft_ms,0), 0)), 0)
         FROM requests
         WHERE started_at >= ?1 AND status != 'running'
         GROUP BY k ORDER BY output_tokens DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![since], |r| {
        Ok(GroupRow {
            key: r.get(0)?,
            requests: r.get(1)?,
            input_uncached: r.get(2)?,
            cache_read: r.get(3)?,
            cache_write: r.get(4)?,
            output_tokens: r.get(5)?,
            avg_ttft_ms: r.get(6)?,
            tok_per_s: r.get(7)?,
        })
    })?;
    rows.collect()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
