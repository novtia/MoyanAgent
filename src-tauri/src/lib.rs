mod ai;
mod app;
mod data;
mod error;
mod media;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    app::run();
}
