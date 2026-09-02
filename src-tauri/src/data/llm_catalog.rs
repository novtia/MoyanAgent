//! Catalog of SDK metadata and default models + builtin supplier presets.
//! Seeded by migration `006_llm_catalog.sql`; read by settings merge and `get_llm_model_catalog`.

use rusqlite::params;
use serde::Serialize;

use crate::data::db::DbConn;
use crate::data::settings::{ModelProvider, ModelServiceModel};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSdkConfigDto {
    pub id: String,
    pub label: String,
    pub description: String,
    pub default_name: String,
    pub default_endpoint: String,
    pub endpoint_placeholder: String,
    pub endpoint_hint: String,
    pub api_key_placeholder: String,
    pub api_key_hint: String,
    pub model_id_placeholder: String,
    pub model_id_hint: String,
    pub models: Vec<ModelServiceModel>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmModelCatalogDto {
    pub provider_sdk_options: Vec<ProviderSdkConfigDto>,
    pub builtin_provider_presets: Vec<ModelProvider>,
}

fn parse_capabilities(json: &str) -> AppResult<Vec<String>> {
    serde_json::from_str(json).map_err(|e| AppError::Invalid(format!("capabilities_json: {e}")))
}

fn load_sdk_models(conn: &DbConn, sdk_id: &str) -> AppResult<Vec<ModelServiceModel>> {
    let mut stmt = conn.prepare(
        "SELECT model_id, name, model_group, capabilities_json, context_window
         FROM llm_sdk_model
         WHERE sdk_id = ?1
         ORDER BY sort_order, id",
    )?;
    let rows = stmt.query_map(params![sdk_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, name, group, caps_json, context_window) = row?;
        out.push(ModelServiceModel {
            id: model_id,
            name,
            group,
            capabilities: parse_capabilities(&caps_json)?,
            context_window,
            max_output_tokens: None,
            pricing: None,
            input_modalities: None,
            output_modalities: None,
            route_providers: Vec::new(),
        });
    }
    Ok(out)
}

fn load_supplier_models(conn: &DbConn, supplier_id: &str) -> AppResult<Vec<ModelServiceModel>> {
    let mut stmt = conn.prepare(
        "SELECT model_id, name, model_group, capabilities_json, context_window
         FROM llm_supplier_model
         WHERE supplier_id = ?1
         ORDER BY sort_order, id",
    )?;
    let rows = stmt.query_map(params![supplier_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<i64>>(4)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, name, group, caps_json, context_window) = row?;
        out.push(ModelServiceModel {
            id: model_id,
            name,
            group,
            capabilities: parse_capabilities(&caps_json)?,
            context_window,
            max_output_tokens: None,
            pricing: None,
            input_modalities: None,
            output_modalities: None,
            route_providers: Vec::new(),
        });
    }
    Ok(out)
}

/// Fallback window for a model that neither the catalog tables nor the user's
/// `model_services` describe.
///
/// Deliberately conservative. Enforcing a 128k budget on a model that really
/// holds more only costs some capacity; the alternative — leaving the window
/// `None` — switches off *every* budget check at once, because history
/// trimming, the compaction threshold and the `max_tokens` clamp all read an
/// unknown window as "no limit". That is how a request reaches a million
/// tokens before anything inspects it.
pub const DEFAULT_CONTEXT_WINDOW: i64 = 128_000;

/// Ceiling on a user-configured `max_tokens` when the model does not publish
/// its own output limit.
///
/// Providers charge the completion reservation against the context window, so an
/// oversized value rejects requests whose messages would have fitted. Values far
/// above this are almost always a context-window figure typed into the sampling
/// field by mistake — no model in the catalog emits 128k tokens in one reply.
pub const MAX_COMPLETION_TOKENS_CEILING: i64 = 65_536;

/// Token limits known for a specific provider + model pair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelLimits {
    pub context_window: Option<i64>,
    pub max_output_tokens: Option<i64>,
}

fn positive(value: Option<i64>) -> Option<i64> {
    value.filter(|v| *v > 0)
}

/// The user's persisted `model_services` blob, if it parses.
fn read_model_services(conn: &DbConn) -> Option<Vec<ModelProvider>> {
    let json: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![crate::data::settings::KEY_MODEL_SERVICES],
            |r| r.get(0),
        )
        .ok()?;
    serde_json::from_str(&json).ok()
}

/// Limits recorded in the user's `model_services` settings blob.
///
/// The catalog tables only describe the builtin presets, so a model added by
/// hand — or imported from a gateway's `/models` — is invisible to them. Since
/// a session only captures a window when the model is (re)picked in the
/// composer, older sessions on such a model have no window at all. Reading the
/// settings blob is what makes the value typed into the UI take effect without
/// forcing a reselect.
fn lookup_in_model_services(
    conn: &DbConn,
    supplier_id: &str,
    model_id: &str,
) -> Option<ModelLimits> {
    let sid = supplier_id.trim();
    let mid = model_id.trim();
    if sid.is_empty() || mid.is_empty() {
        return None;
    }
    let services = read_model_services(conn)?;
    let model = services
        .iter()
        .filter(|p| p.id.trim() == sid)
        .flat_map(|p| p.models.iter())
        .find(|m| m.id.trim() == mid)?;
    Some(ModelLimits {
        context_window: positive(model.context_window),
        max_output_tokens: positive(model.max_output_tokens),
    })
}

