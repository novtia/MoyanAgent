use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use tauri::AppHandle;

use crate::data::backup::{self, BackupKind, BackupModule};
use crate::error::AppError;

use super::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupArgs {
    pub module: String,
    #[serde(default)]
    pub dest_path: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBackupArgs {
    pub archive_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListBackupsArgs {
    #[serde(default)]
    pub module: Option<String>,
}

#[tauri::command]
pub async fn create_backup(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    args: CreateBackupArgs,
) -> Result<backup::BackupResult, AppError> {
    let module = BackupModule::parse(&args.module)?;
    let kind = match args.kind.as_deref().unwrap_or("manual") {
        "auto" => BackupKind::Auto,
        _ => BackupKind::Manual,
    };
    let pool = state.pool.clone();
    let dest_path = args.dest_path.clone();
    let app_job = app.clone();
    let result = tokio::task::spawn_blocking(move || {
        backup::create_backup(&app_job, &pool, module, kind, dest_path.as_deref())
    })
    .await
    .map_err(|e| AppError::Other(format!("backup task join failed: {e}")))??;

    if module == BackupModule::Full && kind == BackupKind::Manual {
        let pool2 = state.pool.clone();
        let created_at = result.created_at;
        let _ = tokio::task::spawn_blocking(move || {
            backup::record_full_success(&app, &pool2, created_at)
        })
        .await;
    }
    Ok(result)
}

#[tauri::command]
pub async fn restore_backup(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    args: RestoreBackupArgs,
) -> Result<backup::RestoreResult, AppError> {
    let pool = state.pool.clone();
    let archive_path = args.archive_path.clone();
    tokio::task::spawn_blocking(move || backup::restore_backup(&app, &pool, &archive_path))
        .await
        .map_err(|e| AppError::Other(format!("restore task join failed: {e}")))?
}

#[tauri::command]
pub fn list_backups(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: Option<ListBackupsArgs>,
) -> Result<Vec<backup::BackupListItem>, AppError> {
    let module = args
        .and_then(|a| a.module)
        .as_deref()
        .map(BackupModule::parse)
        .transpose()?;
    backup::list_backups(&app, &state.pool, module)
}

#[tauri::command]
pub fn get_backup_status(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
) -> Result<backup::BackupStatus, AppError> {
    backup::get_status(&app, &state.pool)
}

/// Spawn the in-process auto-backup scheduler (1-minute tick + startup catch-up).
pub fn spawn_scheduler(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // Brief delay so the UI can finish first paint before catch-up work.
        tokio::time::sleep(Duration::from_secs(3)).await;
        loop {
            let app2 = app.clone();
            let pool = state.pool.clone();
            let result = tokio::task::spawn_blocking(move || backup::run_scheduler_tick(&app2, &pool))
                .await;
            match result {
                Ok(Ok(n)) if n > 0 => {
                    eprintln!("[backup] auto-backup completed ({n} module(s))");
                }
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    eprintln!("[backup] scheduler tick failed: {e}");
                }
                Err(e) => {
                    eprintln!("[backup] scheduler join failed: {e}");
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
