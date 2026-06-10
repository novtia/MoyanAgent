use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use ulid::Ulid;
use zip::write::SimpleFileOptions;

use crate::data::db::{now_ms, DbPool};
use crate::data::{paths, project, session};
use crate::error::{AppError, AppResult};

const MANIFEST_VERSION: &str = "1";

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: String,
    exported_at: i64,
    app_version: String,
}

/// Flat archive payload persisted in `data/sessions.json`.
/// We use `Session` directly (which is Deserialize).
/// For messages we re-use `session::Message`.

#[derive(Serialize)]
pub struct ImportResult {
    pub projects_imported: usize,
    pub sessions_imported: usize,
    pub messages_imported: usize,
}

/// Export one or more projects (with all their sessions + images) to a `.atelier` zip archive.
pub fn export_projects(
    app: &AppHandle,
    pool: &DbPool,
    project_ids: &[String],
    dest_path: &str,
) -> AppResult<()> {
    let conn = pool.get()?;

    let mut all_projects: Vec<project::Project> = Vec::new();
    let mut all_sessions: Vec<session::Session> = Vec::new();
    // old session_id → messages
    let mut session_messages: HashMap<String, Vec<session::Message>> = HashMap::new();

    let target_ids: std::collections::HashSet<&str> =
        project_ids.iter().map(|s| s.as_str()).collect();

    for pid in project_ids {
        all_projects.push(project::get(&conn, pid)?);
    }

    let sessions_list = session::list(&conn)?;
    for summary in &sessions_list {
        if let Some(ref pid) = summary.project_id {
            if target_ids.contains(pid.as_str()) {
                let loaded = session::load_with_messages(&conn, &summary.id)?;
                session_messages.insert(summary.id.clone(), loaded.messages);
                all_sessions.push(loaded.session);
            }
        }
    }

    let file = fs::File::create(dest_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // manifest.json
    let manifest = Manifest {
        version: MANIFEST_VERSION.to_string(),
        exported_at: now_ms(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    zip.start_file("manifest.json", opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

    // data/projects.json
    zip.start_file("data/projects.json", opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(serde_json::to_string_pretty(&all_projects)?.as_bytes())?;

    // data/sessions.json
    zip.start_file("data/sessions.json", opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(serde_json::to_string_pretty(&all_sessions)?.as_bytes())?;

    // data/messages/{session_id}.json + images/{session_id}/…
    for (session_id, messages) in &session_messages {
        let msg_entry = format!("data/messages/{}.json", session_id);
        zip.start_file(&msg_entry, opts)
            .map_err(|e| AppError::Other(format!("zip: {e}")))?;
        zip.write_all(serde_json::to_string_pretty(messages)?.as_bytes())?;

        for msg in messages {
            for img in &msg.images {
                write_image_to_zip(app, &mut zip, opts, session_id, &img.rel_path)?;
                if let Some(ref thumb) = img.thumb_rel_path {
                    write_image_to_zip(app, &mut zip, opts, session_id, thumb)?;
                }
            }
        }
    }

    zip.finish()
        .map_err(|e| AppError::Other(format!("zip finish: {e}")))?;

    Ok(())
}

fn write_image_to_zip<W: Write + std::io::Seek>(
    app: &AppHandle,
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    session_id: &str,
    rel_path: &str,
) -> AppResult<()> {
    let abs = paths::abs_from_rel(app, rel_path)?;
    if !abs.exists() {
        return Ok(());
    }
    // zip path: "images/{session_id}/{subdir}/{filename}"
    // rel_path: "sessions/{session_id}/{subdir}/{filename}"
    let inner = rel_path
        .strip_prefix(&format!("sessions/{}/", session_id))
        .unwrap_or(rel_path);
    let zip_path = format!("images/{}/{}", session_id, inner);

    zip.start_file(&zip_path, opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(&fs::read(&abs)?)?;
    Ok(())
}

/// Export a single session (without a project wrapper) to a `.atelier` archive.
pub fn export_session(
    app: &AppHandle,
    pool: &DbPool,
    session_id: &str,
    dest_path: &str,
) -> AppResult<()> {
    let conn = pool.get()?;
    let loaded = session::load_with_messages(&conn, session_id)?;

    let file = fs::File::create(dest_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let manifest = Manifest {
        version: MANIFEST_VERSION.to_string(),
        exported_at: now_ms(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    zip.start_file("manifest.json", opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;

    // Empty projects list
    zip.start_file("data/projects.json", opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(b"[]")?;

    // Single session (project_id set to null so it imports standalone)
    let mut sess = loaded.session;
    sess.project_id = None;
    let sessions_json = serde_json::to_string_pretty(&vec![sess])?;
    zip.start_file("data/sessions.json", opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(sessions_json.as_bytes())?;

    let msg_entry = format!("data/messages/{}.json", session_id);
    zip.start_file(&msg_entry, opts)
        .map_err(|e| AppError::Other(format!("zip: {e}")))?;
    zip.write_all(serde_json::to_string_pretty(&loaded.messages)?.as_bytes())?;

    for msg in &loaded.messages {
        for img in &msg.images {
            write_image_to_zip(app, &mut zip, opts, session_id, &img.rel_path)?;
            if let Some(ref thumb) = img.thumb_rel_path {
                write_image_to_zip(app, &mut zip, opts, session_id, thumb)?;
            }
        }
    }

    zip.finish()
        .map_err(|e| AppError::Other(format!("zip finish: {e}")))?;

    Ok(())
}

/// Import from a `.atelier` archive. Creates new projects and sessions with fresh ULIDs.
pub fn import_archive(
    app: &AppHandle,
    pool: &DbPool,
    archive_path: &str,
) -> AppResult<ImportResult> {
    let file = fs::File::open(archive_path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Other(format!("open zip: {e}")))?;

    // Read manifest
    let manifest: Manifest = {
        let mut entry = zip
            .by_name("manifest.json")
            .map_err(|_| AppError::Invalid("archive missing manifest.json".into()))?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s)
            .map_err(|e| AppError::Invalid(format!("manifest parse: {e}")))?
    };

    if manifest.version != MANIFEST_VERSION {
        return Err(AppError::Invalid(format!(
            "unsupported archive version {}",
            manifest.version
        )));
    }

    // Read projects
    let projects: Vec<project::Project> = {
        let mut entry = zip
            .by_name("data/projects.json")
            .map_err(|_| AppError::Invalid("archive missing data/projects.json".into()))?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s)
            .map_err(|e| AppError::Invalid(format!("projects parse: {e}")))?
    };

    // Read sessions
    let sessions: Vec<session::Session> = {
        let mut entry = zip
            .by_name("data/sessions.json")
            .map_err(|_| AppError::Invalid("archive missing data/sessions.json".into()))?;
        let mut s = String::new();
        entry.read_to_string(&mut s)?;
        serde_json::from_str(&s)
            .map_err(|e| AppError::Invalid(format!("sessions parse: {e}")))?
    };

    // Build ID maps (old → new ULID)
    let mut project_id_map: HashMap<String, String> = HashMap::new();
    let mut session_id_map: HashMap<String, String> = HashMap::new();
    for p in &projects {
        project_id_map.insert(p.id.clone(), Ulid::new().to_string());
    }
    for s in &sessions {
        session_id_map.insert(s.id.clone(), Ulid::new().to_string());
    }

    // Read all messages per session (keyed by old session_id)
    let mut session_messages: HashMap<String, Vec<session::Message>> = HashMap::new();
    for old_sid in sessions.iter().map(|s| &s.id) {
        let key = format!("data/messages/{}.json", old_sid);
        if let Ok(mut entry) = zip.by_name(&key) {
            let mut s = String::new();
            let _ = entry.read_to_string(&mut s);
            if let Ok(msgs) = serde_json::from_str::<Vec<session::Message>>(&s) {
                session_messages.insert(old_sid.clone(), msgs);
            }
        }
    }

    // Collect image bytes from zip before we start writing to DB/disk.
    // key: zip_path, value: bytes
    let mut image_bytes: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..zip.len() {
        if let Ok(mut entry) = zip.by_index(i) {
            let name = entry.name().to_string();
            if name.starts_with("images/") {
                let mut buf = Vec::new();
                let _ = entry.read_to_end(&mut buf);
                image_bytes.insert(name, buf);
            }
        }
    }

    let conn = pool.get()?;
    let now = now_ms();

    // Insert projects
    let mut sort_base: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM projects",
            params![],
            |r| r.get(0),
        )
        .unwrap_or(-1);

    for p in &projects {
        let new_id = project_id_map.get(&p.id).unwrap();
        sort_base += 1;
        let params_json = serde_json::to_string(&p.llm_params)?;
        let agent_chain_json = match &p.agent_chain {
            Some(c) if !c.is_empty() => Some(serde_json::to_string(c)?),
            _ => None,
        };
        conn.execute(
            "INSERT INTO projects(id, name, path, sort_order, system_prompt, history_turns, llm_params, context_window, agent_chain, created_at, updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                new_id,
                p.name,
                p.path,
                sort_base,
                p.system_prompt,
                p.history_turns,
                params_json,
                p.context_window,
                agent_chain_json,
                now,
                now,
            ],
        )?;
    }

    // Insert sessions and messages
    let mut messages_imported: usize = 0;

    for s in &sessions {
        let new_sid = session_id_map.get(&s.id).unwrap().clone();
        let new_pid = s
            .project_id
            .as_ref()
            .and_then(|old_pid| project_id_map.get(old_pid))
            .cloned();

        let params_json = serde_json::to_string(&s.llm_params)?;

        conn.execute(
            "INSERT INTO sessions(id, title, model, system_prompt, history_turns, llm_params, context_window, context_window_used, agent_type, project_id, created_at, updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![
                new_sid,
                s.title,
                s.model,
                s.system_prompt,
                s.history_turns,
                params_json,
                s.context_window,
                s.context_window_used,
                s.agent_type,
                new_pid,
                now,
                now,
            ],
        )?;

        if let Some(messages) = session_messages.get(&s.id) {
            for msg in messages {
                let new_mid = Ulid::new().to_string();
                let params_str = msg.params.as_ref().and_then(|v| serde_json::to_string(v).ok());

                conn.execute(
                    "INSERT INTO messages(id, session_id, role, text, params_json, created_at)
                     VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        new_mid,
                        new_sid,
                        msg.role,
                        msg.text,
                        params_str,
                        msg.created_at,
                    ],
                )?;

                for img in &msg.images {
                    let new_iid = Ulid::new().to_string();
                    let new_rel = remap_session_in_path(&img.rel_path, &s.id, &new_sid);
                    let new_thumb_rel = img
                        .thumb_rel_path
                        .as_ref()
                        .map(|p| remap_session_in_path(p, &s.id, &new_sid));

                    conn.execute(
                        "INSERT INTO message_images(id, message_id, session_id, role, rel_path, thumb_path, mime, width, height, bytes, ord, created_at)
                         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                        params![
                            new_iid,
                            new_mid,
                            new_sid,
                            img.role,
                            new_rel,
                            new_thumb_rel,
                            img.mime,
                            img.width,
                            img.height,
                            img.bytes,
                            img.ord,
                            now,
                        ],
                    )?;

                    // Extract image to new session dir
                    extract_image_file(
                        app,
                        &image_bytes,
                        &s.id,
                        &new_sid,
                        &img.rel_path,
                    )?;
                    if let Some(ref thumb_rel) = img.thumb_rel_path {
                        extract_image_file(
                            app,
                            &image_bytes,
                            &s.id,
                            &new_sid,
                            thumb_rel,
                        )?;
                    }
                }

                messages_imported += 1;
            }
        }
    }

    Ok(ImportResult {
        projects_imported: projects.len(),
        sessions_imported: sessions.len(),
        messages_imported,
    })
}

/// Replace `sessions/{old_sid}/` prefix with `sessions/{new_sid}/`.
fn remap_session_in_path(rel_path: &str, old_sid: &str, new_sid: &str) -> String {
    let old_prefix = format!("sessions/{}/", old_sid);
    let new_prefix = format!("sessions/{}/", new_sid);
    if let Some(rest) = rel_path.strip_prefix(&old_prefix) {
        format!("{}{}", new_prefix, rest)
    } else {
        rel_path.to_string()
    }
}

/// Copy an image stored in the zip (`images/{old_sid}/{subdir}/{file}`) to
/// the app's data dir at the remapped path (`sessions/{new_sid}/{subdir}/{file}`).
fn extract_image_file(
    app: &AppHandle,
    image_bytes: &HashMap<String, Vec<u8>>,
    old_sid: &str,
    new_sid: &str,
    rel_path: &str,
) -> AppResult<()> {
    // rel_path: "sessions/{old_sid}/{subdir}/{file}"
    let inner = rel_path
        .strip_prefix(&format!("sessions/{}/", old_sid))
        .unwrap_or(rel_path);
    let zip_key = format!("images/{}/{}", old_sid, inner);

    if let Some(bytes) = image_bytes.get(&zip_key) {
        let new_rel = remap_session_in_path(rel_path, old_sid, new_sid);
        let abs = paths::abs_from_rel(app, &new_rel)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, bytes)?;
    }
    Ok(())
}
