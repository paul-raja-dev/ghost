#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

use tauri::{Manager, WebviewUrl, WebviewBuilder, LogicalPosition, LogicalSize};

const TOOLBAR_HEIGHT: f64 = 46.0;

#[tauri::command]
fn navigate(app_handle: tauri::AppHandle, url: String) {
    let full_url = if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else {
        format!("https://{}", url)
    };

    if let Some(main_window) = app_handle.get_window("main") {
        if let Some(content) = app_handle.get_webview("content") {
            let _ = content.navigate(full_url.parse().unwrap());
        } else {
            let size = main_window.inner_size().unwrap();
            let url_parsed = full_url.parse().unwrap();

            let builder = WebviewBuilder::new("content", WebviewUrl::External(url_parsed));
            
            let _ = main_window.add_child(
                builder,
                LogicalPosition::new(0.0, TOOLBAR_HEIGHT),
                LogicalSize::new(
                    size.width as f64,
                    size.height as f64 - TOOLBAR_HEIGHT,
                ),
            );
        }
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![navigate])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}