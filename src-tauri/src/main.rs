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
    app_handle
        .emit_to("main", "tabs-changed", payload)
        .map_err(|e| e.to_string())?;

    if let Some(active_id) = &state.active_id {
        if let Some(active_tab) = state.tabs.iter().find(|t| &t.id == active_id) {
            app_handle
                .emit_to("main", "url-changed", active_tab.url.clone())
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

#[tauri::command]
fn create_tab(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    url: Option<String>,
) -> Result<String, String> {
    let target_url = resolve_input(url.as_deref().unwrap_or("https://www.google.com"))?;
    let url_parsed: Url = target_url.parse().map_err(|e: url::ParseError| e.to_string())?;

    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;

    let (new_tab_id, new_tab_num) = {
        let mut state_guard = state.lock().unwrap();
        state_guard.next_tab_num += 1;
        let tab_num = state_guard.next_tab_num;
        let tab_id = format!("tab-{}", tab_num);
        (tab_id, tab_num)
    };

    let physical_size = main_window.inner_size().map_err(|e| e.to_string())?;
    let scale_factor = main_window.scale_factor().map_err(|e| e.to_string())?;
    let size = physical_size.to_logical::<f64>(scale_factor);

    let builder = WebviewBuilder::new(
        &new_tab_id,
        WebviewUrl::External(url_parsed),
    )
    .devtools(true);

    let _ = main_window.add_child(
        builder,
        LogicalPosition::new(0.0, TOOLBAR_HEIGHT),
        LogicalSize::new(
            size.width,
            (size.height - TOOLBAR_HEIGHT).max(0.0),
        ),
    ).map_err(|e| e.to_string())?;

    {
        let mut state_guard = state.lock().unwrap();

        if let Some(old_active_id) = &state_guard.active_id {
            if let Some(old_webview) = app_handle.get_webview(old_active_id) {
                let _ = old_webview.hide();
            }
        }

        if let Some(new_webview) = app_handle.get_webview(&new_tab_id) {
            let _ = new_webview.show();
        }

        state_guard.tabs.push(TabInfo {
            id: new_tab_id.clone(),
            title: format!("Tab {}", new_tab_num),
            url: target_url,
        });
        state_guard.active_id = Some(new_tab_id.clone());

        emit_tab_state(&app_handle, &state_guard)?;
    }

    fix_gtk_layout(&main_window);

    Ok(new_tab_id)
}

#[tauri::command]
fn switch_tab(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    tab_id: String,
) -> Result<(), String> {
    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;

    let mut state_guard = state.lock().unwrap();
    if state_guard.active_id.as_deref() == Some(&tab_id) {
        return Ok(());
    }

    if let Some(old_id) = &state_guard.active_id {
        if let Some(old_webview) = app_handle.get_webview(old_id) {
            let _ = old_webview.hide();
        }
    }

    if let Some(new_webview) = app_handle.get_webview(&tab_id) {
        let _ = new_webview.show();
        let _ = new_webview.set_focus();
    } else {
        return Err(format!("Tab webview {} not found", tab_id));
    }

    state_guard.active_id = Some(tab_id);
    emit_tab_state(&app_handle, &state_guard)?;

    fix_gtk_layout(&main_window);

    Ok(())
}

#[tauri::command]
fn close_tab(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
    tab_id: String,
) -> Result<(), String> {
    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;

    let mut state_guard = state.lock().unwrap();

    let pos = state_guard
        .tabs
        .iter()
        .position(|t| t.id == tab_id)
        .ok_or("Tab not found")?;

    if let Some(webview) = app_handle.get_webview(&tab_id) {
        let _ = webview.close();
    }

    state_guard.tabs.remove(pos);

    if state_guard.active_id.as_deref() == Some(&tab_id) {
        if state_guard.tabs.is_empty() {
            state_guard.active_id = None;
        } else {
            let new_pos = if pos < state_guard.tabs.len() {
                pos
            } else {
                state_guard.tabs.len() - 1
            };
            let new_active_id = state_guard.tabs[new_pos].id.clone();
            if let Some(new_webview) = app_handle.get_webview(&new_active_id) {
                let _ = new_webview.show();
                let _ = new_webview.set_focus();
            }
            state_guard.active_id = Some(new_active_id);
        }
    }

    emit_tab_state(&app_handle, &state_guard)?;

    fix_gtk_layout(&main_window);

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
        let state_guard = state.lock().unwrap();
        state_guard.active_id.clone()
    };

    if let Some(active_id) = active_id {
        if let Some(content) = app_handle.get_webview(&active_id) {
            content.navigate(url_parsed).map_err(|e| e.to_string())?;

            let mut state_guard = state.lock().unwrap();
            if let Some(tab) = state_guard.tabs.iter_mut().find(|t| t.id == active_id) {
                tab.url = full_url.clone();
            }
            emit_tab_state(&app_handle, &state_guard)?;
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
    let mut state_guard = state.lock().unwrap();
    if let Some(tab) = state_guard.tabs.iter_mut().find(|t| t.id == tab_id) {
        tab.title = title;
    }
    emit_tab_state(&app_handle, &state_guard)?;
    Ok(())
}

#[tauri::command]
fn toggle_devtools(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let active_id = {
        let state_guard = state.lock().unwrap();
        state_guard.active_id.clone().ok_or("No active tab")?
    };

    if let Some(webview) = app_handle.get_webview(&active_id) {
        if webview.is_devtools_open() {
            webview.close_devtools();
        } else {
            webview.open_devtools();
        }
    }

    Ok(())
}

#[tauri::command]
fn go_back(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let active_id = {
        let state_guard = state.lock().unwrap();
        state_guard.active_id.clone().ok_or("No active tab")?
    };

    let webview = app_handle
        .get_webview(&active_id)
        .ok_or("Active webview not found")?;

    webview.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
fn go_forward(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let active_id = {
        let state_guard = state.lock().unwrap();
        state_guard.active_id.clone().ok_or("No active tab")?
    };

    let webview = app_handle
        .get_webview(&active_id)
        .ok_or("Active webview not found")?;

    webview.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
fn reload(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let active_id = {
        let state_guard = state.lock().unwrap();
        state_guard.active_id.clone().ok_or("No active tab")?
    };

    let webview = app_handle
        .get_webview(&active_id)
        .ok_or("Active webview not found")?;

    webview.eval("location.reload()").map_err(|e| e.to_string())
}

fn fix_gtk_layout(main_window: &tauri::Window) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if let Some(toolbar_widget) = children.get(0) {
                toolbar_widget.set_size_request(-1, TOOLBAR_HEIGHT as i32);
                gtk_box.set_child_packing(toolbar_widget, false, true, 0, gtk::PackType::Start);
            }
            for widget in children.iter().skip(1) {
                gtk_box.set_child_packing(widget, true, true, 0, gtk::PackType::Start);
            }
        }
    }
}

fn main() {
    let app_state = Arc::new(Mutex::new(AppState::default()));

    tauri::Builder::default()
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            navigate,
            go_back,
            go_forward,
            reload,
            create_tab,
            switch_tab,
            close_tab,
            update_tab_title,
            toggle_devtools
        ])
        .setup(move |app| {
            if let Some(main_window) = app.get_window("main") {
                fix_gtk_layout(&main_window);
            }

            let handle = app.handle().clone();
            let state = app.state::<Arc<Mutex<AppState>>>();
            let _ = create_tab(handle, state, Some("https://www.google.com".to_string()));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}