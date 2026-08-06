#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, WebviewUrl, WebviewBuilder, LogicalPosition, LogicalSize, State};
use url::Url;

#[cfg(target_os = "linux")]
use gtk::prelude::*;

const TOOLBAR_HEIGHT: f64 = 76.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: String,
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub url: String,
    pub title: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub id: String,
    pub url: String,
    pub title: String,
    pub folder: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    pub search_engine: String,
    pub homepage: String,
    pub theme: String,
    pub restore_session: bool,
    pub max_history_items: usize,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            search_engine: "https://www.google.com/search?q=".to_string(),
            homepage: "newtab.html".to_string(),
            theme: "dark".to_string(),
            restore_session: true,
            max_history_items: 10000,
        }
    }
}

#[derive(Debug)]
pub struct AppState {
    pub tabs: Vec<TabInfo>,
    pub active_id: Option<String>,
    pub next_tab_num: usize,
    pub toolbar_visible: bool,
    pub closed_tabs_stack: Vec<String>,
    pub history: Vec<HistoryEntry>,
    pub bookmarks: Vec<BookmarkEntry>,
    pub settings: UserSettings,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_id: None,
            next_tab_num: 0,
            toolbar_visible: true,
            closed_tabs_stack: Vec::new(),
            history: Vec::new(),
            bookmarks: Vec::new(),
            settings: UserSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabStatePayload {
    pub tabs: Vec<TabInfo>,
    pub active_id: Option<String>,
}

fn get_app_dir(app_handle: &tauri::AppHandle) -> std::path::PathBuf {
    if let Ok(dir) = app_handle.path().app_config_dir() {
        let _ = std::fs::create_dir_all(&dir);
        dir
    } else {
        std::path::PathBuf::from(".")
    }
}

fn get_session_file_path(app_handle: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    Some(get_app_dir(app_handle).join("session.json"))
}

fn load_history(app_handle: &tauri::AppHandle) -> Vec<HistoryEntry> {
    let path = get_app_dir(app_handle).join("history.json");
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(entries) = serde_json::from_str::<Vec<HistoryEntry>>(&content) {
                return entries;
            }
        }
    }
    Vec::new()
}

fn save_history(app_handle: &tauri::AppHandle, history: &[HistoryEntry]) {
    let path = get_app_dir(app_handle).join("history.json");
    if let Ok(content) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, content);
    }
}

fn load_bookmarks(app_handle: &tauri::AppHandle) -> Vec<BookmarkEntry> {
    let path = get_app_dir(app_handle).join("bookmarks.json");
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(entries) = serde_json::from_str::<Vec<BookmarkEntry>>(&content) {
                return entries;
            }
        }
    }
    Vec::new()
}

fn save_bookmarks(app_handle: &tauri::AppHandle, bookmarks: &[BookmarkEntry]) {
    let path = get_app_dir(app_handle).join("bookmarks.json");
    if let Ok(content) = serde_json::to_string_pretty(bookmarks) {
        let _ = std::fs::write(path, content);
    }
}

fn load_settings(app_handle: &tauri::AppHandle) -> UserSettings {
    let path = get_app_dir(app_handle).join("settings.json");
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(settings) = serde_json::from_str::<UserSettings>(&content) {
                return settings;
            }
        }
    }
    UserSettings::default()
}

fn save_settings(app_handle: &tauri::AppHandle, settings: &UserSettings) {
    let path = get_app_dir(app_handle).join("settings.json");
    if let Ok(content) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, content);
    }
}

fn record_history_visit(app_handle: &tauri::AppHandle, state: &Arc<Mutex<AppState>>, url: &str, title: &str) {
    if url.is_empty() || is_newtab_url(url) {
        return;
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = HistoryEntry {
        id: format!("{}-{}", timestamp, state.lock().unwrap().history.len() + 1),
        url: url.to_string(),
        title: if title.is_empty() { url.to_string() } else { title.to_string() },
        timestamp,
    };

    let updated_history = {
        let mut sg = state.lock().unwrap();
        if let Some(last) = sg.history.first() {
            if last.url == url && (timestamp - last.timestamp) < 5 {
                return;
            }
        }
        sg.history.insert(0, entry);
        let max_items = sg.settings.max_history_items;
        if sg.history.len() > max_items {
            sg.history.truncate(max_items);
        }
        sg.history.clone()
    };

    save_history(app_handle, &updated_history);
}

fn resolve_input_with_engine(input: &str, search_engine_base: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Empty input".to_string());
    }
    let lower = trimmed.to_lowercase();
    if lower.starts_with("file://") || lower.starts_with("javascript:") || lower.starts_with("data:") {
        return Err(format!("Blocked scheme: {}", trimmed));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed).map(|u| u.to_string()).map_err(|e| format!("Invalid URL: {}", e));
    }
    let with_scheme = format!("https://{}", trimmed);
    if let Ok(parsed) = Url::parse(&with_scheme) {
        if let Some(host) = parsed.host_str() {
            if host.contains('.') || host == "localhost" {
                return Ok(with_scheme);
            }
        }
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("q", trimmed);
    let query_str = serializer.finish();
    let q_val = query_str.replace("q=", "");
    if search_engine_base.contains("%s") {
        Ok(search_engine_base.replace("%s", &q_val))
    } else if search_engine_base.ends_with('=') || search_engine_base.ends_with('?') {
        Ok(format!("{}{}", search_engine_base, q_val))
    } else if search_engine_base.contains('?') {
        Ok(format!("{}&{}", search_engine_base, query_str))
    } else {
        Ok(format!("{}?{}", search_engine_base, query_str))
    }
}