/// Resolve the token limits for `supplier_id` + `model_id`.
///
/// Precedence is user-first: an explicit value in `model_services` overrides
/// the seeded catalog, so correcting a wrong window in the UI takes effect
/// immediately.
pub fn lookup_model_limits(
    conn: &DbConn,
    supplier_id: &str,
    sdk_id: &str,
    model_id: &str,
) -> AppResult<ModelLimits> {
    let from_settings = lookup_in_model_services(conn, supplier_id, model_id).unwrap_or_default();
    let mut limits = from_settings;

    if limits.context_window.is_none() {
        limits.context_window = lookup_catalog_context_window(conn, supplier_id, sdk_id, model_id)?;
    }
    Ok(limits)
}

/// Catalog `context_window` for the active provider + model when the UI omits it
/// (persisted `model_services` often strips fields not stored in JSON).
pub fn lookup_context_window(
    conn: &DbConn,
    supplier_id: &str,
    sdk_id: &str,
    model_id: &str,
) -> AppResult<Option<i64>> {
    Ok(lookup_model_limits(conn, supplier_id, sdk_id, model_id)?.context_window)
}

/// The window as resolved for budget enforcement: never `None`, so the agent
/// loop always has something to enforce. See [`DEFAULT_CONTEXT_WINDOW`].
pub fn resolve_context_window(
    conn: &DbConn,
    supplier_id: &str,
    sdk_id: &str,
    model_id: &str,
) -> i64 {
    lookup_context_window(conn, supplier_id, sdk_id, model_id)
        .ok()
        .flatten()
        .filter(|w| *w > 0)
        .unwrap_or(DEFAULT_CONTEXT_WINDOW)
}

fn lookup_catalog_context_window(
    conn: &DbConn,
    supplier_id: &str,
    sdk_id: &str,
    model_id: &str,
) -> AppResult<Option<i64>> {
    let sid = supplier_id.trim();
    let mid = model_id.trim();
    if !sid.is_empty() && !mid.is_empty() {
        let mut stmt = conn.prepare(
            "SELECT context_window FROM llm_supplier_model WHERE supplier_id = ?1 AND model_id = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![sid, mid])?;
        if let Some(row) = rows.next()? {
            let cw: Option<i64> = row.get(0)?;
            if cw.is_some() {
                return Ok(cw);
            }
        }
    }

    let sdk = crate::ai::providers::normalize_sdk(sdk_id);
    let mid = model_id.trim();
    if sdk.is_empty() || mid.is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT context_window FROM llm_sdk_model WHERE sdk_id = ?1 AND model_id = ?2 LIMIT 1",
    )?;
    let mut rows = stmt.query(params![sdk.as_str(), mid])?;
    if let Some(row) = rows.next()? {
        let cw: Option<i64> = row.get(0)?;
        return Ok(cw);
    }
    Ok(None)
}

/// Builtin supplier rows merged into persisted `model_services` (same ids as UI "cannot delete").
pub fn supplier_presets_as_providers(conn: &DbConn) -> AppResult<Vec<ModelProvider>> {
    let mut stmt = conn.prepare(
        "SELECT supplier_id, name, sdk_id, avatar, endpoint, enabled
         FROM llm_supplier_preset
         ORDER BY sort_order, supplier_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)? != 0,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (supplier_id, name, sdk_id, avatar, endpoint, enabled) = row?;
        let models = load_supplier_models(conn, &supplier_id)?;
        out.push(ModelProvider {
            id: supplier_id,
            name,
            sdk: sdk_id,
            avatar,
            endpoint,
            api_key: String::new(),
            enabled,
            context_cache_enabled: false,
            models,
        });
    }
    Ok(out)
}

pub fn fetch_for_frontend(conn: &DbConn) -> AppResult<LlmModelCatalogDto> {
    let mut stmt = conn.prepare(
        "SELECT sdk_id, label, description, default_name, default_endpoint,
                endpoint_placeholder, endpoint_hint, api_key_placeholder, api_key_hint,
                model_id_placeholder, model_id_hint
         FROM llm_sdk_option
         ORDER BY sort_order, sdk_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
            r.get::<_, String>(10)?,
        ))
    })?;

    let mut provider_sdk_options = Vec::new();
    for row in rows {
        let (
            sdk_id,
            label,
            description,
            default_name,
            default_endpoint,
            endpoint_placeholder,
            endpoint_hint,
            api_key_placeholder,
            api_key_hint,
            model_id_placeholder,
            model_id_hint,
        ) = row?;
        let models = load_sdk_models(conn, &sdk_id)?;
        provider_sdk_options.push(ProviderSdkConfigDto {
            id: sdk_id,
            label,
            description,
            default_name,
            default_endpoint,
            endpoint_placeholder,
            endpoint_hint,
            api_key_placeholder,
            api_key_hint,
            model_id_placeholder,
            model_id_hint,
            models,
        });
    }

    let builtin_provider_presets = supplier_presets_as_providers(conn)?;

    Ok(LlmModelCatalogDto {
        provider_sdk_options,
        builtin_provider_presets,
    })
}
