use rusqlite::params;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::db::{now_ms, DbConn};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub updated_at: i64,
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRef {
    pub id: String,
    pub role: String, // input | output | edited | draft
    pub rel_path: String,
    pub thumb_rel_path: Option<String>,
    pub mime: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: Option<i64>,
    pub ord: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String, // user | assistant | error
    pub text: Option<String>,
    pub params: Option<serde_json::Value>,
    pub created_at: i64,
    pub images: Vec<ImageRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionWithMessages {
    pub session: Session,
    pub messages: Vec<Message>,
}

pub fn create(conn: &DbConn, title: Option<String>, model: Option<String>) -> AppResult<Session> {
    let id = Ulid::new().to_string();
    let now = now_ms();
    let title = title.unwrap_or_else(|| "New session".into());
    conn.execute(
        "INSERT INTO sessions(id, title, model, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?4)",
        params![id, title, model, now],
    )?;
    Ok(Session {
        id,
        title,
        model,
        created_at: now,
        updated_at: now,
    })
}

pub fn rename(conn: &DbConn, id: &str, title: &str) -> AppResult<()> {
    let updated = now_ms();
    let n = conn.execute(
        "UPDATE sessions SET title=?1, updated_at=?2 WHERE id=?3",
        params![title, updated, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("session {id}")));
    }
    Ok(())
}

pub fn delete(conn: &DbConn, id: &str) -> AppResult<()> {
    conn.execute("DELETE FROM sessions WHERE id=?1", params![id])?;
    Ok(())
}

pub fn update_message_text(conn: &DbConn, id: &str, text: &str) -> AppResult<()> {
    let n = conn.execute("UPDATE messages SET text=?1 WHERE id=?2", params![text, id])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("message {id}")));
    }
    Ok(())
}

pub fn update_message_params(conn: &DbConn, id: &str, params_json: &str) -> AppResult<()> {
    let n = conn.execute(
        "UPDATE messages SET params_json=?1 WHERE id=?2",
        params![params_json, id],
    )?;
    if n == 0 {
        return Err(AppError::NotFound(format!("message {id}")));
    }
    Ok(())
}

/// Returns image rel_paths (and thumb rel_paths) that should be cleaned from disk.
pub fn delete_message(conn: &DbConn, id: &str) -> AppResult<Vec<(String, Option<String>)>> {
    let mut stmt =
        conn.prepare("SELECT rel_path, thumb_path FROM message_images WHERE message_id=?1")?;
    let rows = stmt.query_map(params![id], |r| {
        let rel: String = r.get(0)?;
        let thumb: Option<String> = r.get(1)?;
        Ok((rel, thumb))
    })?;
    let mut paths = Vec::new();
    for r in rows {
        paths.push(r?);
    }
    conn.execute(
        "DELETE FROM message_images WHERE message_id=?1",
        params![id],
    )?;
    let n = conn.execute("DELETE FROM messages WHERE id=?1", params![id])?;
    if n == 0 {
        return Err(AppError::NotFound(format!("message {id}")));
    }
    Ok(paths)
}

