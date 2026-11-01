// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use nestfactory_service::{Backend, EventSink};
use serde_json::Value;
use tauri::{Emitter, Manager, WindowEvent};

struct TauriSink(tauri::AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.0.emit(event, payload);
    }
}

struct AppState(Backend);

/// The single, typed-by-convention boundary: every UI call goes through here to
/// `Backend::dispatch`, on a blocking thread so long tasks never freeze the window.
#[tauri::command]
async fn backend(state: tauri::State<'_, AppState>, cmd: String, args: Value) -> Result<Value, String> {
    let b = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || b.dispatch(&cmd, args)).await.map_err(|e| e.to_string())?
}

/// A project path passed on the command line, e.g. `nestfactory C:\dev\orders-api`.
#[tauri::command]
fn launch_project() -> Option<String> {
    std::env::args().nth(1).filter(|a| !a.starts_with('-'))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let backend = Backend::new(dir, Arc::new(TauriSink(app.handle().clone())));
            app.manage(AppState(backend));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![backend, launch_project])
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                if let Some(state) = window.try_state::<AppState>() {
                    state.0.shutdown();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running NestFactory");
}
