mod db;
mod guard;
mod proxy;

use proxy::{AppState, EndpointCfg, EndpointMap};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, State, WindowEvent,
};

#[derive(serde::Deserialize)]
struct EndpointInput {
    id: String,
    name: String,
    base_url: String,
    api_key: String,
    protocol: String,
}

fn now_ms() -> i64 {
    db::now_ms()
}

#[tauri::command]
fn list_endpoints(st: State<AppState>) -> Result<Vec<db::EndpointRow>, String> {
    st.db.list_endpoints().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_endpoint(st: State<AppState>, input: EndpointInput) -> Result<db::EndpointRow, String> {
    let id = input.id.trim().to_string();
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("endpoint-id 只能包含字母、数字、- 和 _（它会被用作 URL 路径）".into());
    }
    if !matches!(input.protocol.as_str(), "anthropic" | "openai") {
        return Err("protocol 必须是 anthropic 或 openai".into());
    }
    if !input.base_url.starts_with("http://") && !input.base_url.starts_with("https://") {
        return Err("base_url 必须以 http:// 或 https:// 开头".into());
    }
    if input.api_key.trim().is_empty() {
        return Err("api_key 不能为空".into());
    }
    let row = st
        .db
        .upsert_endpoint(&id, input.name.trim(), input.base_url.trim(), input.api_key.trim(), &input.protocol)
        .map_err(|e| e.to_string())?;
    st.endpoints.write().unwrap().insert(
        row.id.clone(),
        EndpointCfg {
            id: row.id.clone(),
            name: row.name.clone(),
            base_url: row.base_url.clone(),
            api_key: input.api_key.trim().to_string(),
            protocol: row.protocol.clone(),
        },
    );
    Ok(row)
}

#[tauri::command]
fn delete_endpoint(st: State<AppState>, id: String) -> Result<(), String> {
    st.db.delete_endpoint(&id).map_err(|e| e.to_string())?;
    st.endpoints.write().unwrap().remove(&id);
    Ok(())
}

#[tauri::command]
fn recent_requests(st: State<AppState>, limit: Option<i64>) -> Result<Vec<db::RequestRow>, String> {
    st.db.recent_requests(limit.unwrap_or(200)).map_err(|e| e.to_string())
}

#[tauri::command]
fn stats_summary(st: State<AppState>, hours: Option<i64>) -> Result<db::StatsSummary, String> {
    let since = match hours {
        Some(h) if h > 0 => now_ms() - h * 3_600_000,
        _ => 0,
    };
    st.db.stats_since(since).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // second launch: just raise the existing window
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .on_window_event(|window, event| {
            // closing the window hides to tray — the proxy must stay alive
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = handle.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = Arc::new(
                db::init(&data_dir.join("llm-monitor.db")).map_err(|e| e.to_string())?,
            );

            let mut map = HashMap::new();
            for e in db.list_endpoints_raw().map_err(|e| e.to_string())? {
                map.insert(
                    e.id.clone(),
                    EndpointCfg {
                        id: e.id,
                        name: e.name,
                        base_url: e.base_url,
                        api_key: e.api_key,
                        protocol: e.protocol,
                    },
                );
            }
            let endpoints: EndpointMap = Arc::new(RwLock::new(map));

            let http = reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())?;

            let state = AppState { db, endpoints, app: handle.clone(), http };
            app.manage(state.clone());

            tauri::async_runtime::spawn(async move {
                if let Err(e) = proxy::serve(state).await {
                    eprintln!("[llm-monitor] proxy server error: {e}");
                }
            });

            // Register launch-at-login for packaged builds only — dev runs must
            // not clobber the registry entry with the debug exe path.
            if !cfg!(debug_assertions) {
                use tauri_plugin_autostart::ManagerExt;
                match handle.autolaunch().enable() {
                    Ok(_) => eprintln!("[llm-monitor] autostart enabled (launch at login)"),
                    Err(e) => eprintln!("[llm-monitor] autostart enable failed: {e}"),
                }
            }

            // tray icon: left click shows the panel, right click opens the menu
            let open_item = MenuItem::with_id(app, "open", "打开面板", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&open_item, &quit_item])?;
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("LLM Monitor — 代理运行中")
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_endpoints,
            save_endpoint,
            delete_endpoint,
            recent_requests,
            stats_summary
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
