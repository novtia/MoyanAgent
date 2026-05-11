use rusqlite::params;
use serde::{Deserialize, Deserializer, Serialize};

use crate::data::db::DbConn;
use crate::error::{AppError, AppResult};

pub const KEY_API_KEY: &str = "api_key";
pub const KEY_ENDPOINT: &str = "endpoint";
pub const KEY_MODEL: &str = "model";
pub const KEY_ACTIVE_PROVIDER_ID: &str = "active_provider_id";
pub const KEY_MODEL_SERVICES: &str = "model_services";
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
pub struct ModelParamSettings {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    /// When `Some(true)`, request extended reasoning where the provider
    /// supports it (OpenAI: `reasoning_effort`; Claude: `output_config.effort`).
    #[serde(default)]
    pub thinking_enabled: Option<bool>,
    /// Provider-specific effort level, e.g. `low` / `medium` / `high` / `max`.
    /// When enabled and unset, backends default to `high`.
    #[serde(default)]
    pub thinking_effort: Option<String>,
}

impl ModelParamSettings {
    /// Effort string to send upstream when thinking is enabled.
    pub fn resolved_thinking_effort(&self) -> Option<String> {
        if !self.thinking_enabled.unwrap_or(false) {
            return None;
        }
        let effort = self
            .thinking_effort
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Some(effort.unwrap_or_else(|| "high".into()))
    }
}

