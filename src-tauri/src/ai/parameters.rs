use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::ai::tokens::TokenUsage;
use crate::data::settings::ModelParamSettings;

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ParameterScope {
    Model,
    Image,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ParameterRegistration {
    pub key: &'static str,
    pub scope: ParameterScope,
}

const REGISTERED_PARAMETERS: &[ParameterRegistration] = &[
    ParameterRegistration {
        key: "temperature",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "top_p",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "max_tokens",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "frequency_penalty",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "presence_penalty",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "thinking_enabled",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "thinking_effort",
        scope: ParameterScope::Model,
    },
    ParameterRegistration {
        key: "aspect_ratio",
        scope: ParameterScope::Image,
    },
    ParameterRegistration {
        key: "image_size",
        scope: ParameterScope::Image,
    },
    ParameterRegistration {
        key: "custom",
        scope: ParameterScope::Custom,
    },
];

#[derive(Debug, Clone)]
pub struct ParameterFactory;

impl ParameterFactory {
    pub fn registered(&self) -> &'static [ParameterRegistration] {
        REGISTERED_PARAMETERS
    }

    pub fn build(
        &self,
        aspect_ratio: String,
        image_size: String,
        model: ModelParamSettings,
    ) -> GenerationParameters {
        let _ = self.registered();
        GenerationParameters {
            aspect_ratio,
            image_size,
            model,
            custom: Map::new(),
        }
    }
}

pub fn factory() -> ParameterFactory {
    ParameterFactory
}

#[derive(Debug, Clone)]
pub struct GenerationParameters {
    pub aspect_ratio: String,
    pub image_size: String,
    pub model: ModelParamSettings,
    pub custom: Map<String, Value>,
}

impl GenerationParameters {
    pub fn apply_model_params(&self, body: &mut Map<String, Value>) {
        if let Some(v) = self.model.temperature {
            body.insert("temperature".into(), json!(v));
        }
        if let Some(v) = self.model.top_p {
            body.insert("top_p".into(), json!(v));
        }
        if let Some(v) = self.model.max_tokens {
            body.insert("max_tokens".into(), json!(v));
        }
        if let Some(v) = self.model.frequency_penalty {
            body.insert("frequency_penalty".into(), json!(v));
        }
        if let Some(v) = self.model.presence_penalty {
            body.insert("presence_penalty".into(), json!(v));
        }
        for (key, value) in &self.custom {
            body.insert(key.clone(), value.clone());
        }
    }

    /// OpenAI Chat Completions / compatible endpoints: `reasoning_effort`.
    pub fn apply_openai_reasoning_effort(&self, body: &mut Map<String, Value>) {
        if let Some(effort) = self.model.resolved_thinking_effort() {
            body.insert("reasoning_effort".into(), json!(effort));
        }
    }

    /// DeepSeek-style OpenAI-compatible APIs expect a top-level `thinking` object
    /// (e.g. `{"type":"enabled"}`) alongside `reasoning_effort`, matching Python
    /// `extra_body={"thinking": {"type": "enabled"}}` on `chat.completions.create`.
    ///
    /// Only applies when extended thinking is enabled and the model or endpoint
    /// looks DeepSeek-related, to avoid sending unknown fields to plain OpenAI.
    pub fn apply_openai_compat_thinking_object(
        &self,
        body: &mut Map<String, Value>,
        model: &str,
        endpoint: &str,
    ) {
        if self.model.resolved_thinking_effort().is_none() {
            return;
        }
        if !openai_compat_wants_thinking_body(model, endpoint) {
            return;
        }
        body.insert("thinking".into(), json!({ "type": "enabled" }));
    }

    pub fn image_config(&self) -> Option<Value> {
        let mut image_config = Map::new();
        if self.aspect_ratio != "auto" {
            image_config.insert(
                "aspect_ratio".into(),
                Value::String(self.aspect_ratio.clone()),
            );
        }
        if self.image_size != "auto" {
            image_config.insert("image_size".into(), Value::String(self.image_size.clone()));
        }
        if image_config.is_empty() {
            None
        } else {
            Some(Value::Object(image_config))
        }
    }

    pub fn to_message_params_json(&self) -> Value {
        json!({
            "aspect_ratio": self.aspect_ratio,
            "image_size": self.image_size,
        })
    }

    pub fn to_message_params_with_usage(&self, usage: &TokenUsage) -> Value {
        let mut params = self
            .to_message_params_json()
            .as_object()
            .cloned()
            .unwrap_or_default();
        if !usage.is_empty() {
            params.insert("usage".into(), json!(usage));
        }
        Value::Object(params)
    }

    /// Same as [`Self::to_message_params_with_usage`], plus optional
    /// model thinking/reasoning text returned on the assistant turn.
    pub fn to_assistant_message_params(
        &self,
        usage: &TokenUsage,
        thinking_content: Option<&str>,
    ) -> Value {
        let mut v = self.to_message_params_with_usage(usage);
        let Some(t) = thinking_content.map(str::trim).filter(|s| !s.is_empty()) else {
            return v;
        };
        if let Some(obj) = v.as_object_mut() {
            obj.insert("thinking_content".into(), json!(t));
        }
        v
    }
}

fn openai_compat_wants_thinking_body(model: &str, endpoint: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    let e = endpoint.trim().to_ascii_lowercase();
    m.contains("deepseek") || e.contains("deepseek")
}