pub fn list(conn: &DbConn) -> AppResult<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT s.id, s.title, s.model, s.updated_at,
            (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id) AS cnt
         FROM sessions s
         ORDER BY s.updated_at DESC",
    )?;
    let rows = stmt.query_map(params![], |r| {
        Ok(SessionSummary {
            id: r.get(0)?,
            title: r.get(1)?,
            model: r.get(2)?,
            updated_at: r.get(3)?,
            message_count: r.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get(conn: &DbConn, id: &str) -> AppResult<Session> {
    let mut stmt =
        conn.prepare("SELECT id, title, model, created_at, updated_at FROM sessions WHERE id=?1")?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Session {
            id: row.get(0)?,
            title: row.get(1)?,
            model: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    } else {
        Err(AppError::NotFound(format!("session {id}")))
    }
}

pub fn touch(conn: &DbConn, id: &str) -> AppResult<()> {
    conn.execute(
        "UPDATE sessions SET updated_at=?1 WHERE id=?2",
        params![now_ms(), id],
    )?;
    Ok(())
}

pub fn insert_message(
    conn: &DbConn,
    session_id: &str,
    role: &str,
    text: Option<&str>,
    params_json: Option<&str>,
) -> AppResult<Message> {
    let id = Ulid::new().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO messages(id, session_id, role, text, params_json, created_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![id, session_id, role, text, params_json, now],
    )?;
    touch(conn, session_id)?;
    let params_v: Option<serde_json::Value> = match params_json {
        Some(s) => serde_json::from_str(s).ok(),
        None => None,
    };
    Ok(Message {
        id,
        session_id: session_id.into(),
        role: role.into(),
        text: text.map(|s| s.to_string()),
        params: params_v,
        created_at: now,
        images: vec![],
    })
}

pub fn insert_image(
    conn: &DbConn,
    session_id: &str,
    message_id: Option<&str>,
    role: &str,
    rel_path: &str,
    thumb_rel_path: Option<&str>,
    mime: &str,
    width: Option<u32>,
    height: Option<u32>,
    bytes: Option<u64>,
    ord: i64,
) -> AppResult<ImageRef> {
    let id = Ulid::new().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO message_images(id, message_id, session_id, role, rel_path, thumb_path, mime, width, height, bytes, ord, created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            id,
            message_id,
            session_id,
            role,
            rel_path,
            thumb_rel_path,
            mime,
            width.map(|v| v as i64),
            height.map(|v| v as i64),
            bytes.map(|v| v as i64),
            ord,
            now
        ],
    )?;
    Ok(ImageRef {
        id,
        role: role.into(),
        rel_path: rel_path.into(),
        thumb_rel_path: thumb_rel_path.map(|s| s.to_string()),
        mime: mime.into(),
        width: width.map(|v| v as i64),
        height: height.map(|v| v as i64),
        bytes: bytes.map(|v| v as i64),
        ord,
    })
}

pub fn bind_images_to_message(
    conn: &DbConn,
    message_id: &str,
    image_ids: &[String],
) -> AppResult<()> {
    for id in image_ids {
        conn.execute(
            "UPDATE message_images SET message_id=?1 WHERE id=?2",
            params![message_id, id],
        )?;
    }
    Ok(())
}

/// Replace the set of `input` images on a message with the given ordered list of image ids.
/// Each id must already exist in `message_images` and must either be unbound (a draft)
/// or already bound to this message; all must be in the same session.
/// Returns (rel_path, thumb_path) pairs for images that were removed and should be cleaned from disk.
pub fn update_message_input_images(
    conn: &DbConn,
    message_id: &str,
    new_image_ids: &[String],
) -> AppResult<Vec<(String, Option<String>)>> {
    let session_id: String = match conn.query_row(
        "SELECT session_id FROM messages WHERE id=?1",
        params![message_id],
        |r| r.get(0),
    ) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(AppError::NotFound(format!("message {message_id}")));
        }
        Err(e) => return Err(e.into()),
    };

    let mut current: Vec<(String, String, Option<String>)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, rel_path, thumb_path FROM message_images
             WHERE message_id=?1 AND role='input'",
        )?;
        let rows = stmt.query_map(params![message_id], |r| {
            let id: String = r.get(0)?;
            let rel: String = r.get(1)?;
            let thumb: Option<String> = r.get(2)?;
            Ok((id, rel, thumb))
        })?;
        for r in rows {
            current.push(r?);
        }
    }

    let new_set: std::collections::HashSet<&str> =
        new_image_ids.iter().map(|s| s.as_str()).collect();

    for image_id in new_image_ids {
        let row: Result<(String, Option<String>, String), rusqlite::Error> = conn.query_row(
            "SELECT session_id, message_id, role FROM message_images WHERE id=?1",
            params![image_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        );
        match row {
            Ok((sid, mid, role)) => {
                if sid != session_id {
                    return Err(AppError::Invalid(format!(
                        "image {image_id} not in session"
                    )));
                }
                match mid {
                    None => {}
                    Some(ref m) if m == message_id => {}
                    Some(_) => {
                        return Err(AppError::Invalid(format!(
                            "image {image_id} bound to another message"
                        )));
                    }
                }
                if role != "input" {
                    return Err(AppError::Invalid(format!(
                        "image {image_id} is not an input image"
                    )));
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(AppError::NotFound(format!("image {image_id}")));
            }
            Err(e) => return Err(e.into()),
        }
    }

    let mut removed: Vec<(String, Option<String>)> = Vec::new();
    for (id, rel, thumb) in &current {
        if !new_set.contains(id.as_str()) {
            conn.execute("DELETE FROM message_images WHERE id=?1", params![id])?;
            removed.push((rel.clone(), thumb.clone()));
        }
    }

    for (i, image_id) in new_image_ids.iter().enumerate() {
        conn.execute(
            "UPDATE message_images SET message_id=?1, ord=?2, role='input' WHERE id=?3",
            params![message_id, i as i64, image_id],
        )?;
    }

    touch(conn, &session_id)?;
    Ok(removed)
}

pub fn get_image(conn: &DbConn, id: &str) -> AppResult<ImageRef> {
    let mut stmt = conn.prepare(
        "SELECT id, role, rel_path, thumb_path, mime, width, height, bytes, ord
         FROM message_images WHERE id=?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(r) = rows.next()? {
        Ok(ImageRef {
            id: r.get(0)?,
            role: r.get(1)?,
            rel_path: r.get(2)?,
            thumb_rel_path: r.get(3)?,
            mime: r.get(4)?,
            width: r.get(5)?,
            height: r.get(6)?,
            bytes: r.get(7)?,
            ord: r.get(8)?,
        })
    } else {
        Err(AppError::NotFound(format!("image {id}")))
    }
}

pub fn image_session_id(conn: &DbConn, id: &str) -> AppResult<String> {
    let mut stmt = conn.prepare("SELECT session_id FROM message_images WHERE id=?1")?;
    let mut rows = stmt.query(params![id])?;
    if let Some(r) = rows.next()? {
        Ok(r.get(0)?)
    } else {
        Err(AppError::NotFound(format!("image {id}")))
    }
}

fn load_message_images(conn: &DbConn, message_id: &str) -> AppResult<Vec<ImageRef>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, rel_path, thumb_path, mime, width, height, bytes, ord
         FROM message_images WHERE message_id=?1 ORDER BY ord ASC",
    )?;
    let rows = stmt.query_map(params![message_id], |r| {
        Ok(ImageRef {
            id: r.get(0)?,
            role: r.get(1)?,
            rel_path: r.get(2)?,
            thumb_rel_path: r.get(3)?,
            mime: r.get(4)?,
            width: r.get(5)?,
            height: r.get(6)?,
            bytes: r.get(7)?,
            ord: r.get(8)?,
        })
    })?;
    let mut v = Vec::new();
    for r in rows {
        v.push(r?);
    }
    Ok(v)
}

pub fn load_with_messages(conn: &DbConn, session_id: &str) -> AppResult<SessionWithMessages> {
    let session = get(conn, session_id)?;
    let mut stmt = conn.prepare(
        "SELECT id, role, text, params_json, created_at FROM messages
         WHERE session_id=?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![session_id], |r| {
        let params_str: Option<String> = r.get(3)?;
        Ok(Message {
            id: r.get(0)?,
            session_id: session_id.into(),
            role: r.get(1)?,
            text: r.get(2)?,
            params: params_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: r.get(4)?,
            images: vec![],
        })
    })?;
    let mut messages = Vec::new();
    for r in rows {
        let mut m = r?;
        m.images = load_message_images(conn, &m.id)?;
        messages.push(m);
    }
    Ok(SessionWithMessages { session, messages })
}
