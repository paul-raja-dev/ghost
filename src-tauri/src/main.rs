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

/// Robust GTK Layout Manager:
/// 1. Toolbar widget (children[0]): fixed 76px height.
/// 2. Active tab widget: shown with expand=true (fills 100% of remaining space).
/// 3. Inactive tab widgets: hidden with expand=false.
fn fix_gtk_layout(main_window: &tauri::Window, state: &AppState) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if children.is_empty() {
                return;
            }

            if let Some(toolbar_widget) = children.get(0) {
                toolbar_widget.set_size_request(-1, TOOLBAR_HEIGHT as i32);
                gtk_box.set_child_packing(toolbar_widget, false, true, 0, gtk::PackType::Start);
            }

            let active_index = state.active_id.as_ref().and_then(|active_id| {
                state.tabs.iter().position(|t| &t.id == active_id)
            });

            for (idx, widget) in children.iter().skip(1).enumerate() {
                let is_active = Some(idx) == active_index;
                if is_active {
                    widget.show();
                    gtk_box.set_child_packing(widget, true, true, 0, gtk::PackType::Start);
                } else {
                    widget.hide();
                    gtk_box.set_child_packing(widget, false, false, 0, gtk::PackType::Start);
                }
            }
        }
    }
}

/// Adaptive Toolbar Visibility:
/// - If active tab is newtab.html -> hide toolbar (GTK collapses toolbar height to 0px).
/// - If active tab is external page (Google, etc.) -> show toolbar (GTK allocates 76px at top).
fn apply_adaptive_toolbar(main_window: &tauri::Window, state: &AppState, active_tab_url: Option<&str>) {
    #[cfg(target_os = "linux")]
    {
        let is_newtab = match active_tab_url {
            Some(u) => u == "newtab.html" || u.contains("newtab.html"),
            None => true,
        };

        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if let Some(toolbar_widget) = children.get(0) {
                if is_newtab {
                    toolbar_widget.hide();
                } else {
                    toolbar_widget.show();
                }
                fix_gtk_layout(main_window, state);
            }
        }
    }
}

#[tauri::command]
fn toggle_toolbar(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<bool, String> {
    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;
    let state_guard = state.lock().unwrap();
    
    #[cfg(target_os = "linux")]
    {
        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if let Some(toolbar_widget) = children.get(0) {
                if toolbar_widget.is_visible() {
                    toolbar_widget.hide();
                    let _ = app_handle.emit_to("main", "toolbar-hidden", ());
                    fix_gtk_layout(&main_window, &state_guard);
                    return Ok(false);
                } else {
                    toolbar_widget.show();
                    let _ = app_handle.emit_to("main", "toolbar-shown", ());
                    fix_gtk_layout(&main_window, &state_guard);
                    return Ok(true);
                }
            }
        }
    }
    
    Ok(false)
}

#[tauri::command]
fn hide_toolbar(
    app_handle: tauri::AppHandle,
    state: State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let main_window = app_handle.get_window("main").ok_or("Main window not found")?;
    let state_guard = state.lock().unwrap();
    
    #[cfg(target_os = "linux")]
    {
        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if let Some(toolbar_widget) = children.get(0) {
                toolbar_widget.hide();
                fix_gtk_layout(&main_window, &state_guard);
            }
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
        webview_url,
    )
    .devtools(true);

    let _ = main_window.add_child(
        builder,
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(
            size.width,
            size.height,
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
            title: if target_url == "newtab.html" { "New Tab".to_string() } else { format!("Tab {}", new_tab_num) },
            url: target_url.clone(),
        });
        state_guard.active_id = Some(new_tab_id.clone());

        emit_tab_state(&app_handle, &state_guard)?;

        apply_adaptive_toolbar(&main_window, &state_guard, Some(&target_url));
    }

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

    state_guard.active_id = Some(tab_id.clone());
    emit_tab_state(&app_handle, &state_guard)?;

    let target_url = state_guard.tabs.iter().find(|t| t.id == tab_id).map(|t| t.url.clone());
    apply_adaptive_toolbar(&main_window, &state_guard, target_url.as_deref());

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

    let active_url = if state_guard.active_id.as_deref() == Some(&tab_id) {
        if state_guard.tabs.is_empty() {
            state_guard.active_id = None;
            None
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
            state_guard.active_id = Some(new_active_id.clone());
            state_guard.tabs[new_pos].url.clone().into()
        }
    } else {
        state_guard.active_id.as_ref().and_then(|id| {
            state_guard.tabs.iter().find(|t| &t.id == id).map(|t| t.url.clone())
        })
    };

    emit_tab_state(&app_handle, &state_guard)?;

    apply_adaptive_toolbar(&main_window, &state_guard, active_url.as_deref());

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

            if let Some(main_window) = app_handle.get_window("main") {
                apply_adaptive_toolbar(&main_window, &state_guard, Some(&full_url));
            }
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
            toggle_devtools,
            toggle_toolbar,
            hide_toolbar
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = app.state::<Arc<Mutex<AppState>>>();
            let _ = create_tab(handle.clone(), state, None);

            // Set up GTK: hide toolbar on newtab by default, capture Ctrl+B globally
            #[cfg(target_os = "linux")]
            {
                if let Some(main_window) = app.get_window("main") {
                    let state_guard = app.state::<Arc<Mutex<AppState>>>();
                    let state_locked = state_guard.lock().unwrap();
                    apply_adaptive_toolbar(&main_window, &state_locked, Some("newtab.html"));

                    // Capture Ctrl+B at GTK window level
                    if let Ok(gtk_window) = main_window.gtk_window() {
                        let handle_clone = handle.clone();
                        gtk_window.connect_key_press_event(move |_win, event| {
                            let keyval = event.keyval();
                            let state = event.state();
                            let is_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
                            
                            // Ctrl+B = toggle toolbar manually
                            if is_ctrl && (keyval == gdk::keys::constants::b || keyval == gdk::keys::constants::B) {
                                if let Some(mw) = handle_clone.get_window("main") {
                                    if let Ok(vbox) = mw.default_vbox() {
                                        let ch = vbox.children();
                                        if let Some(tw) = ch.get(0) {
                                            let app_st = handle_clone.state::<Arc<Mutex<AppState>>>();
                                            let st = app_st.lock().unwrap();
                                            if tw.is_visible() {
                                                tw.hide();
                                                let _ = handle_clone.emit_to("main", "toolbar-hidden", ());
                                            } else {
                                                tw.show();
                                                let _ = handle_clone.emit_to("main", "toolbar-shown", ());
                                            }
                                            fix_gtk_layout(&mw, &st);
                                        }
                                    }
                                }
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