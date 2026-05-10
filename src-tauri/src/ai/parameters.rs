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
}
