use serde::Serialize;
use tauri::AppHandle;

use crate::data::{paths, session};

#[derive(Debug, Serialize)]
pub(crate) struct ImageRefAbs {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) rel_path: String,
    pub(crate) thumb_rel_path: Option<String>,
    pub(crate) abs_path: String,
    pub(crate) thumb_abs_path: Option<String>,
    pub(crate) mime: String,
    pub(crate) media_role: Option<String>,
    pub(crate) source_url: Option<String>,
    pub(crate) width: Option<i64>,
    pub(crate) height: Option<i64>,
    pub(crate) bytes: Option<i64>,
    pub(crate) ord: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct MessageAbs {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) role: String,
    pub(crate) text: Option<String>,
    pub(crate) params: Option<serde_json::Value>,
    pub(crate) created_at: i64,
    pub(crate) images: Vec<ImageRefAbs>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionWithMessagesAbs {
    pub(crate) session: session::Session,
    pub(crate) messages: Vec<MessageAbs>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent_title: Option<String>,
}

pub(crate) fn decorate_image(app: &AppHandle, i: session::ImageRef) -> ImageRefAbs {
    let abs = if i.rel_path.trim().is_empty() {
        String::new()
    } else {
        paths::abs_from_rel(app, &i.rel_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let thumb_abs = i.thumb_rel_path.as_ref().and_then(|r| {
        paths::abs_from_rel(app, r)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });
    ImageRefAbs {
        id: i.id,
        role: i.role,
        rel_path: i.rel_path,
        thumb_rel_path: i.thumb_rel_path,
        abs_path: abs,
        thumb_abs_path: thumb_abs,
        mime: i.mime,
        media_role: i.media_role,
        source_url: i.source_url,
        width: i.width,
        height: i.height,
        bytes: i.bytes,
        ord: i.ord,
    }
}

pub(crate) fn decorate_message(app: &AppHandle, m: session::Message) -> MessageAbs {
    MessageAbs {
        id: m.id,
        session_id: m.session_id,
        role: m.role,
        text: m.text,
        params: m.params,
        created_at: m.created_at,
        images: m
            .images
            .into_iter()
            .map(|i| decorate_image(app, i))
            .collect(),
    }
}

pub(crate) fn decorate_session(app: &AppHandle, s: session::SessionWithMessages) -> SessionWithMessagesAbs {
    SessionWithMessagesAbs {
        session: s.session,
        messages: s
            .messages
            .into_iter()
            .map(|m| decorate_message(app, m))
            .collect(),
        parent_title: s.parent_title,
    }
}
