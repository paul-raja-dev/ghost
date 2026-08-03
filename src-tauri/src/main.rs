#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use tauri::{Emitter, Manager, WebviewUrl, WebviewBuilder, LogicalPosition, LogicalSize};
use url::Url;

#[cfg(target_os = "linux")]
use gtk::prelude::*;

const TOOLBAR_HEIGHT: f64 = 46.0;

/// Determines if the input looks like a URL or a search query.
/// Returns the final URL string to navigate to.
fn resolve_input(input: &str) -> Result<String, String> {
    let trimmed = input.trim();

    if trimmed.is_empty() {
        return Err("Empty input".to_string());
    }

    // Block dangerous schemes
    let lower = trimmed.to_lowercase();
    if lower.starts_with("file://") || lower.starts_with("javascript:") || lower.starts_with("data:") {
        return Err(format!("Blocked scheme: {}", trimmed));
    }

    // If it already has a valid scheme, use it directly
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Url::parse(trimmed)
            .map(|u| u.to_string())
            .map_err(|e| format!("Invalid URL: {}", e));
    }

    // Try prepending https:// — if it parses as a valid URL with a dot in the host,
    // it's likely a URL (e.g., "github.com", "localhost:3000")
    let with_scheme = format!("https://{}", trimmed);
    if let Ok(parsed) = Url::parse(&with_scheme) {
        if let Some(host) = parsed.host_str() {
            // Has a dot (github.com) or is localhost — treat as URL
            if host.contains('.') || host == "localhost" {
                return Ok(with_scheme);
            }
        }
    }

    // Otherwise, it's a search query — send to DuckDuckGo
    let encoded = trimmed.replace(' ', "+");
    Ok(format!("https://duckduckgo.com/?q={}", encoded))
}

/// Helper function to configure GTK packing on Linux so toolbar doesn't expand
fn fix_gtk_layout(main_window: &tauri::Window) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(gtk_box) = main_window.default_vbox() {
            let children = gtk_box.children();
            if let Some(toolbar_widget) = children.get(0) {
                toolbar_widget.set_size_request(-1, TOOLBAR_HEIGHT as i32);
                gtk_box.set_child_packing(toolbar_widget, false, true, 0, gtk::PackType::Start);
            }
            if let Some(content_widget) = children.get(1) {
                gtk_box.set_child_packing(content_widget, true, true, 0, gtk::PackType::Start);
            }
        }
    }
}

/// Navigate the content webview to a URL.
/// Creates the content webview on first call, reuses it on subsequent calls.
#[tauri::command]
fn navigate(app_handle: tauri::AppHandle, url: String) -> Result<(), String> {
    let full_url = resolve_input(&url)?;

    let url_parsed: Url = full_url.parse().map_err(|e: url::ParseError| e.to_string())?;

    if let Some(content) = app_handle.get_webview("content") {
        // Content webview already exists — just navigate it
        content
            .navigate(url_parsed)
            .map_err(|e| e.to_string())?;
    } else if let Some(main_window) = app_handle.get_window("main") {
        // First navigation — create the content webview
        let physical_size = main_window.inner_size().map_err(|e| e.to_string())?;
        let scale_factor = main_window.scale_factor().map_err(|e| e.to_string())?;
        let size = physical_size.to_logical::<f64>(scale_factor);

        let builder = WebviewBuilder::new(
            "content",
            WebviewUrl::External(url_parsed),
        );

        main_window
            .add_child(
                builder,
                LogicalPosition::new(0.0, TOOLBAR_HEIGHT),
                LogicalSize::new(
                    size.width,
                    (size.height - TOOLBAR_HEIGHT).max(0.0),
                ),
            )
            .map_err(|e| e.to_string())?;

        // Adjust GTK box child packing so toolbar stays fixed at 46px and content fills the rest
        fix_gtk_layout(&main_window);
    } else {
        return Err("Main window not found".to_string());
    }

    // Emit the resolved URL back to the toolbar so the URL bar stays in sync
    app_handle
        .emit_to("main", "url-changed", full_url)
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Navigate the content webview back in history.
#[tauri::command]
fn go_back(app_handle: tauri::AppHandle) -> Result<(), String> {
    let webview = app_handle
        .get_webview("content")
        .ok_or("No page loaded yet")?;

    webview.eval("history.back()").map_err(|e| e.to_string())
}

/// Navigate the content webview forward in history.
#[tauri::command]
fn go_forward(app_handle: tauri::AppHandle) -> Result<(), String> {
    let webview = app_handle
        .get_webview("content")
        .ok_or("No page loaded yet")?;

    webview.eval("history.forward()").map_err(|e| e.to_string())
}

/// Reload the current page in the content webview.
#[tauri::command]
fn reload(app_handle: tauri::AppHandle) -> Result<(), String> {
    let webview = app_handle
        .get_webview("content")
        .ok_or("No page loaded yet")?;

    webview.eval("location.reload()").map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![navigate, go_back, go_forward, reload])
        .setup(|app| {
            if let Some(main_window) = app.get_window("main") {
                fix_gtk_layout(&main_window);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}