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
        "SELECT model_id, name, model_group, capabilities_json
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
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, name, group, caps_json) = row?;
        out.push(ModelServiceModel {
            id: model_id,
            name,
            group,
            capabilities: parse_capabilities(&caps_json)?,
        });
    }
    Ok(out)
}

fn load_supplier_models(conn: &DbConn, supplier_id: &str) -> AppResult<Vec<ModelServiceModel>> {
    let mut stmt = conn.prepare(
        "SELECT model_id, name, model_group, capabilities_json
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
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, name, group, caps_json) = row?;
        out.push(ModelServiceModel {
            id: model_id,
            name,
            group,
            capabilities: parse_capabilities(&caps_json)?,
        });
    }
    Ok(out)
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
