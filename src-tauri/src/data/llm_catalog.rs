//! Catalog of SDK metadata and default models + builtin supplier presets.
//! Seeded by migration `006_llm_catalog.sql`; read by settings merge and `get_llm_model_catalog`.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::HashMap;

use crate::data::db::DbConn;
use crate::data::settings::{normalize_route_provider_slugs, ModelProvider, ModelServiceModel};
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

fn parse_route_providers_json(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty() && *s != "null") else {
        return Vec::new();
    };
    let Ok(list) = serde_json::from_str::<Vec<String>>(raw) else {
        return Vec::new();
    };
    normalize_route_provider_slugs(&list)
}

fn load_sdk_models(conn: &DbConn, sdk_id: &str) -> AppResult<Vec<ModelServiceModel>> {
    let mut stmt = conn.prepare(
        "SELECT model_id, name, model_group, capabilities_json, context_window,
                route_providers_json
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
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, name, group, caps_json, context_window, route_json) = row?;
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
            route_providers: parse_route_providers_json(route_json.as_deref()),
        });
    }
    Ok(out)
}

fn load_supplier_models(conn: &DbConn, supplier_id: &str) -> AppResult<Vec<ModelServiceModel>> {
    let mut stmt = conn.prepare(
        "SELECT model_id, name, model_group, capabilities_json, context_window,
                route_providers_json
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
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (model_id, name, group, caps_json, context_window, route_json) = row?;
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
            route_providers: parse_route_providers_json(route_json.as_deref()),
        });
    }
    Ok(out)
}

/// User-edited OpenRouter pins, keyed by live provider id + model id.
fn load_route_overlay(conn: &DbConn) -> AppResult<HashMap<(String, String), Vec<String>>> {
    let mut stmt = conn.prepare(
        "SELECT provider_id, model_id, route_providers_json FROM llm_model_route",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (provider_id, model_id, json) = row?;
        let slugs = parse_route_providers_json(Some(&json));
        if !slugs.is_empty() {
            out.insert((provider_id, model_id), slugs);
        }
    }
    Ok(out)
}

/// Seed `llm_model_route` from the existing `model_services` JSON blob.
/// Used once when upgrading to schema 31 so pins saved before the table existed
/// land in the overlay without waiting for the next settings write.
pub fn backfill_route_overlay_from_settings(conn: &Connection) -> AppResult<()> {
    let Some(services) = read_model_services(conn) else {
        return Ok(());
    };
    sync_route_providers(conn, &services)
}

/// Write the current `model_services` routing pins into `llm_model_route`.
/// Also updates matching builtin catalog rows so the column stays in sync.
pub fn sync_route_providers(conn: &Connection, services: &[ModelProvider]) -> AppResult<()> {
    conn.execute("DELETE FROM llm_model_route", [])?;
    for provider in services {
        for model in &provider.models {
            let json = serde_json::to_string(&model.route_providers)
                .unwrap_or_else(|_| "[]".into());
            if !model.route_providers.is_empty() {
                conn.execute(
                    "INSERT INTO llm_model_route (provider_id, model_id, route_providers_json)
                     VALUES (?1, ?2, ?3)",
                    params![provider.id, model.id, json],
                )?;
            }
            conn.execute(
                "UPDATE llm_supplier_model
                 SET route_providers_json = ?1
                 WHERE supplier_id = ?2 AND model_id = ?3",
                params![json, provider.id, model.id],
            )?;
        }
    }
    Ok(())
}