impl Default for ModelParamSettings {
    fn default() -> Self {
        Self {
            temperature: None,
            top_p: None,
            max_tokens: None,
            frequency_penalty: None,
            presence_penalty: None,
            thinking_enabled: None,
            thinking_effort: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelServiceModel {
    pub id: String,
    pub name: String,
    pub group: String,
    pub capabilities: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_provider_sdk() -> String {
    crate::ai::providers::OPENAI_SDK.into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProvider {
    pub id: String,
    pub name: String,
    #[serde(default = "default_provider_sdk")]
    pub sdk: String,
    #[serde(default)]
    pub avatar: String,
    pub endpoint: String,
    pub api_key: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub models: Vec<ModelServiceModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub api_key: String,
    pub endpoint: String,
    pub model: String,
    pub active_provider_id: String,
    pub model_services: Vec<ModelProvider>,
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
            active_provider_id: String::new(),
            model_services: Vec::new(),
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
    pub active_provider_id: Option<String>,
    #[serde(default)]
    pub model_services: Option<Vec<ModelProvider>>,
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
        default_aspect_ratio: "auto".into(),
        default_image_size: "auto".into(),
        ..Default::default()
    };
    let mut parsed_services: Option<Vec<ModelProvider>> = None;
    for r in rows {
        let (k, v) = r?;
        match k.as_str() {
            KEY_API_KEY => s.api_key = v,
            KEY_ENDPOINT => s.endpoint = v,
            KEY_MODEL => s.model = v,
            KEY_ACTIVE_PROVIDER_ID => s.active_provider_id = v,
            KEY_MODEL_SERVICES => {
                parsed_services = serde_json::from_str::<Vec<ModelProvider>>(&v).ok()
            }
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
    s.model_services = normalize_services(merge_builtin_services(
        conn,
        parsed_services.unwrap_or_default(),
    )?);
    if s.active_provider_id.trim().is_empty()
        || !s
            .model_services
            .iter()
            .any(|p| p.id == s.active_provider_id)
    {
        s.active_provider_id = s
            .model_services
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default();
    }
    if !s.active_provider_id.is_empty() {
        let cur_ok = s
            .model_services
            .iter()
            .find(|p| p.id == s.active_provider_id)
            .map(|p| p.enabled)
            .unwrap_or(false);
        if !cur_ok {
            if let Some(p) = s.model_services.iter().find(|p| p.enabled) {
                s.active_provider_id = p.id.clone();
            }
        }
    }
    if let Some((endpoint, api_key, model)) = active_provider(&s).map(|provider| {
        let model = if provider.models.iter().any(|model| model.id == s.model) {
            s.model.clone()
        } else {
            provider
                .models
                .first()
                .map(|model| model.id.clone())
                .unwrap_or_default()
        };
        (provider.endpoint.clone(), provider.api_key.clone(), model)
    }) {
        s.endpoint = endpoint;
        s.api_key = api_key;
        s.model = model;
    } else {
        s.endpoint.clear();
        s.api_key.clear();
        s.model.clear();
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

pub fn active_provider(s: &Settings) -> Option<&ModelProvider> {
    if let Some(p) = s
        .model_services
        .iter()
        .find(|p| p.id == s.active_provider_id)
    {
        if p.enabled {
            return Some(p);
        }
    }
    s.model_services
        .iter()
        .find(|p| p.enabled)
        .or_else(|| s.model_services.first())
}

pub fn validate_model_param_settings(p: &ModelParamSettings) -> AppResult<()> {
    validate_optional_f64(p.temperature, "temperature")?;
    validate_optional_f64(p.top_p, "top_p")?;
    validate_optional_f64(p.frequency_penalty, "frequency_penalty")?;
    validate_optional_f64(p.presence_penalty, "presence_penalty")?;
    if let Some(n) = p.max_tokens {
        if n < 0 {
            return Err(AppError::Invalid("max_tokens must be non-negative".into()));
        }
    }
    if let Some(ref s) = p.thinking_effort {
        if s.trim().is_empty() {
            return Err(AppError::Invalid(
                "thinking_effort must not be empty when set".into(),
            ));
        }
    }
    Ok(())
}

fn merge_builtin_services(
    conn: &DbConn,
    mut services: Vec<ModelProvider>,
) -> AppResult<Vec<ModelProvider>> {
    let builtin_list = crate::data::llm_catalog::supplier_presets_as_providers(conn)?;
    for builtin in builtin_list {
        if let Some(existing) = services
            .iter_mut()
            .find(|provider| provider.id == builtin.id)
        {
            if existing.name.trim().is_empty() {
                existing.name = builtin.name.clone();
            }
            if existing.sdk.trim().is_empty()
                || existing.sdk.trim().eq_ignore_ascii_case("openrouter")
                || existing.sdk.trim().eq_ignore_ascii_case("deepseek")
            {
                existing.sdk = builtin.sdk.clone();
            }
            if !builtin.avatar.trim().is_empty()
                && (existing.avatar.trim().is_empty() || !avatar_is_image(&existing.avatar))
            {
                existing.avatar = builtin.avatar.clone();
            }
            if existing.endpoint.trim().is_empty() {
                existing.endpoint = builtin.endpoint.clone();
            }
            // Do not merge `builtin.models` into an existing provider: users may remove
            // default/catalog models; re-adding them on every read made deletes ineffective.
        } else {
            services.push(builtin);
        }
    }
    Ok(services)
}

fn normalize_services(mut services: Vec<ModelProvider>) -> Vec<ModelProvider> {
    for (provider_index, provider) in services.iter_mut().enumerate() {
        if provider.id.trim().is_empty() {
            provider.id = format!("provider-{}", provider_index + 1);
        }
        if provider.name.trim().is_empty() {
            provider.name = provider.id.clone();
        }
        provider.sdk = crate::ai::providers::normalize_sdk(&provider.sdk);
        provider.avatar = provider.avatar.trim().to_string();
        if !provider.avatar.is_empty() && !avatar_is_image(&provider.avatar) {
            provider.avatar.clear();
        }
        for model in &mut provider.models {
            if model.name.trim().is_empty() {
                model.name = short_model_name(&model.id);
            }
            if model.group.trim().is_empty() {
                model.group = model_group(&model.id);
            }
            if model.capabilities.is_empty() {
                model.capabilities = infer_capabilities(&model.id);
            }
        }
    }
    services
}

fn avatar_is_image(avatar: &str) -> bool {
    let avatar = avatar.trim().to_ascii_lowercase();
    avatar.starts_with('/')
        || avatar.starts_with("data:image/")
        || avatar.starts_with("http://")
        || avatar.starts_with("https://")
        || avatar.ends_with(".apng")
        || avatar.ends_with(".avif")
        || avatar.ends_with(".gif")
        || avatar.ends_with(".jpg")
        || avatar.ends_with(".jpeg")
        || avatar.ends_with(".png")
        || avatar.ends_with(".svg")
        || avatar.ends_with(".webp")
}

fn validate_services(services: &[ModelProvider]) -> AppResult<()> {
    for provider in services {
        let sdk = crate::ai::providers::normalize_sdk(&provider.sdk);
        if !crate::ai::providers::is_supported_sdk(&sdk) {
            return Err(AppError::Invalid(format!(
                "unsupported provider sdk: {}",
                provider.sdk
            )));
        }
        for model in &provider.models {
            if model.id.trim().is_empty() {
                return Err(AppError::Invalid("model id cannot be empty".into()));
            }
        }
    }
    Ok(())
}

fn validate_optional_f64(value: Option<f64>, label: &str) -> AppResult<()> {
    if value.map(|n| !n.is_finite()).unwrap_or(false) {
        return Err(AppError::Invalid(format!("{label} must be finite")));
    }
    Ok(())
}

fn short_model_name(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_string()
}

fn model_group(id: &str) -> String {
    id.split('/').next().unwrap_or("custom").to_string()
}

fn infer_capabilities(id: &str) -> Vec<String> {
    let id = id.to_ascii_lowercase();
    let mut out = Vec::new();
    if id.contains("image")
        || id.contains("vision")
        || id.contains("gemini")
        || id.contains("flux")
        || id.contains("gpt-5")
    {
        out.push("vision".into());
    }
    if id.contains("search") || id.contains("sonar") {
        out.push("web".into());
    }
    if id.contains("reason") || id.contains("thinking") || id.contains("o1") || id.contains("o3") {
        out.push("reasoning".into());
    }
    if out.is_empty() {
        out.push("text".into());
    }
    out
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
    if let Some(v) = patch.active_provider_id {
        write_kv(conn, KEY_ACTIVE_PROVIDER_ID, &v)?;
    }
    if let Some(v) = patch.model_services {
        validate_services(&v)?;
        let json = serde_json::to_string(&normalize_services(v))
            .map_err(|e| AppError::Invalid(e.to_string()))?;
        write_kv(conn, KEY_MODEL_SERVICES, &json)?;
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
