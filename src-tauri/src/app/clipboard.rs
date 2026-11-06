//! Clipboard helpers.
//!
//! On Windows, `tauri-plugin-clipboard-manager` / arboard open the clipboard with a
//! NULL owner HWND. That still lets Ctrl+V work, but Windows Clipboard History
//! (Win+V) often ignores the update. We re-open with the host window HWND so
//! history captures the text.
//!
//! WebView2 commits the copy event's data, then Win+V opens the clipboard to
//! snapshot it. Those holders stick around for tens of milliseconds.
//! `Sleep(0)` retries finish before the lock is released, and the arboard
//! fallback then fails with "held by another party". Wait and try again.

use tauri::{AppHandle, WebviewWindow};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::error::{AppError, AppResult};

#[cfg(windows)]
const OPEN_ATTEMPTS: u32 = 24;
#[cfg(windows)]
const OPEN_WAIT: std::time::Duration = std::time::Duration::from_millis(25);

#[tauri::command]
pub async fn clipboard_write_text(
    app: AppHandle,
    window: WebviewWindow,
    text: String,
) -> AppResult<()> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(windows)]
    {
        if write_text_owned(&window, &text).await? {
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

/// Write plain text and HTML to the clipboard in one open/close, like native
/// Windows apps do: external apps and Win+V see the text, while in-app paste
/// can read the HTML (which carries structured metadata).
#[tauri::command]
pub async fn clipboard_write_rich(
    app: AppHandle,
    window: WebviewWindow,
    text: String,
    html: String,
) -> AppResult<()> {
    if text.is_empty() {
        return Ok(());
    }

    #[cfg(windows)]
    {
        if write_rich_owned(&window, &text, &html).await? {
            return Ok(());
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (&window, &html);
    }

    app.clipboard()
        .write_text(text)
        .map_err(|e| AppError::Invalid(format!("clipboard write: {e}")))
}

#[cfg(windows)]
enum WriteAttempt {
    Busy,
    Failed(AppError),
}

#[cfg(windows)]
fn window_owner(window: &WebviewWindow) -> Option<clipboard_win::types::HWND> {
    let hwnd = window.hwnd().ok()?;
    if hwnd.0.is_null() {
        return None;
    }
    Some(hwnd.0 as clipboard_win::types::HWND)
}

/// Open with the host HWND, run `write` while that open is held, and retry when
/// another party still owns the clipboard. `Ok(false)` means there is no window
/// to own the clipboard, or it stayed busy for the whole wait — the caller
/// falls back to arboard.
#[cfg(windows)]
async fn with_owned_clipboard(
    window: &WebviewWindow,
    mut write: impl FnMut() -> Result<(), AppError>,
) -> AppResult<bool> {
    for attempt in 0..OPEN_ATTEMPTS {
        // The raw HWND is `*mut c_void` and must not live across `.await`,
        // or the command future is no longer `Send`.
        let attempt_result = {
            let Some(owner) = window_owner(window) else {
                return Ok(false);
            };
            open_and_write(owner, &mut write)
        };
        match attempt_result {
            Ok(()) => return Ok(true),
            Err(WriteAttempt::Failed(err)) => return Err(err),
            Err(WriteAttempt::Busy) if attempt + 1 < OPEN_ATTEMPTS => {
                tokio::time::sleep(OPEN_WAIT).await;
            }
            Err(WriteAttempt::Busy) => return Ok(false),
        }
    }
    Ok(false)
}

#[cfg(windows)]
fn open_and_write(
    owner: clipboard_win::types::HWND,
    write: &mut impl FnMut() -> Result<(), AppError>,
) -> Result<(), WriteAttempt> {
    use clipboard_win::Clipboard;

    // Drop closes the clipboard before the caller waits to retry.
    let _clip = Clipboard::new_for(owner).map_err(|_| WriteAttempt::Busy)?;
    write().map_err(WriteAttempt::Failed)
}

#[cfg(windows)]
async fn write_text_owned(window: &WebviewWindow, text: &str) -> AppResult<bool> {
    use clipboard_win::{formats, Setter};

    with_owned_clipboard(window, || {
        formats::Unicode
            .write_clipboard(&text)
            .map_err(|e| AppError::Invalid(format!("clipboard write: {e}")))
    })
    .await
}

#[cfg(windows)]
async fn write_rich_owned(window: &WebviewWindow, text: &str, html: &str) -> AppResult<bool> {
    use clipboard_win::{formats, Setter};

    with_owned_clipboard(window, || {
        // Unicode text first: this empties the clipboard, later formats are added.
        formats::Unicode
            .write_clipboard(&text)
            .map_err(|e| AppError::Invalid(format!("clipboard write: {e}")))?;
        if !html.is_empty() {
            if let Some(fmt) = formats::Html::new() {
                // Best effort: plain text is already on the clipboard.
                let _ = fmt.write_clipboard(&html);
            }
        }
        Ok(())
    })
    .await
}
