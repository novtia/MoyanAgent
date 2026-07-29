use serde::Deserialize;
use std::sync::Arc;
use tauri::AppHandle;

use crate::data::{settings, skills};
use crate::error::AppResult;

use super::state::AppState;

#[tauri::command]
pub fn list_skills(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
) -> AppResult<Vec<skills::SkillInfo>> {
    let conn = state.conn()?;
    let s = settings::read(&conn)?;
    skills::list_skills(&app, &s.enabled_skill_ids)
}

#[tauri::command]
pub fn get_skill(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> AppResult<skills::SkillInfo> {
    let conn = state.conn()?;
    let s = settings::read(&conn)?;
    skills::get_skill(&app, &id, &s.enabled_skill_ids)
}

#[derive(Debug, Deserialize)]
pub struct SetSkillEnabledArgs {
    pub id: String,
    pub enabled: bool,
}

#[tauri::command]
pub fn set_skill_enabled(
    state: tauri::State<'_, Arc<AppState>>,
    args: SetSkillEnabledArgs,
) -> AppResult<settings::Settings> {
    let conn = state.conn()?;
    let mut s = settings::read(&conn)?;
    let id = args.id.trim().to_string();
    if id.is_empty() {
        return Err(crate::error::AppError::Invalid("empty skill id".into()));
    }
    if args.enabled {
        if !s.enabled_skill_ids.iter().any(|x| x == &id) {
            s.enabled_skill_ids.push(id);
        }
    } else {
        s.enabled_skill_ids.retain(|x| x != &id);
    }
    settings::apply_patch(
        &conn,
        settings::SettingsPatch {
            enabled_skill_ids: Some(s.enabled_skill_ids.clone()),
            ..Default::default()
        },
    )
}

#[derive(Debug, Deserialize)]
pub struct ImportSkillArgs {
    pub path: String,
}

#[tauri::command]
pub fn import_skill(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    args: ImportSkillArgs,
) -> AppResult<skills::SkillInfo> {
    let path = std::path::PathBuf::from(args.path.trim());
    let info = skills::import_skill_file(&app, &path)?;
    // Auto-enable newly imported skills.
    let conn = state.conn()?;
    let mut s = settings::read(&conn)?;
    if !s.enabled_skill_ids.iter().any(|x| x == &info.id) {
        s.enabled_skill_ids.push(info.id.clone());
        let _ = settings::apply_patch(
            &conn,
            settings::SettingsPatch {
                enabled_skill_ids: Some(s.enabled_skill_ids),
                ..Default::default()
            },
        )?;
    }
    let s = settings::read(&conn)?;
    skills::get_skill(&app, &info.id, &s.enabled_skill_ids)
}

#[tauri::command]
pub fn uninstall_skill(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> AppResult<()> {
    skills::uninstall_skill(&app, &id)?;
    let conn = state.conn()?;
    let mut s = settings::read(&conn)?;
    s.enabled_skill_ids.retain(|x| x != &id);
    let _ = settings::apply_patch(
        &conn,
        settings::SettingsPatch {
            enabled_skill_ids: Some(s.enabled_skill_ids),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[tauri::command]
pub fn get_skills_dir(app: AppHandle) -> AppResult<String> {
    skills::skills_dir_path(&app)
}

#[tauri::command]
pub fn list_enabled_skills(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
) -> AppResult<Vec<skills::SkillInfo>> {
    let all = list_skills(state, app)?;
    Ok(all.into_iter().filter(|s| s.enabled).collect())
}