/// Overlay user-saved pins onto the in-memory `model_services` list.
pub fn apply_route_provider_overlay(
    conn: &DbConn,
    services: &mut [ModelProvider],
) -> AppResult<()> {
    let overlay = load_route_overlay(conn)?;
    if overlay.is_empty() {
        return Ok(());
    }
    for provider in services {
        for model in &mut provider.models {
            if let Some(slugs) = overlay.get(&(provider.id.clone(), model.id.clone())) {
                model.route_providers = slugs.clone();
            }
        }
    }
    Ok(())
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
fn read_model_services(conn: &Connection) -> Option<Vec<ModelProvider>> {
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

#[cfg(test)]
mod overlay_tests {
    use super::*;
    use crate::data::db::test_support::TempDb;

    fn sample_services(slugs: &[&str]) -> Vec<ModelProvider> {
        vec![ModelProvider {
            id: "openrouter".into(),
            name: "OpenRouter".into(),
            sdk: "openai".into(),
            avatar: String::new(),
            endpoint: "https://openrouter.ai/api/v1/chat/completions".into(),
            api_key: "k".into(),
            enabled: true,
            context_cache_enabled: false,
            models: vec![ModelServiceModel {
                id: "qwen/qwen3.7-max".into(),
                name: "qwen3.7-max".into(),
                group: "qwen".into(),
                capabilities: vec!["text".into()],
                context_window: None,
                max_output_tokens: None,
                pricing: None,
                input_modalities: None,
                output_modalities: None,
                route_providers: slugs.iter().map(|s| (*s).to_string()).collect(),
            }],
        }]
    }

    #[test]
    fn route_pins_round_trip_through_overlay_table() {
        let db = TempDb::new("route-overlay");
        let conn = db.conn();
        sync_route_providers(&conn, &sample_services(&["alibaba", "together"])).unwrap();

        let json: String = conn
            .query_row(
                "SELECT route_providers_json FROM llm_model_route
                 WHERE provider_id = ?1 AND model_id = ?2",
                params!["openrouter", "qwen/qwen3.7-max"],
                |r| r.get(0),
            )
            .expect("overlay row");
        assert!(json.contains("alibaba"));

        let mut loaded = sample_services(&[]);
        apply_route_provider_overlay(&conn, &mut loaded).unwrap();
        assert_eq!(
            loaded[0].models[0].route_providers,
            vec!["alibaba".to_string(), "together".to_string()]
        );
    }

    #[test]
    fn settings_apply_patch_writes_overlay_and_survives_read() {
        let db = TempDb::new("route-settings");
        let conn = db.conn();
        crate::data::settings::apply_patch(
            &conn,
            crate::data::settings::SettingsPatch {
                model_services: Some(sample_services(&["Alibaba", "together"])),
                ..Default::default()
            },
        )
        .unwrap();

        let json: String = conn
            .query_row(
                "SELECT route_providers_json FROM llm_model_route
                 WHERE provider_id = ?1 AND model_id = ?2",
                params!["openrouter", "qwen/qwen3.7-max"],
                |r| r.get(0),
            )
            .expect("overlay row after apply_patch");
        assert!(json.contains("Alibaba") || json.contains("alibaba"));

        let loaded = crate::data::settings::read(&conn).unwrap();
        let model = loaded
            .model_services
            .iter()
            .find(|p| p.id == "openrouter")
            .and_then(|p| p.models.iter().find(|m| m.id == "qwen/qwen3.7-max"))
            .expect("model survives merge + overlay");
        assert_eq!(model.route_providers, vec!["Alibaba", "together"]);
    }

    #[test]
    fn backfill_copies_json_blob_into_overlay_table() {
        let db = TempDb::new("route-backfill");
        let conn = db.conn();
        let json = serde_json::to_string(&sample_services(&["google-ai-studio"])).unwrap();
        crate::data::settings::write_kv(
            &conn,
            crate::data::settings::KEY_MODEL_SERVICES,
            &json,
        )
        .unwrap();
        backfill_route_overlay_from_settings(&conn).unwrap();

        let stored: String = conn
            .query_row(
                "SELECT route_providers_json FROM llm_model_route
                 WHERE provider_id = ?1 AND model_id = ?2",
                params!["openrouter", "qwen/qwen3.7-max"],
                |r| r.get(0),
            )
            .expect("backfill row");
        assert!(stored.contains("google-ai-studio"));
    }

    #[test]
    fn clearing_pins_removes_the_overlay_row() {
        let db = TempDb::new("route-clear");
        let conn = db.conn();
        sync_route_providers(&conn, &sample_services(&["alibaba"])).unwrap();
        sync_route_providers(&conn, &sample_services(&[])).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM llm_model_route", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}
