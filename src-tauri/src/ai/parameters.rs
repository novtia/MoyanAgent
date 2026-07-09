use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::ai::tokens::TokenUsage;
use crate::data::settings::ModelParamSettings;

#[derive(Debug, Clone, Copy, Serialize)]
pub enum ParameterScope {
    Model,
    Image,
    Video,
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
        key: "video_mode",
        scope: ParameterScope::Video,
    },
    ParameterRegistration {
        key: "video_duration",
        scope: ParameterScope::Video,
    },
    ParameterRegistration {
        key: "video_resolution",
        scope: ParameterScope::Video,
    },
    ParameterRegistration {
        key: "generate_audio",
        scope: ParameterScope::Video,
    },
    ParameterRegistration {
        key: "watermark",
        scope: ParameterScope::Video,
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
            video_mode: None,
            video_duration: None,
            video_resolution: None,
            generate_audio: None,
            watermark: None,
            camera_fixed: None,
            seed: None,
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
    pub video_mode: Option<String>,
    pub video_duration: Option<i64>,
    pub video_resolution: Option<String>,
    pub generate_audio: Option<bool>,
    pub watermark: Option<bool>,
    pub camera_fixed: Option<bool>,
    pub seed: Option<i64>,
    pub custom: Map<String, Value>,
}

impl GenerationParameters {
    #[allow(clippy::too_many_arguments)]
    pub fn with_video(
        mut self,
        mode: String,
        duration: i64,
        resolution: String,
        generate_audio: bool,
        watermark: bool,
        camera_fixed: Option<bool>,
        seed: Option<i64>,
    ) -> Self {
        self.video_mode = Some(mode);
        self.video_duration = Some(duration);
        self.video_resolution = Some(resolution);
        self.generate_audio = Some(generate_audio);
        self.watermark = Some(watermark);
        self.camera_fixed = camera_fixed;
        self.seed = seed;
        self
    }

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

    /// OpenRouter normalizes reasoning via the top-level `reasoning` object.
    /// Do not combine this with `reasoning_effort` or DeepSeek's `thinking` field.
    pub fn apply_openrouter_reasoning(&self, body: &mut Map<String, Value>) {
        if let Some(effort) = self.model.resolved_thinking_effort() {
            body.insert(
                "reasoning".into(),
                json!({
                    "effort": effort,
                    "enabled": true,
                }),
            );
        }
    }

    /// DeepSeek / Volcengine Ark (Doubao) chat APIs require an explicit
    /// `thinking.type`. If `reasoning_effort` is sent without
    /// `thinking.type: "enabled"`, Doubao defaults thinking to `disabled`
    /// and rejects the request.
    pub fn apply_native_thinking_object(&self, body: &mut Map<String, Value>) {
        if self.model.resolved_thinking_effort().is_some() {
            body.insert("thinking".into(), json!({ "type": "enabled" }));
        } else {
            body.insert("thinking".into(), json!({ "type": "disabled" }));
        }
    }

    /// Route thinking/reasoning controls to the shape each upstream expects.
    pub fn apply_thinking_params(&self, body: &mut Map<String, Value>, endpoint: &str) {
        if is_openrouter_endpoint(endpoint) {
            self.apply_openrouter_reasoning(body);
            return;
        }
        if uses_native_thinking_object(endpoint) {
            if self.model.resolved_thinking_effort().is_some() {
                self.apply_openai_reasoning_effort(body);
            }
            self.apply_native_thinking_object(body);
            return;
        }
        self.apply_openai_reasoning_effort(body);
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
        let mut params = Map::new();
        params.insert("aspect_ratio".into(), json!(self.aspect_ratio));
        params.insert("image_size".into(), json!(self.image_size));
        if let Some(value) = self.video_mode.as_ref() {
            params.insert("video_mode".into(), json!(value));
        }
        if let Some(value) = self.video_duration {
            params.insert("video_duration".into(), json!(value));
        }
        if let Some(value) = self.video_resolution.as_ref() {
            params.insert("video_resolution".into(), json!(value));
        }
        if let Some(value) = self.generate_audio {
            params.insert("generate_audio".into(), json!(value));
        }
        if let Some(value) = self.watermark {
            params.insert("watermark".into(), json!(value));
        }
        if let Some(value) = self.camera_fixed {
            params.insert("camera_fixed".into(), json!(value));
        }
        if let Some(value) = self.seed {
            params.insert("seed".into(), json!(value));
        }
        Value::Object(params)
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

fn is_openrouter_endpoint(endpoint: &str) -> bool {
    endpoint
        .trim()
        .to_ascii_lowercase()
        .contains("openrouter.ai")
}

fn uses_native_thinking_object(endpoint: &str) -> bool {
    is_deepseek_endpoint(endpoint) || is_volcengine_endpoint(endpoint)
}

fn is_deepseek_endpoint(endpoint: &str) -> bool {
    let e = endpoint.trim().to_ascii_lowercase();
    e.contains("deepseek.com") || e.contains("deepseek.ai")
}

fn is_volcengine_endpoint(endpoint: &str) -> bool {
    let e = endpoint.trim().to_ascii_lowercase();
    e.contains("volces.com") || e.contains("volcengine.com") || e.contains("volcengine.cn")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::settings::ModelParamSettings;

    fn params(thinking_enabled: bool) -> GenerationParameters {
        factory().build(
            "auto".into(),
            "auto".into(),
            ModelParamSettings {
                thinking_enabled: Some(thinking_enabled),
                thinking_effort: Some("high".into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn volcengine_thinking_enabled_sends_reasoning_effort_and_enabled_type() {
        let p = params(true);
        let mut body = Map::new();
        p.apply_thinking_params(
            &mut body,
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
        );
        assert_eq!(
            body.get("reasoning_effort").and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            body.get("thinking")
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str),
            Some("enabled")
        );
    }

    #[test]
    fn volcengine_thinking_disabled_sends_disabled_type_only() {
        let p = params(false);
        let mut body = Map::new();
        p.apply_thinking_params(
            &mut body,
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
        );
        assert!(!body.contains_key("reasoning_effort"));
        assert_eq!(
            body.get("thinking")
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str),
            Some("disabled")
        );
    }
}
