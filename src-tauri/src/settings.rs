use rusqlite::params;
use serde::{Deserialize, Deserializer, Serialize};

use crate::db::DbConn;
use crate::error::{AppError, AppResult};

pub const KEY_API_KEY: &str = "api_key";
pub const KEY_ENDPOINT: &str = "endpoint";
pub const KEY_MODEL: &str = "model";
pub const KEY_DEFAULT_RATIO: &str = "default_aspect_ratio";
pub const KEY_DEFAULT_SIZE: &str = "default_image_size";
pub const KEY_SYSTEM_PROMPT: &str = "system_prompt";
pub const KEY_TEMPERATURE: &str = "temperature";
pub const KEY_TOP_P: &str = "top_p";
pub const KEY_MAX_TOKENS: &str = "max_tokens";
pub const KEY_FREQ_PENALTY: &str = "frequency_penalty";
pub const KEY_PRES_PENALTY: &str = "presence_penalty";
pub const KEY_HISTORY_TURNS: &str = "history_turns";

pub const DEFAULT_HISTORY_TURNS: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub api_key: String,
    pub endpoint: String,
    pub model: String,
    pub default_aspect_ratio: String,
    pub default_image_size: String,
    pub system_prompt: String,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    /// Number of prior messages (user + assistant, oldest dropped first)
    /// to include as multi-turn context. 0 disables history.
    pub history_turns: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            endpoint: String::new(),
            model: String::new(),
            default_aspect_ratio: String::new(),
            default_image_size: String::new(),
            system_prompt: String::new(),
            temperature: None,
            top_p: None,
            max_tokens: None,
            frequency_penalty: None,
            presence_penalty: None,
            history_turns: DEFAULT_HISTORY_TURNS,
        }
    }
}

/// Marker type for patch fields:
/// - field absent → `Unset` (don't touch persistent value)
/// - field present with `null` → `Set(None)` (clear stored value)
/// - field present with a value → `Set(Some(v))`
#[derive(Debug, Clone, Default)]
pub enum Patchable<T> {
    #[default]
    Unset,
    Set(Option<T>),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patchable<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Patchable::Set(Option::<T>::deserialize(d)?))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingsPatch {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub default_aspect_ratio: Option<String>,
    #[serde(default)]
    pub default_image_size: Option<String>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub temperature: Patchable<f64>,
    #[serde(default)]
    pub top_p: Patchable<f64>,
    #[serde(default)]
    pub max_tokens: Patchable<i64>,
    #[serde(default)]
    pub frequency_penalty: Patchable<f64>,
    #[serde(default)]
    pub presence_penalty: Patchable<f64>,
    #[serde(default)]
    pub history_turns: Option<i64>,
}

pub fn read(conn: &DbConn) -> AppResult<Settings> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map(params![], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut s = Settings {
        endpoint: "https://openrouter.ai/api/v1/chat/completions".into(),
        model: "openai/gpt-5.4-image-2".into(),
        default_aspect_ratio: "auto".into(),
        default_image_size: "auto".into(),
        ..Default::default()
    };
    for r in rows {
        let (k, v) = r?;
        match k.as_str() {
            KEY_API_KEY => s.api_key = v,
            KEY_ENDPOINT => s.endpoint = v,
            KEY_MODEL => s.model = v,
            KEY_DEFAULT_RATIO => s.default_aspect_ratio = v,
            KEY_DEFAULT_SIZE => s.default_image_size = v,
            KEY_SYSTEM_PROMPT => s.system_prompt = v,
            KEY_TEMPERATURE => s.temperature = parse_optional_f64(&v),
            KEY_TOP_P => s.top_p = parse_optional_f64(&v),
            KEY_MAX_TOKENS => s.max_tokens = parse_optional_i64(&v),
            KEY_FREQ_PENALTY => s.frequency_penalty = parse_optional_f64(&v),
            KEY_PRES_PENALTY => s.presence_penalty = parse_optional_f64(&v),
            KEY_HISTORY_TURNS => {
                if let Some(n) = parse_optional_i64(&v) {
                    s.history_turns = n.max(0);
                }
            }
            _ => {}
        }
    }
    Ok(s)
}

pub fn write_kv(conn: &DbConn, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn apply_patch(conn: &DbConn, patch: SettingsPatch) -> AppResult<Settings> {
    if let Some(v) = patch.api_key {
        write_kv(conn, KEY_API_KEY, &v)?;
    }
    if let Some(v) = patch.endpoint {
        write_kv(conn, KEY_ENDPOINT, &v)?;
    }
    if let Some(v) = patch.model {
        write_kv(conn, KEY_MODEL, &v)?;
    }
    if let Some(v) = patch.default_aspect_ratio {
        write_kv(conn, KEY_DEFAULT_RATIO, &v)?;
    }
    if let Some(v) = patch.default_image_size {
        write_kv(conn, KEY_DEFAULT_SIZE, &v)?;
    }
    if let Some(v) = patch.system_prompt {
        write_kv(conn, KEY_SYSTEM_PROMPT, &v)?;
    }
    write_optional_f64(conn, KEY_TEMPERATURE, &patch.temperature, "temperature")?;
    write_optional_f64(conn, KEY_TOP_P, &patch.top_p, "top_p")?;
    write_optional_i64(conn, KEY_MAX_TOKENS, &patch.max_tokens, "max_tokens")?;
    write_optional_f64(
        conn,
        KEY_FREQ_PENALTY,
        &patch.frequency_penalty,
        "frequency_penalty",
    )?;
    write_optional_f64(
        conn,
        KEY_PRES_PENALTY,
        &patch.presence_penalty,
        "presence_penalty",
    )?;
    if let Some(n) = patch.history_turns {
        if n < 0 {
            return Err(AppError::Invalid("history_turns 必须是非负整数".into()));
        }
        write_kv(conn, KEY_HISTORY_TURNS, &n.to_string())?;
    }
    read(conn)
}

fn parse_optional_f64(v: &str) -> Option<f64> {
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        t.parse().ok()
    }
}

fn parse_optional_i64(v: &str) -> Option<i64> {
    let t = v.trim();
    if t.is_empty() {
        None
    } else {
        t.parse().ok()
    }
}

fn write_optional_f64(
    conn: &DbConn,
    key: &str,
    value: &Patchable<f64>,
    label: &str,
) -> AppResult<()> {
    match value {
        Patchable::Unset => Ok(()),
        Patchable::Set(None) => write_kv(conn, key, ""),
        Patchable::Set(Some(n)) => {
            if !n.is_finite() {
                return Err(AppError::Invalid(format!("{label} 必须是有限数值")));
            }
            write_kv(conn, key, &n.to_string())
        }
    }
}

fn write_optional_i64(
    conn: &DbConn,
    key: &str,
    value: &Patchable<i64>,
    label: &str,
) -> AppResult<()> {
    match value {
        Patchable::Unset => Ok(()),
        Patchable::Set(None) => write_kv(conn, key, ""),
        Patchable::Set(Some(n)) => {
            if *n < 0 {
                return Err(AppError::Invalid(format!("{label} 必须是非负整数")));
            }
            write_kv(conn, key, &n.to_string())
        }
    }
}
