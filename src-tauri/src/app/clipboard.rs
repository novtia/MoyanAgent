//! Clipboard helpers.
//!
//! On Windows, `tauri-plugin-clipboard-manager` / arboard open the clipboard with a
//! NULL owner HWND. That still lets Ctrl+V work, but Windows Clipboard History
//! (Win+V) often ignores the update. We re-open with the host window HWND so
//! history captures the text.

use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::error::{AppError, AppResult};

#[tauri::command]
pub fn clipboard_write_text(app: AppHandle, window: WebviewWindow, text: String) -> AppResult<()> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(windows)]
    {
        if write_text_with_window_owner(&window, &text)? {
            return Ok(());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = window;
    }

    app.clipboard()
        .write_text(text)
        .map_err(|e| AppError::Invalid(format!("clipboard write: {e}")))
}

#[cfg(windows)]
fn write_text_with_window_owner(window: &WebviewWindow, text: &str) -> AppResult<bool> {
    use clipboard_win::{formats, Clipboard, Setter};

    let hwnd = match window.hwnd() {
        Ok(h) => h,
        Err(_) => return Ok(false),
    };
    let owner = hwnd.0 as clipboard_win::types::HWND;

    // Retry: another process may briefly hold the clipboard.
    let _clip = match Clipboard::new_attempts_for(owner, 10) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };

    formats::Unicode
        .write_clipboard(&text)
        .map_err(|e| AppError::Invalid(format!("clipboard write: {e}")))?;
    Ok(true)
}