fn resolve_input(input: &str) -> Result<String, String> {
    resolve_input_with_engine(input, "https://www.google.com/search?q=")
}

fn save_session(app_handle: &tauri::AppHandle, state: &AppState) {
    if let Some(path) = get_session_file_path(app_handle) {
        let payload = TabStatePayload {
            tabs: state.tabs.clone(),
            active_id: state.active_id.clone(),
        };
        if let Ok(data) = serde_json::to_string_pretty(&payload) {
            let _ = std::fs::write(path, data);
        }
    }
}

fn emit_tab_state(app_handle: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    save_session(app_handle, state);
    let payload = TabStatePayload { tabs: state.tabs.clone(), active_id: state.active_id.clone() };
    app_handle.emit_to("main", "tabs-changed", payload).map_err(|e| e.to_string())?;
    if let Some(active_id) = &state.active_id {
        if let Some(tab) = state.tabs.iter().find(|t| &t.id == active_id) {
            app_handle.emit_to("main", "url-changed", tab.url.clone()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn is_internal_url(url: &str) -> bool {
    url == "newtab.html" || url.contains("newtab.html")
        || url == "history.html" || url.contains("history.html")
        || url == "bookmarks.html" || url.contains("bookmarks.html")
        || url == "settings.html" || url.contains("settings.html")
}

fn is_newtab_url(url: &str) -> bool {
    is_internal_url(url)
}

fn relayout(app_handle: &tauri::AppHandle, state: &AppState) {
    let Some(main_window) = app_handle.get_window("main") else { return };
    let Ok(size) = main_window.inner_size() else { return };
    let factor = main_window.scale_factor().unwrap_or(1.0);
    let win_w = (size.width as f64 / factor) as i32;
    let win_h = (size.height as f64 / factor) as i32;
    let toolbar_h = if state.toolbar_visible { TOOLBAR_HEIGHT as i32 } else { 0 };

    println!(
        "[GHOST DEBUG] relayout: win={}x{}, toolbar_h={}, active_id={:?}",
        win_w, win_h, toolbar_h, state.active_id
    );

    #[cfg(target_os = "linux")]
    {
        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if !children.is_empty() {
                // Toolbar widget (children[0])
                if let Some(toolbar_widget) = children.get(0) {
                    if state.toolbar_visible {
                        toolbar_widget.show_all();
                        toolbar_widget.set_vexpand(false);
                        toolbar_widget.set_valign(gtk::Align::Start);
                        toolbar_widget.set_size_request(-1, TOOLBAR_HEIGHT as i32);
                        gtk_box.set_child_packing(toolbar_widget, false, false, 0, gtk::PackType::Start);
                    } else {
                        toolbar_widget.hide();
                        toolbar_widget.set_vexpand(false);
                        toolbar_widget.set_size_request(-1, 0);
                        gtk_box.set_child_packing(toolbar_widget, false, false, 0, gtk::PackType::Start);
                    }
                }

                let active_index = state.active_id.as_ref().and_then(|active_id| {
                    state.tabs.iter().position(|t| &t.id == active_id)
                });

                // Tab webview widgets (children[1..N])
                for (idx, widget) in children.iter().skip(1).enumerate() {
                    let is_active = Some(idx) == active_index;
                    if is_active {
                        widget.show_all();
                        widget.set_vexpand(true);
                        widget.set_valign(gtk::Align::Fill);
                        widget.set_size_request(-1, -1);
                        gtk_box.set_child_packing(widget, true, true, 0, gtk::PackType::Start);
                    } else {
                        widget.hide();
                        widget.set_vexpand(false);
                        widget.set_size_request(-1, 0);
                        gtk_box.set_child_packing(widget, false, false, 0, gtk::PackType::Start);
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let toolbar_h_f = if state.toolbar_visible { TOOLBAR_HEIGHT } else { 0.0 };
        let logical_w = win_w as f64;
        let logical_h = win_h as f64;

        if let Some(main_wv) = app_handle.get_webview("main") {
            if state.toolbar_visible {
                let _ = main_wv.show();
                let _ = main_wv.set_position(LogicalPosition::new(0.0, 0.0));
                let _ = main_wv.set_size(LogicalSize::new(logical_w, toolbar_h_f));
            } else {
                let _ = main_wv.hide();
            }
        }

        for tab in &state.tabs {
            if let Some(wv) = app_handle.get_webview(&tab.id) {
                if Some(&tab.id) == state.active_id.as_ref() {
                    let _ = wv.show();
                    let _ = wv.set_position(LogicalPosition::new(0.0, toolbar_h_f));
                    let content_h = (logical_h - toolbar_h_f).max(0.0);
                    let _ = wv.set_size(LogicalSize::new(logical_w, content_h));
                } else {
                    let _ = wv.hide();
                }
            }
        }
    }
}

#[tauri::command]
fn toggle_toolbar(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<bool, String> {
    let mut sg = state.lock().unwrap();
    sg.toolbar_visible = true;
    let _ = app_handle.emit_to("main", "toolbar-shown", ());
    relayout(&app_handle, &sg);
    Ok(true)
}

#[tauri::command]
fn hide_toolbar(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let mut sg = state.lock().unwrap();
    sg.toolbar_visible = true;
    relayout(&app_handle, &sg);
    Ok(())
}

#[tauri::command]
fn focus_urlbar(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let mut sg = state.lock().unwrap();
    sg.toolbar_visible = true;
    let _ = app_handle.emit_to("main", "toolbar-shown", ());
    relayout(&app_handle, &sg);
    Ok(())
}

#[tauri::command]
fn create_tab(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    url: Option<String>,
) -> Result<String, String> {
    let (target_url, webview_url) = match url {
        Some(ref u) if !u.trim().is_empty() => {
            let lower = u.trim().to_lowercase();
            if lower == "history.html" || lower.ends_with("/history.html") {
                ("history.html".to_string(), WebviewUrl::App("history.html".into()))
            } else if lower == "bookmarks.html" || lower.ends_with("/bookmarks.html") {
                ("bookmarks.html".to_string(), WebviewUrl::App("bookmarks.html".into()))
            } else if lower == "settings.html" || lower.ends_with("/settings.html") {
                ("settings.html".to_string(), WebviewUrl::App("settings.html".into()))
            } else if lower == "newtab.html" || lower.ends_with("/newtab.html") {
                ("newtab.html".to_string(), WebviewUrl::App("newtab.html".into()))
            } else {
                let engine = state.lock().unwrap().settings.search_engine.clone();
                let resolved = resolve_input_with_engine(u, &engine)?;
                let parsed: Url = resolved.parse().map_err(|e: url::ParseError| e.to_string())?;
                (resolved, WebviewUrl::External(parsed))
            }
        }
        _ => ("newtab.html".to_string(), WebviewUrl::App("newtab.html".into())),
    };

    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;
    let (new_tab_id, new_tab_num) = {
        let mut sg = state.lock().unwrap();
        sg.next_tab_num += 1;
        (format!("tab-{}", sg.next_tab_num), sg.next_tab_num)
    };

    let show_toolbar = !is_newtab_url(&target_url);
    println!(
        "[GHOST DEBUG] create_tab: id={}, url={}, show_toolbar={}",
        new_tab_id, target_url, show_toolbar
    );

    let init_script = r#"
        (function() {
            window.addEventListener('keydown', function(e) {
                var ctrl = e.ctrlKey || e.metaKey;
                var shift = e.shiftKey;
                var alt = e.altKey;
                var key = e.key ? e.key.toLowerCase() : '';

                if (ctrl && !shift && key === 't') {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('create_tab', { url: null }).catch(function(){});
                } else if (ctrl && shift && key === 't') {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('reopen_closed_tab').catch(function(){});
                } else if ((ctrl && !shift && key === 'w') || (ctrl && e.key === 'F4')) {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('close_active_tab').catch(function(){});
                } else if ((ctrl && shift && key === 'tab') || (ctrl && e.key === 'PageUp')) {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('prev_tab').catch(function(){});
                } else if ((ctrl && !shift && key === 'tab') || (ctrl && e.key === 'PageDown')) {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('next_tab').catch(function(){});
                } else if (ctrl && !shift && key >= '1' && key <= '8') {
                    e.preventDefault(); e.stopPropagation();
                    var idx = parseInt(key) - 1;
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('switch_tab_by_index', { index: idx }).catch(function(){});
                } else if (ctrl && !shift && key === '9') {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('switch_tab_by_index', { index: 999 }).catch(function(){});
                } else if ((ctrl && !shift && (key === 'l' || key === 'b')) || (alt && key === 'd') || e.key === 'F6') {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('focus_urlbar').catch(function(){});
                } else if ((ctrl && key === 'r') || e.key === 'F5') {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('reload').catch(function(){});
                } else if (alt && (key === 'arrowleft' || key === 'left')) {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('go_back').catch(function(){});
                } else if (alt && (key === 'arrowright' || key === 'right')) {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('go_forward').catch(function(){});
                } else if (e.key === 'F12' || (ctrl && shift && key === 'i')) {
                    e.preventDefault(); e.stopPropagation();
                    if (window.__TAURI__ && window.__TAURI__.core) window.__TAURI__.core.invoke('toggle_devtools').catch(function(){});
                }
            }, true);
        })();
    "#;

    let handle_clone = app_handle.clone();
    let tab_id_clone = new_tab_id.clone();
    let builder = WebviewBuilder::new(&new_tab_id, webview_url)
        .devtools(true)
        .initialization_script(init_script)
        .on_navigation(move |url| {
            let url_str = url.to_string();
            let st = handle_clone.state::<Arc<Mutex<AppState>>>();
            {
                let mut sg = st.lock().unwrap();
                if let Some(tab) = sg.tabs.iter_mut().find(|t| t.id == tab_id_clone) {
                    if tab.url != url_str {
                        tab.url = url_str.clone();
                        let _ = emit_tab_state(&handle_clone, &sg);
                    }
                }
            }
            if !is_newtab_url(&url_str) {
                record_history_visit(&handle_clone, &st, &url_str, "");
            }
            true
        });
    let _ = main_window.add_child(
        builder,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(800.0, 600.0),
    ).map_err(|e| e.to_string())?;

    {
        let mut sg = state.lock().unwrap();
        if let Some(old_id) = &sg.active_id {
            if let Some(old_wv) = app_handle.get_webview(old_id) {
                let _ = old_wv.hide();
            }
        }
        if let Some(new_wv) = app_handle.get_webview(&new_tab_id) {
            let _ = new_wv.show();
        }
        let friendly_title = match target_url.as_str() {
            "newtab.html" => "New Tab".to_string(),
            "history.html" => "History".to_string(),
            "bookmarks.html" => "Bookmarks".to_string(),
            "settings.html" => "Settings".to_string(),
            _ => format!("Tab {}", new_tab_num),
        };
        sg.tabs.push(TabInfo {
            id: new_tab_id.clone(),
            title: friendly_title,
            url: target_url.clone(),
        });
        sg.active_id = Some(new_tab_id.clone());
        sg.toolbar_visible = true;
        emit_tab_state(&app_handle, &sg)?;
        relayout(&app_handle, &sg);
    }

    if !is_newtab_url(&target_url) {
        record_history_visit(&app_handle, state.inner(), &target_url, "");
    }
    Ok(new_tab_id)
}

#[tauri::command]
fn switch_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, tab_id: String) -> Result<(), String> {
    println!("[GHOST DEBUG] switch_tab: to={}", tab_id);
    let mut sg = state.lock().unwrap();
    if sg.active_id.as_deref() == Some(&tab_id) { return Ok(()); }
    if let Some(old_id) = &sg.active_id {
        if let Some(old_wv) = app_handle.get_webview(old_id) { let _ = old_wv.hide(); }
    }
    if let Some(new_wv) = app_handle.get_webview(&tab_id) {
        let _ = new_wv.show();
        let _ = new_wv.set_focus();
    } else {
        return Err(format!("Tab {} not found", tab_id));
    }
    sg.active_id = Some(tab_id.clone());
    sg.toolbar_visible = true;
    emit_tab_state(&app_handle, &sg)?;
    relayout(&app_handle, &sg);
    Ok(())
}

#[tauri::command]
fn close_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, tab_id: String) -> Result<(), String> {
    println!("[GHOST DEBUG] close_tab: id={}", tab_id);
    let mut sg = state.lock().unwrap();
    let pos = sg.tabs.iter().position(|t| t.id == tab_id).ok_or("Tab not found")?;
    let closed_url = sg.tabs[pos].url.clone();
    if !closed_url.is_empty() && !is_newtab_url(&closed_url) {
        sg.closed_tabs_stack.push(closed_url);
    }
    if let Some(wv) = app_handle.get_webview(&tab_id) { let _ = wv.close(); }
    sg.tabs.remove(pos);
    if sg.active_id.as_deref() == Some(&tab_id) {
        if sg.tabs.is_empty() {
            sg.active_id = None;
            sg.toolbar_visible = true;
        } else {
            let np = if pos < sg.tabs.len() { pos } else { sg.tabs.len() - 1 };
            let new_id = sg.tabs[np].id.clone();
            if let Some(nw) = app_handle.get_webview(&new_id) { let _ = nw.show(); let _ = nw.set_focus(); }
            sg.toolbar_visible = true;
            sg.active_id = Some(new_id);
        }
    }
    emit_tab_state(&app_handle, &sg)?;
    relayout(&app_handle, &sg);
    Ok(())
}

#[tauri::command]
fn close_active_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let active_id = state.lock().unwrap().active_id.clone();
    if let Some(id) = active_id {
        close_tab(app_handle, state, id)?;
    }
    Ok(())
}

#[tauri::command]
fn next_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let next_id = {
        let sg = state.lock().unwrap();
        if sg.tabs.len() <= 1 {
            return Ok(());
        }
        let pos = sg.tabs.iter().position(|t| Some(&t.id) == sg.active_id.as_ref()).unwrap_or(0);
        let next_pos = (pos + 1) % sg.tabs.len();
        sg.tabs[next_pos].id.clone()
    };
    switch_tab(app_handle, state, next_id)?;
    Ok(())
}

#[tauri::command]
fn prev_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let prev_id = {
        let sg = state.lock().unwrap();
        if sg.tabs.len() <= 1 {
            return Ok(());
        }
        let pos = sg.tabs.iter().position(|t| Some(&t.id) == sg.active_id.as_ref()).unwrap_or(0);
        let prev_pos = if pos == 0 { sg.tabs.len() - 1 } else { pos - 1 };
        sg.tabs[prev_pos].id.clone()
    };
    switch_tab(app_handle, state, prev_id)?;
    Ok(())
}

#[tauri::command]
fn switch_tab_by_index(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, index: usize) -> Result<(), String> {
    let target_id = {
        let sg = state.lock().unwrap();
        if sg.tabs.is_empty() {
            return Ok(());
        }
        if index == 999 {
            sg.tabs.last().map(|t| t.id.clone())
        } else if index < sg.tabs.len() {
            Some(sg.tabs[index].id.clone())
        } else {
            None
        }
    };
    if let Some(id) = target_id {
        switch_tab(app_handle, state, id)?;
    }
    Ok(())
}

#[tauri::command]
fn reopen_closed_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let last_url = {
        let mut sg = state.lock().unwrap();
        sg.closed_tabs_stack.pop()
    };
    if let Some(url) = last_url {
        create_tab(app_handle, state, Some(url))?;
    } else {
        create_tab(app_handle, state, None)?;
    }
    Ok(())
}

#[tauri::command]
fn navigate(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, url: String) -> Result<(), String> {
    println!("[GHOST DEBUG] navigate: input={}", url);
    let full_url = resolve_input(&url)?;
    let url_parsed: Url = full_url.parse().map_err(|e: url::ParseError| e.to_string())?;

    let (active_id, is_newtab) = {
        let sg = state.lock().unwrap();
        let active_id = sg.active_id.clone();
        let is_newtab = active_id.as_ref().and_then(|id| {
            sg.tabs.iter().find(|t| &t.id == id).map(|t| is_newtab_url(&t.url))
        }).unwrap_or(false);
        (active_id, is_newtab)
    };

    if let Some(active_id) = active_id {
        if is_newtab {
            close_tab(app_handle.clone(), state.clone(), active_id)?;
            create_tab(app_handle, state, Some(url))?;
            return Ok(());
        }

        if let Some(content) = app_handle.get_webview(&active_id) {
            content.navigate(url_parsed).map_err(|e| e.to_string())?;
            let mut sg = state.lock().unwrap();
            if let Some(tab) = sg.tabs.iter_mut().find(|t| t.id == active_id) { tab.url = full_url.clone(); }
            sg.toolbar_visible = true;
            emit_tab_state(&app_handle, &sg)?;
            relayout(&app_handle, &sg);
            return Ok(());
        }
    }
    create_tab(app_handle, state, Some(url))?;
    Ok(())
}

#[tauri::command]
fn update_tab_title(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, tab_id: String, title: String) -> Result<(), String> {
    let history_to_save = {
        let mut sg = state.lock().unwrap();
        let target_url = sg.tabs.iter_mut().find(|t| t.id == tab_id).map(|t| {
            t.title = title.clone();
            t.url.clone()
        });
        if let Some(url) = target_url {
            if let Some(entry) = sg.history.iter_mut().find(|h| h.url == url) {
                entry.title = title;
            }
        }
        sg.history.clone()
    };
    save_history(&app_handle, &history_to_save);
    let sg = state.lock().unwrap();
    emit_tab_state(&app_handle, &sg)?;
    Ok(())
}

#[tauri::command]
fn toggle_devtools(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let id = state.lock().unwrap().active_id.clone().ok_or("No active tab")?;
    if let Some(wv) = app_handle.get_webview(&id) {
        if wv.is_devtools_open() { wv.close_devtools(); } else { wv.open_devtools(); }
    }
    Ok(())
}

#[tauri::command]
fn go_back(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let id = state.lock().unwrap().active_id.clone().ok_or("No active tab")?;
    app_handle.get_webview(&id).ok_or("Not found")?.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
fn go_forward(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let id = state.lock().unwrap().active_id.clone().ok_or("No active tab")?;
    app_handle.get_webview(&id).ok_or("Not found")?.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
fn reload(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    let id = state.lock().unwrap().active_id.clone().ok_or("No active tab")?;
    app_handle.get_webview(&id).ok_or("Not found")?.eval("location.reload()").map_err(|e| e.to_string())
}

// --- History Commands ---

#[tauri::command]
fn get_history(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<HistoryEntry>, String> {
    let sg = state.lock().unwrap();
    Ok(sg.history.clone())
}

#[tauri::command]
fn clear_history(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<(), String> {
    {
        let mut sg = state.lock().unwrap();
        sg.history.clear();
    }
    save_history(&app_handle, &[]);
    Ok(())
}

#[tauri::command]
fn remove_history_entry(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, id: String) -> Result<(), String> {
    let updated = {
        let mut sg = state.lock().unwrap();
        sg.history.retain(|h| h.id != id);
        sg.history.clone()
    };
    save_history(&app_handle, &updated);
    Ok(())
}

#[tauri::command]
fn search_history(state: State<'_, Arc<Mutex<AppState>>>, query: String) -> Result<Vec<HistoryEntry>, String> {
    let q = query.to_lowercase();
    let sg = state.lock().unwrap();
    let matches = sg.history.iter()
        .filter(|h| h.url.to_lowercase().contains(&q) || h.title.to_lowercase().contains(&q))
        .cloned()
        .collect();
    Ok(matches)
}

// --- Bookmarks Commands ---

#[tauri::command]
fn get_bookmarks(state: State<'_, Arc<Mutex<AppState>>>) -> Result<Vec<BookmarkEntry>, String> {
    let sg = state.lock().unwrap();
    Ok(sg.bookmarks.clone())
}

#[tauri::command]
fn add_bookmark(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    url: String,
    title: String,
    folder: Option<String>,
) -> Result<BookmarkEntry, String> {
    if url.is_empty() || is_newtab_url(&url) {
        return Err("Cannot bookmark empty or newtab URL".to_string());
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = BookmarkEntry {
        id: format!("bm-{}", timestamp),
        url: url.clone(),
        title: if title.is_empty() { url } else { title },
        folder,
        created_at: timestamp,
    };

    let updated = {
        let mut sg = state.lock().unwrap();
        if let Some(existing) = sg.bookmarks.iter().find(|b| b.url == entry.url) {
            return Ok(existing.clone());
        }
        sg.bookmarks.push(entry.clone());
        sg.bookmarks.clone()
    };

    save_bookmarks(&app_handle, &updated);
    Ok(entry)
}

#[tauri::command]
fn remove_bookmark(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>, id: String) -> Result<(), String> {
    let updated = {
        let mut sg = state.lock().unwrap();
        sg.bookmarks.retain(|b| b.id != id && b.url != id);
        sg.bookmarks.clone()
    };
    save_bookmarks(&app_handle, &updated);
    Ok(())
}

#[tauri::command]
fn is_bookmarked(state: State<'_, Arc<Mutex<AppState>>>, url: String) -> Result<bool, String> {
    let sg = state.lock().unwrap();
    Ok(sg.bookmarks.iter().any(|b| b.url == url))
}

#[tauri::command]
fn toggle_bookmark_active_tab(app_handle: tauri::AppHandle, state: State<'_, Arc<Mutex<AppState>>>) -> Result<bool, String> {
    let (active_url, active_title) = {
        let sg = state.lock().unwrap();
        let active_id = sg.active_id.as_ref();
        let tab = active_id.and_then(|id| sg.tabs.iter().find(|t| &t.id == id));
        match tab {
            Some(t) => (t.url.clone(), t.title.clone()),
            None => return Err("No active tab".to_string()),
        }
    };

    if active_url.is_empty() || is_newtab_url(&active_url) {
        return Ok(false);
    }

    let is_bm = {
        let sg = state.lock().unwrap();
        sg.bookmarks.iter().any(|b| b.url == active_url)
    };

    if is_bm {
        remove_bookmark(app_handle, state, active_url)?;
        Ok(false)
    } else {
        add_bookmark(app_handle, state, active_url, active_title, None)?;
        Ok(true)
    }
}

// --- Settings Commands ---

#[tauri::command]
fn get_settings(state: State<'_, Arc<Mutex<AppState>>>) -> Result<UserSettings, String> {
    let sg = state.lock().unwrap();
    Ok(sg.settings.clone())
}

#[tauri::command]
fn update_settings(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    settings: UserSettings,
) -> Result<UserSettings, String> {
    {
        let mut sg = state.lock().unwrap();
        sg.settings = settings.clone();
    }
    save_settings(&app_handle, &settings);
    Ok(settings)
}

fn init_env_flags() {
    let args: Vec<String> = std::env::args().collect();

    // Disable WebKit sandboxing issues on Linux
    if std::env::var("WEBKIT_FORCE_SANDBOX").is_err() {
        std::env::set_var("WEBKIT_FORCE_SANDBOX", "0");
    }

    // Default WEBKIT_DISABLE_DMABUF_RENDERER to 1 on Linux to prevent WebKitGTK black surface buffer glitches
    if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() && !args.iter().any(|a| a == "--enable-dmabuf") {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Default WEBKIT_DISABLE_COMPOSITING_MODE to 1 on Linux to prevent WebKitGTK WebProcess crash on heavy sites like Google search
    if std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").is_err() && !args.iter().any(|a| a == "--enable-compositing") {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }

    println!("============================================================");
    println!("[GHOST LOG] WebKitGTK Rendering Environment Flags:");
    println!(
        "  WEBKIT_FORCE_SANDBOX            = {}",
        std::env::var("WEBKIT_FORCE_SANDBOX").unwrap_or_else(|_| "1".into())
    );
    println!(
        "  WEBKIT_DISABLE_COMPOSITING_MODE = {}",
        std::env::var("WEBKIT_DISABLE_COMPOSITING_MODE").unwrap_or_else(|_| "0".into())
    );
    println!(
        "  WEBKIT_DISABLE_DMABUF_RENDERER  = {}",
        std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").unwrap_or_else(|_| "0".into())
    );
    println!("============================================================");
}

fn main() {
    init_env_flags();

    let app_state = Arc::new(Mutex::new(AppState::default()));

    tauri::Builder::default()
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            navigate, go_back, go_forward, reload,
            create_tab, switch_tab, close_tab, close_active_tab,
            next_tab, prev_tab, switch_tab_by_index, reopen_closed_tab,
            focus_urlbar, update_tab_title, toggle_devtools,
            toggle_toolbar, hide_toolbar,
            get_history, clear_history, remove_history_entry, search_history,
            get_bookmarks, add_bookmark, remove_bookmark, is_bookmarked, toggle_bookmark_active_tab,
            get_settings, update_settings
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = app.state::<Arc<Mutex<AppState>>>();

            {
                let mut sg = state.lock().unwrap();
                sg.history = load_history(&handle);
                sg.bookmarks = load_bookmarks(&handle);
                sg.settings = load_settings(&handle);
            }

            let restore_enabled = state.lock().unwrap().settings.restore_session;
            let mut session_restored = false;

            if restore_enabled {
                if let Some(session_path) = get_session_file_path(&handle) {
                    if session_path.exists() {
                        if let Ok(content) = std::fs::read_to_string(&session_path) {
                            if let Ok(payload) = serde_json::from_str::<TabStatePayload>(&content) {
                                if !payload.tabs.is_empty() {
                                    for tab in payload.tabs {
                                        let url_arg = if is_newtab_url(&tab.url) { None } else { Some(tab.url) };
                                        let _ = create_tab(handle.clone(), state.clone(), url_arg);
                                    }
                                    session_restored = true;
                                }
                            }
                        }
                    }
                }
            }

            if !session_restored {
                let _ = create_tab(handle.clone(), state, None);
            }

            // Listen for window resize to relayout
            if let Some(main_window) = app.get_window("main") {
                let h = handle.clone();
                main_window.on_window_event(move |event| {
                    match event {
                        tauri::WindowEvent::Resized(physical_size) => {
                            println!("[GHOST DEBUG] Window Resized event: physical={}x{}", physical_size.width, physical_size.height);
                            let st = h.state::<Arc<Mutex<AppState>>>();
                            let sg = st.lock().unwrap();
                            relayout(&h, &sg);
                        }
                        tauri::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                            println!("[GHOST DEBUG] ScaleFactorChanged event: factor={}", scale_factor);
                            let st = h.state::<Arc<Mutex<AppState>>>();
                            let sg = st.lock().unwrap();
                            relayout(&h, &sg);
                        }
                        _ => {}
                    }
                });
            }

            // Capture universal browser shortcuts at GTK window level (works on all websites)
            #[cfg(target_os = "linux")]
            {
                if let Some(main_window) = app.get_window("main") {
                    if let Ok(gtk_window) = main_window.gtk_window() {
                        let hc = handle.clone();
                        gtk_window.connect_key_press_event(move |_win, event| {
                            let keyval = event.keyval();
                            let ev_state = event.state();
                            let ctrl = ev_state.contains(gdk::ModifierType::CONTROL_MASK);
                            let shift = ev_state.contains(gdk::ModifierType::SHIFT_MASK);
                            let alt = ev_state.contains(gdk::ModifierType::MOD1_MASK);

                            use gdk::keys::constants::*;

                            // Ctrl + T (New Tab)
                            if ctrl && !shift && !alt && (keyval == t || keyval == T) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = create_tab(hc.clone(), st, None);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + Shift + T (Reopen Closed Tab)
                            if ctrl && shift && !alt && (keyval == t || keyval == T) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = reopen_closed_tab(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + W / Ctrl + F4 (Close Tab)
                            if (ctrl && !shift && !alt && (keyval == w || keyval == W))
                                || (ctrl && keyval == F4)
                            {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = close_active_tab(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + Tab / Ctrl + PageDown (Next Tab)
                            if (ctrl && !shift && keyval == Tab) || (ctrl && keyval == Page_Down) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = next_tab(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + Shift + Tab / Ctrl + PageUp (Prev Tab)
                            if (ctrl && (keyval == ISO_Left_Tab || (shift && keyval == Tab)))
                                || (ctrl && keyval == Page_Up)
                            {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = prev_tab(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + 1..8 (Switch to Tab N)
                            if ctrl && !shift && !alt {
                                let idx_opt = match keyval {
                                    _1 => Some(0),
                                    _2 => Some(1),
                                    _3 => Some(2),
                                    _4 => Some(3),
                                    _5 => Some(4),
                                    _6 => Some(5),
                                    _7 => Some(6),
                                    _8 => Some(7),
                                    _9 => Some(999), // Ctrl + 9 switches to last tab
                                    _ => None,
                                };
                                if let Some(idx) = idx_opt {
                                    let st = hc.state::<Arc<Mutex<AppState>>>();
                                    let _ = switch_tab_by_index(hc.clone(), st, idx);
                                    return gtk::glib::Propagation::Stop;
                                }
                            }

                            // Ctrl + L / Ctrl + B (Focus URL bar / Toggle toolbar)
                            if (ctrl && !shift && (keyval == l || keyval == L))
                                || (alt && (keyval == d || keyval == D))
                                || keyval == F6
                            {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = focus_urlbar(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + B (Toggle Toolbar Visibility)
                            if ctrl && !shift && !alt && (keyval == b || keyval == B) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let mut sg = st.lock().unwrap();
                                sg.toolbar_visible = !sg.toolbar_visible;
                                if sg.toolbar_visible {
                                    let _ = hc.emit_to("main", "toolbar-shown", ());
                                } else {
                                    let _ = hc.emit_to("main", "toolbar-hidden", ());
                                }
                                relayout(&hc, &sg);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + R / F5 (Reload)
                            if (ctrl && !shift && (keyval == r || keyval == R)) || keyval == F5 {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = reload(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Alt + Left / Alt + Right (Go Back / Go Forward)
                            if alt && (keyval == Left || keyval == KP_Left) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = go_back(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }
                            if alt && (keyval == Right || keyval == KP_Right) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = go_forward(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + Shift + I / F12 (Toggle DevTools)
                            if (ctrl && shift && (keyval == i || keyval == I)) || keyval == F12 {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = toggle_devtools(hc.clone(), st);
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + H (Open History)
                            if ctrl && !shift && !alt && (keyval == h || keyval == H) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = create_tab(hc.clone(), st, Some("history.html".to_string()));
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + Shift + O (Open Bookmarks)
                            if ctrl && shift && !alt && (keyval == o || keyval == O) {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = create_tab(hc.clone(), st, Some("bookmarks.html".to_string()));
                                return gtk::glib::Propagation::Stop;
                            }

                            // Ctrl + Comma (Open Settings)
                            if ctrl && !shift && !alt && keyval == comma {
                                let st = hc.state::<Arc<Mutex<AppState>>>();
                                let _ = create_tab(hc.clone(), st, Some("settings.html".to_string()));
                                return gtk::glib::Propagation::Stop;
                            }

                            gtk::glib::Propagation::Proceed
                        });
                    }
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}