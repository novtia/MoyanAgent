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
        });
    }
    Ok(out)
}

/// Catalog `context_window` for the active provider + model when the UI omits it
/// (persisted `model_services` often strips fields not stored in JSON).
pub fn lookup_context_window(
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
