use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::data::paths;
use crate::error::AppError;

#[derive(Debug, Serialize)]
pub(crate) struct AppInfo {
    pub(crate) version: String,
    pub(crate) data_dir: String,
    pub(crate) db_path: String,
    pub(crate) sessions_dir: String,
}

#[tauri::command]
pub fn get_app_info(app: AppHandle) -> Result<AppInfo, AppError> {
    let data_dir = paths::root_dir(&app)?;
    let db_path = paths::db_path(&app)?;
    let sessions_dir = paths::sessions_dir(&app)?;
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        db_path: db_path.to_string_lossy().into_owned(),
        sessions_dir: sessions_dir.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), AppError> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(AppError::NotFound(format!("path does not exist: {path}")));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(&path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&path).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&path).spawn()?;
    }
    Ok(())
}

/// Open an absolute http(s) URL in the user's default browser.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), AppError> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::Invalid("url must be an http(s) URL".into()));
    }
    #[cfg(target_os = "windows")]
    {
        // `explorer <url>` reliably hands off to the default browser.
        std::process::Command::new("explorer").arg(trimmed).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(trimmed).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(trimmed).spawn()?;
    }
    Ok(())
}

#[tauri::command]
pub fn toggle_devtools(app: AppHandle) -> Result<(), AppError> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    #[cfg(any(debug_assertions, feature = "devtools"))]
    {
        #[cfg(not(target_os = "windows"))]
        {
            if window.is_devtools_open() {
                window.close_devtools();
                return Ok(());
            }
        }
        window.open_devtools();
    }
    Ok(())
}
