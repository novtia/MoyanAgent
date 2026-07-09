use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use image::{imageops::FilterType, DynamicImage, ImageFormat};
use serde::Serialize;
use tauri::AppHandle;
use ulid::Ulid;

use crate::data::db::DbConn;
use crate::data::{paths, session};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
pub struct AttachmentDraft {
    pub image_id: String,
    pub rel_path: String,
    pub thumb_rel_path: Option<String>,
    pub abs_path: String,
    pub thumb_abs_path: Option<String>,
    pub mime: String,
    pub media_role: Option<String>,
    pub source_url: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bytes: Option<i64>,
}

const THUMB_SIZE: u32 = 256;
const ALLOWED: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/bmp",
    "image/gif",
    "image/tiff",
    "audio/wav",
    "audio/mpeg",
    "video/mp4",
    "video/quicktime",
];

fn ext_from_mime(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/gif" => "gif",
        "image/tiff" => "tiff",
        "audio/wav" => "wav",
        "audio/mpeg" => "mp3",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        _ => "png",
    }
}

fn ext_from_path(p: &Path) -> Option<String> {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
}

fn detect_mime_from_path(p: &Path) -> AppResult<String> {
    let mut f = File::open(p)?;
    let mut header = [0u8; 12];
    let n = f.read(&mut header)?;
    let hint_ext = ext_from_path(p);
    Ok(detect_mime(&header[..n], hint_ext.as_deref()))
}

pub fn detect_mime(bytes: &[u8], hint_ext: Option<&str>) -> String {
    if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        return "image/png".into();
    }
    if bytes.len() >= 3 && &bytes[..3] == b"\xff\xd8\xff" {
        return "image/jpeg".into();
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".into();
    }
    if bytes.len() >= 2 && &bytes[..2] == b"BM" {
        return "image/bmp".into();
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return "image/gif".into();
    }
    if bytes.len() >= 4 && (&bytes[..4] == b"II*\0" || &bytes[..4] == b"MM\0*") {
        return "image/tiff".into();
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return "audio/wav".into();
    }
    if bytes.len() >= 3 && &bytes[..3] == b"ID3" {
        return "audio/mpeg".into();
    }
    if bytes.len() >= 2 && bytes[0] == 0xff && (bytes[1] & 0xe0) == 0xe0 {
        return "audio/mpeg".into();
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return if matches!(hint_ext, Some("mov")) {
            "video/quicktime".into()
        } else {
            "video/mp4".into()
        };
    }
    match hint_ext.unwrap_or("") {
        "jpg" | "jpeg" => "image/jpeg".into(),
        "webp" => "image/webp".into(),
        "png" => "image/png".into(),
        "bmp" => "image/bmp".into(),
        "gif" => "image/gif".into(),
        "tif" | "tiff" => "image/tiff".into(),
        "wav" => "audio/wav".into(),
        "mp3" => "audio/mpeg".into(),
        "mp4" => "video/mp4".into(),
        "mov" => "video/quicktime".into(),
        _ => "application/octet-stream".into(),
    }
}

fn default_media_role(mime: &str) -> Option<&'static str> {
    if mime.starts_with("audio/") {
        Some("reference_audio")
    } else if mime.starts_with("video/") {
        Some("reference_video")
    } else {
        None
    }
}

fn validate_attachment_size(mime: &str, bytes: u64) -> AppResult<()> {
    let max = if mime.starts_with("image/") {
        30 * 1024 * 1024
    } else if mime.starts_with("audio/") {
        15 * 1024 * 1024
    } else if mime.starts_with("video/") {
        200 * 1024 * 1024
    } else {
        0
    };
    if max == 0 || bytes > max {
        return Err(AppError::Invalid(format!(
            "附件 {} 大小不符合限制（{} 字节）",
            mime, bytes
        )));
    }
    Ok(())
}

fn make_thumb(src_bytes: &[u8], dest: &Path) -> AppResult<()> {
    let img = image::load_from_memory(src_bytes)?;
    make_thumb_from_image(&img, dest)
}

