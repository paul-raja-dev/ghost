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

#[derive(Debug, Default)]
pub struct AppState {
    pub tabs: Vec<TabInfo>,
    pub active_id: Option<String>,
    pub next_tab_num: usize,
    pub toolbar_visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabStatePayload {
    pub tabs: Vec<TabInfo>,
    pub active_id: Option<String>,
}

fn resolve_input(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Empty input".to_string());
    }
    let lower = trimmed.to_lowercase();
    if lower.starts_with("file://") || lower.starts_with("javascript:") || lower.starts_with("data:") {
        return Err(format!("Blocked scheme: {}", trimmed));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed)
            .map(|u| u.to_string())
            .map_err(|e| format!("Invalid URL: {}", e));
    }
    let with_scheme = format!("https://{}", trimmed);
    if let Ok(parsed) = Url::parse(&with_scheme) {
        if let Some(host) = parsed.host_str() {
            if host.contains('.') || host == "localhost" {
                return Ok(with_scheme);
            }
        }
    }
    let encoded = trimmed.replace(' ', "+");
    Ok(format!("https://www.google.com/search?q={}", encoded))
}

fn emit_tab_state(app_handle: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let payload = TabStatePayload {
        tabs: state.tabs.clone(),
        active_id: state.active_id.clone(),
    };
    app_handle.emit_to("main", "tabs-changed", payload).map_err(|e| e.to_string())?;
    if let Some(active_id) = &state.active_id {
        if let Some(active_tab) = state.tabs.iter().find(|t| &t.id == active_id) {
            app_handle.emit_to("main", "url-changed", active_tab.url.clone()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Resize active webview to fill the correct area based on toolbar visibility.
/// This is the ONLY layout function needed. No GTK packing tricks.
fn resize_active_webview(app_handle: &tauri::AppHandle, state: &AppState) {
    let Some(active_id) = &state.active_id else { return };
    let Some(main_window) = app_handle.get_window("main") else { return };
    let Ok(physical_size) = main_window.inner_size() else { return };
    let Ok(scale_factor) = main_window.scale_factor() else { return };
    let size = physical_size.to_logical::<f64>(scale_factor);

    let (y_offset, content_height) = if state.toolbar_visible {
        (TOOLBAR_HEIGHT, (size.height - TOOLBAR_HEIGHT).max(1.0))
    } else {
        (0.0, size.height.max(1.0))
    };

    if let Some(webview) = app_handle.get_webview(active_id) {
        let _ = webview.set_position(LogicalPosition::new(0.0, y_offset));
        let _ = webview.set_size(LogicalSize::new(size.width, content_height));
    }

    // Also hide/show the toolbar webview (index 0 child)
    if let Some(toolbar_wv) = app_handle.get_webview("main") {
        if state.toolbar_visible {
            let _ = toolbar_wv.set_position(LogicalPosition::new(0.0, 0.0));
            let _ = toolbar_wv.set_size(LogicalSize::new(size.width, TOOLBAR_HEIGHT));
        }
    }
}

/// Decide toolbar visibility based on active tab URL
fn is_newtab_url(url: &str) -> bool {
    url == "newtab.html" || url.contains("newtab.html")
}

#[tauri::command]
fn toggle_toolbar(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<bool, String> {
    let mut state_guard = state.lock().unwrap();
    state_guard.toolbar_visible = !state_guard.toolbar_visible;
    let visible = state_guard.toolbar_visible;

    if visible {
        let _ = app_handle.emit_to("main", "toolbar-shown", ());
    } else {
        let _ = app_handle.emit_to("main", "toolbar-hidden", ());
    }

    resize_active_webview(&app_handle, &state_guard);
    Ok(visible)
}

#[tauri::command]
fn hide_toolbar(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let mut state_guard = state.lock().unwrap();
    state_guard.toolbar_visible = false;
    resize_active_webview(&app_handle, &state_guard);
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
            let resolved = resolve_input(u)?;
            let parsed: Url = resolved.parse().map_err(|e: url::ParseError| e.to_string())?;
            (resolved, WebviewUrl::External(parsed))
        }
        _ => (
            "newtab.html".to_string(),
            WebviewUrl::App("newtab.html".into()),
        ),
    };

    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;

    let (new_tab_id, new_tab_num) = {
        let mut sg = state.lock().unwrap();
        sg.next_tab_num += 1;
        (format!("tab-{}", sg.next_tab_num), sg.next_tab_num)
    };

    let physical_size = main_window.inner_size().map_err(|e| e.to_string())?;
    let scale_factor = main_window.scale_factor().map_err(|e| e.to_string())?;
    let size = physical_size.to_logical::<f64>(scale_factor);

    // Determine if toolbar should be visible for this tab
    let show_toolbar = !is_newtab_url(&target_url);
    let y_offset = if show_toolbar { TOOLBAR_HEIGHT } else { 0.0 };
    let content_height = if show_toolbar { (size.height - TOOLBAR_HEIGHT).max(1.0) } else { size.height };

    let builder = WebviewBuilder::new(&new_tab_id, webview_url).devtools(true);

    let _ = main_window.add_child(
        builder,
        LogicalPosition::new(0.0, y_offset),
        LogicalSize::new(size.width, content_height),
    ).map_err(|e| e.to_string())?;

    {
        let mut sg = state.lock().unwrap();

        // Hide old active tab
        if let Some(old_id) = &sg.active_id {
            if let Some(old_wv) = app_handle.get_webview(old_id) {
                let _ = old_wv.hide();
            }
        }

        // Show new tab
        if let Some(new_wv) = app_handle.get_webview(&new_tab_id) {
            let _ = new_wv.show();
        }

        sg.tabs.push(TabInfo {
            id: new_tab_id.clone(),
            title: if target_url == "newtab.html" { "New Tab".to_string() } else { format!("Tab {}", new_tab_num) },
            url: target_url.clone(),
        });
        sg.active_id = Some(new_tab_id.clone());
        sg.toolbar_visible = show_toolbar;

        emit_tab_state(&app_handle, &sg)?;
        resize_active_webview(&app_handle, &sg);
    }

    Ok(new_tab_id)
}

#[tauri::command]
fn switch_tab(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    tab_id: String,
) -> Result<(), String> {
    let mut sg = state.lock().unwrap();
    if sg.active_id.as_deref() == Some(&tab_id) {
        return Ok(());
    }

    if let Some(old_id) = &sg.active_id {
        if let Some(old_wv) = app_handle.get_webview(old_id) {
            let _ = old_wv.hide();
        }
    }

    if let Some(new_wv) = app_handle.get_webview(&tab_id) {
        let _ = new_wv.show();
        let _ = new_wv.set_focus();
    } else {
        return Err(format!("Tab webview {} not found", tab_id));
    }

    sg.active_id = Some(tab_id.clone());

    // Adaptive toolbar
    let show_toolbar = sg.tabs.iter()
        .find(|t| t.id == tab_id)
        .map(|t| !is_newtab_url(&t.url))
        .unwrap_or(false);
    sg.toolbar_visible = show_toolbar;

    emit_tab_state(&app_handle, &sg)?;
    resize_active_webview(&app_handle, &sg);

    Ok(())
}

#[tauri::command]
fn close_tab(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    tab_id: String,
) -> Result<(), String> {
    let mut sg = state.lock().unwrap();

    let pos = sg.tabs.iter().position(|t| t.id == tab_id).ok_or("Tab not found")?;

    if let Some(wv) = app_handle.get_webview(&tab_id) {
        let _ = wv.close();
    }

    sg.tabs.remove(pos);

    if sg.active_id.as_deref() == Some(&tab_id) {
        if sg.tabs.is_empty() {
            sg.active_id = None;
            sg.toolbar_visible = false;
        } else {
            let new_pos = if pos < sg.tabs.len() { pos } else { sg.tabs.len() - 1 };
            let new_id = sg.tabs[new_pos].id.clone();
            if let Some(new_wv) = app_handle.get_webview(&new_id) {
                let _ = new_wv.show();
                let _ = new_wv.set_focus();
            }
            let show_toolbar = !is_newtab_url(&sg.tabs[new_pos].url);
            sg.active_id = Some(new_id);
            sg.toolbar_visible = show_toolbar;
        }
    }

    emit_tab_state(&app_handle, &sg)?;
    resize_active_webview(&app_handle, &sg);

    Ok(())
}

#[tauri::command]
fn navigate(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    url: String,
) -> Result<(), String> {
    let full_url = resolve_input(&url)?;
    let url_parsed: Url = full_url.parse().map_err(|e: url::ParseError| e.to_string())?;

    let active_id = {
        let sg = state.lock().unwrap();
        sg.active_id.clone()
    };

    if let Some(active_id) = active_id {
        if let Some(content) = app_handle.get_webview(&active_id) {
            content.navigate(url_parsed).map_err(|e| e.to_string())?;

            let mut sg = state.lock().unwrap();
            if let Some(tab) = sg.tabs.iter_mut().find(|t| t.id == active_id) {
                tab.url = full_url.clone();
            }
            sg.toolbar_visible = !is_newtab_url(&full_url);
            emit_tab_state(&app_handle, &sg)?;
            resize_active_webview(&app_handle, &sg);
            return Ok(());
        }
    }

    create_tab(app_handle, state, Some(url))?;
    Ok(())
}

#[tauri::command]
fn update_tab_title(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    tab_id: String,
    title: String,
) -> Result<(), String> {
    let mut sg = state.lock().unwrap();
    if let Some(tab) = sg.tabs.iter_mut().find(|t| t.id == tab_id) {
        tab.title = title;
    }
    emit_tab_state(&app_handle, &sg)?;
    Ok(())
}

#[tauri::command]
fn toggle_devtools(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let active_id = {
        let sg = state.lock().unwrap();
        sg.active_id.clone().ok_or("No active tab")?
    };
    if let Some(wv) = app_handle.get_webview(&active_id) {
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

fn main() {
    let app_state = Arc::new(Mutex::new(AppState::default()));

    tauri::Builder::default()
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            navigate, go_back, go_forward, reload,
            create_tab, switch_tab, close_tab,
            update_tab_title, toggle_devtools,
            toggle_toolbar, hide_toolbar
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = app.state::<Arc<Mutex<AppState>>>();
            let _ = create_tab(handle.clone(), state, None);

            // Capture Ctrl+B at GTK window level
            #[cfg(target_os = "linux")]
            {
                if let Some(main_window) = app.get_window("main") {
                    if let Ok(gtk_window) = main_window.gtk_window() {
                        let handle_clone = handle.clone();
                        gtk_window.connect_key_press_event(move |_win, event| {
                            let keyval = event.keyval();
                            let ev_state = event.state();
                            let is_ctrl = ev_state.contains(gdk::ModifierType::CONTROL_MASK);

                            if is_ctrl && (keyval == gdk::keys::constants::b || keyval == gdk::keys::constants::B) {
                                let app_st = handle_clone.state::<Arc<Mutex<AppState>>>();
                                let mut sg = app_st.lock().unwrap();
                                sg.toolbar_visible = !sg.toolbar_visible;
                                if sg.toolbar_visible {
                                    let _ = handle_clone.emit_to("main", "toolbar-shown", ());
                                } else {
                                    let _ = handle_clone.emit_to("main", "toolbar-hidden", ());
                                }
                                resize_active_webview(&handle_clone, &sg);
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