fn make_thumb_from_image(img: &DynamicImage, dest: &Path) -> AppResult<()> {
    let thumb = img.resize(THUMB_SIZE, THUMB_SIZE, FilterType::Triangle);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    thumb.save_with_format(dest, ImageFormat::Png)?;
    Ok(())
}

fn link_or_copy(src: &Path, dest: &Path) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::hard_link(src, dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dest)?;
            Ok(())
        }
    }
}

fn to_abs_string(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

pub fn save_bytes_as_attachment(
    app: &AppHandle,
    conn: &DbConn,
    session_id: &str,
    name_hint: Option<&str>,
    bytes: &[u8],
) -> AppResult<AttachmentDraft> {
    let hint_ext: Option<String> = name_hint.and_then(|n| ext_from_path(Path::new(n)));
    let mime = detect_mime(bytes, hint_ext.as_deref());
    if !ALLOWED.contains(&mime.as_str()) {
        return Err(AppError::Invalid(format!("unsupported mime: {mime}")));
    }
    validate_attachment_size(&mime, bytes.len() as u64)?;

    let session_dir = paths::session_dir(app, session_id)?;
    let id = Ulid::new().to_string();
    let ext = ext_from_mime(&mime);
    let abs_path: PathBuf = session_dir.join("in").join(format!("{id}.{ext}"));
    std::fs::write(&abs_path, bytes)?;

    let (w, h, thumb_exists, thumb_abs) = match image::load_from_memory(bytes) {
        Ok(img) => {
            let thumb_abs = session_dir.join("thumb").join(format!("{id}.png"));
            let _ = make_thumb_from_image(&img, &thumb_abs);
            (
                Some(img.width()),
                Some(img.height()),
                thumb_abs.exists(),
                thumb_abs,
            )
        }
        Err(_) => {
            let thumb_abs = session_dir.join("thumb").join(format!("{id}.png"));
            (None, None, false, thumb_abs)
        }
    };

    let rel = paths::rel_to_root(app, &abs_path)?;
    let thumb_rel = if thumb_exists {
        Some(paths::rel_to_root(app, &thumb_abs)?)
    } else {
        None
    };
    let media_role = default_media_role(&mime);
    let img_ref = session::insert_media(
        conn,
        session_id,
        None,
        "input",
        &rel,
        thumb_rel.as_deref(),
        &mime,
        media_role,
        None,
        w,
        h,
        Some(bytes.len() as u64),
        0,
    )?;
    Ok(AttachmentDraft {
        image_id: img_ref.id,
        rel_path: rel,
        thumb_rel_path: thumb_rel,
        abs_path: to_abs_string(&abs_path),
        thumb_abs_path: if thumb_exists {
            Some(to_abs_string(&thumb_abs))
        } else {
            None
        },
        mime,
        media_role: media_role.map(str::to_string),
        source_url: None,
        width: w.map(|v| v as i64),
        height: h.map(|v| v as i64),
        bytes: Some(bytes.len() as i64),
    })
}

pub fn save_path_as_attachment(
    app: &AppHandle,
    conn: &DbConn,
    session_id: &str,
    src_path: &Path,
) -> AppResult<AttachmentDraft> {
    let mime = detect_mime_from_path(src_path)?;
    if !ALLOWED.contains(&mime.as_str()) {
        return Err(AppError::Invalid(format!("unsupported mime: {mime}")));
    }
    let source_bytes = std::fs::metadata(src_path)?.len();
    validate_attachment_size(&mime, source_bytes)?;

    let session_dir = paths::session_dir(app, session_id)?;
    let id = Ulid::new().to_string();
    let ext = ext_from_mime(&mime);
    let abs_path: PathBuf = session_dir.join("in").join(format!("{id}.{ext}"));
    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src_path, &abs_path)?;

    let thumb_abs = session_dir.join("thumb").join(format!("{id}.png"));
    let (w, h) = match image::open(src_path) {
        Ok(img) => {
            let _ = make_thumb_from_image(&img, &thumb_abs);
            (Some(img.width()), Some(img.height()))
        }
        Err(_) => (None, None),
    };
    let thumb_exists = thumb_abs.exists();
    let bytes = std::fs::metadata(&abs_path).ok().map(|m| m.len() as i64);

    let rel = paths::rel_to_root(app, &abs_path)?;
    let thumb_rel = if thumb_exists {
        Some(paths::rel_to_root(app, &thumb_abs)?)
    } else {
        None
    };
    let media_role = default_media_role(&mime);
    let img_ref = session::insert_media(
        conn,
        session_id,
        None,
        "input",
        &rel,
        thumb_rel.as_deref(),
        &mime,
        media_role,
        None,
        w,
        h,
        bytes.map(|v| v as u64),
        0,
    )?;
    Ok(AttachmentDraft {
        image_id: img_ref.id,
        rel_path: rel,
        thumb_rel_path: thumb_rel,
        abs_path: to_abs_string(&abs_path),
        thumb_abs_path: if thumb_exists {
            Some(to_abs_string(&thumb_abs))
        } else {
            None
        },
        mime,
        media_role: media_role.map(str::to_string),
        source_url: None,
        width: w.map(|v| v as i64),
        height: h.map(|v| v as i64),
        bytes,
    })
}

pub fn save_url_as_attachment(
    conn: &DbConn,
    session_id: &str,
    url: &str,
) -> AppResult<AttachmentDraft> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("asset://"))
    {
        return Err(AppError::Invalid(
            "参考视频 URL 必须使用 http://、https:// 或 asset://".into(),
        ));
    }
    let media = session::insert_media(
        conn,
        session_id,
        None,
        "input",
        "",
        None,
        "video/mp4",
        Some("reference_video"),
        Some(url),
        None,
        None,
        None,
        0,
    )?;
    Ok(AttachmentDraft {
        image_id: media.id,
        rel_path: String::new(),
        thumb_rel_path: None,
        abs_path: String::new(),
        thumb_abs_path: None,
        mime: "video/mp4".into(),
        media_role: Some("reference_video".into()),
        source_url: Some(url.to_string()),
        width: None,
        height: None,
        bytes: None,
    })
}

pub fn write_output_image(
    app: &AppHandle,
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
    bytes: &[u8],
    mime: &str,
    ord: i64,
) -> AppResult<session::ImageRef> {
    let session_dir = paths::session_dir(app, session_id)?;
    let id = Ulid::new().to_string();
    let ext = ext_from_mime(mime);
    let abs = session_dir.join("out").join(format!("{id}.{ext}"));
    std::fs::write(&abs, bytes)?;
    let thumb_abs = session_dir.join("thumb").join(format!("{id}.png"));
    let _ = make_thumb(bytes, &thumb_abs);
    let thumb_exists = thumb_abs.exists();
    let rel = paths::rel_to_root(app, &abs)?;
    let thumb_rel = if thumb_exists {
        Some(paths::rel_to_root(app, &thumb_abs)?)
    } else {
        None
    };
    let (w, h) = image::load_from_memory(bytes)
        .ok()
        .map(|i| (Some(i.width()), Some(i.height())))
        .unwrap_or((None, None));
    session::insert_image(
        conn,
        session_id,
        Some(message_id),
        "output",
        &rel,
        thumb_rel.as_deref(),
        mime,
        w,
        h,
        Some(bytes.len() as u64),
        ord,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn write_output_video(
    app: &AppHandle,
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
    bytes: &[u8],
    mime: &str,
    width: Option<u32>,
    height: Option<u32>,
    ord: i64,
) -> AppResult<session::ImageRef> {
    if !mime.starts_with("video/") {
        return Err(AppError::Invalid(format!("视频输出 MIME 无效：{mime}")));
    }
    let session_dir = paths::session_dir(app, session_id)?;
    let id = Ulid::new().to_string();
    let ext = ext_from_mime(mime);
    let abs = session_dir.join("out").join(format!("{id}.{ext}"));
    std::fs::write(&abs, bytes)?;
    let rel = paths::rel_to_root(app, &abs)?;
    session::insert_media(
        conn,
        session_id,
        Some(message_id),
        "output",
        &rel,
        None,
        mime,
        None,
        None,
        width,
        height,
        Some(bytes.len() as u64),
        ord,
    )
}

pub fn write_edited_image(
    app: &AppHandle,
    conn: &DbConn,
    session_id: &str,
    bytes: &[u8],
    mime: &str,
) -> AppResult<session::ImageRef> {
    let session_dir = paths::session_dir(app, session_id)?;
    let id = Ulid::new().to_string();
    let ext = ext_from_mime(mime);
    let abs = session_dir.join("edit").join(format!("{id}.{ext}"));
    std::fs::write(&abs, bytes)?;
    let thumb_abs = session_dir.join("thumb").join(format!("{id}.png"));
    let _ = make_thumb(bytes, &thumb_abs);
    let thumb_exists = thumb_abs.exists();
    let rel = paths::rel_to_root(app, &abs)?;
    let thumb_rel = if thumb_exists {
        Some(paths::rel_to_root(app, &thumb_abs)?)
    } else {
        None
    };
    let (w, h) = image::load_from_memory(bytes)
        .ok()
        .map(|i| (Some(i.width()), Some(i.height())))
        .unwrap_or((None, None));
    session::insert_image(
        conn,
        session_id,
        None,
        "edited",
        &rel,
        thumb_rel.as_deref(),
        mime,
        w,
        h,
        Some(bytes.len() as u64),
        0,
    )
}

pub fn read_image_bytes(app: &AppHandle, image: &session::ImageRef) -> AppResult<Vec<u8>> {
    if image.rel_path.trim().is_empty() {
        return Ok(Vec::new());
    }
    let abs = paths::abs_from_rel(app, &image.rel_path)?;
    Ok(std::fs::read(&abs)?)
}

/// Copy an existing persisted image file into a new draft row (`message_id` = NULL).
pub fn clone_image_as_draft(
    app: &AppHandle,
    conn: &DbConn,
    session_id: &str,
    image_id: &str,
) -> AppResult<AttachmentDraft> {
    let sid = session::image_session_id(conn, image_id)?;
    if sid != session_id {
        return Err(AppError::Invalid("image does not belong to session".into()));
    }
    let img = session::get_image(conn, image_id)?;
    if !matches!(img.role.as_str(), "input" | "output" | "edited") {
        return Err(AppError::Invalid("cannot quote this image role".into()));
    }
    let session_dir = paths::session_dir(app, session_id)?;
    let id = Ulid::new().to_string();
    let ext = ext_from_mime(&img.mime);
    let abs_path = if img.rel_path.trim().is_empty() {
        PathBuf::new()
    } else {
        let src_abs = paths::abs_from_rel(app, &img.rel_path)?;
        let abs_path = session_dir.join("in").join(format!("{id}.{ext}"));
        link_or_copy(&src_abs, &abs_path)?;
        abs_path
    };

    let thumb_abs = session_dir.join("thumb").join(format!("{id}.png"));
    let mut thumb_rel = None;
    let mut thumb_abs_path = None;
    if let Some(existing_thumb) = &img.thumb_rel_path {
        if let Ok(src_thumb) = paths::abs_from_rel(app, existing_thumb) {
            if link_or_copy(&src_thumb, &thumb_abs).is_ok() {
                thumb_rel = Some(paths::rel_to_root(app, &thumb_abs)?);
                thumb_abs_path = Some(to_abs_string(&thumb_abs));
            }
        }
    }

    let rel = if abs_path.as_os_str().is_empty() {
        String::new()
    } else {
        paths::rel_to_root(app, &abs_path)?
    };
    let img_ref = session::insert_media(
        conn,
        session_id,
        None,
        "input",
        &rel,
        thumb_rel.as_deref(),
        &img.mime,
        img.media_role.as_deref(),
        img.source_url.as_deref(),
        img.width.map(|v| v as u32),
        img.height.map(|v| v as u32),
        img.bytes.map(|v| v as u64),
        0,
    )?;
    Ok(AttachmentDraft {
        image_id: img_ref.id,
        rel_path: rel,
        thumb_rel_path: thumb_rel,
        abs_path: if abs_path.as_os_str().is_empty() {
            String::new()
        } else {
            to_abs_string(&abs_path)
        },
        thumb_abs_path,
        mime: img.mime,
        media_role: img.media_role,
        source_url: img.source_url,
        width: img.width,
        height: img.height,
        bytes: img.bytes,
    })
}
