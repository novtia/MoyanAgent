//! Google Cloud Vertex AI Gemini (`generateContent` / SSE `streamGenerateContent`).
//!
//! Request JSON and stream parsing are shared with the AI Studio Gemini adapter.
//! This module only validates the Vertex URL template and selects Vertex auth.

use super::gemini::{generate_google, generate_google_stream, GoogleAuthStyle};
use super::{ChatProvider, ProviderFuture, VERTEX_SDK};
use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::error::{AppError, AppResult};

pub struct VertexProvider;

impl VertexProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for VertexProvider {
    fn sdk(&self) -> &'static str {
        VERTEX_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move {
            ensure_vertex_endpoint_ready(&request.provider.endpoint)?;
            generate_google(request, GoogleAuthStyle::VertexAuto).await
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest,
        on_text_delta: TextDeltaCallback,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            ensure_vertex_endpoint_ready(&request.provider.endpoint)?;
            generate_google_stream(request, on_text_delta, GoogleAuthStyle::VertexAuto).await
        })
    }
}

/// Vertex default catalog URLs keep `{project}` / `{location}` until the
/// settings UI fills them in. Refuse to send a request that would 404.
pub(crate) fn ensure_vertex_endpoint_ready(endpoint: &str) -> AppResult<()> {
    let e = endpoint.trim();
    if e.is_empty() {
        return Err(AppError::Config("Vertex API 地址不能为空。".into()));
    }
    if e.contains("{project}") || e.contains("{location}") {
        return Err(AppError::Config(
            "Vertex 地址仍含 {project} 或 {location}，请在设置里填写 GCP 项目 ID 和区域。".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::providers::gemini::gemini_url;

    #[test]
    fn leftover_placeholders_are_config_errors() {
        let err = ensure_vertex_endpoint_ready(
            "https://aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent",
        )
        .unwrap_err();
        match err {
            AppError::Config(msg) => {
                assert!(msg.contains("{project}") || msg.contains("{location}"));
            }
            other => panic!("expected Config, got {other:?}"),
        }
        let err = ensure_vertex_endpoint_ready(
            "https://aiplatform.googleapis.com/v1/projects/{project}/locations/global/publishers/google/models/{model}:generateContent",
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn filled_vertex_url_is_ready() {
        ensure_vertex_endpoint_ready(
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-proj/locations/us-central1/publishers/google/models/{model}:generateContent",
        )
        .unwrap();
    }

    #[test]
    fn empty_endpoint_is_config_error() {
        assert!(matches!(
            ensure_vertex_endpoint_ready("  "),
            Err(AppError::Config(_))
        ));
    }

    #[test]
    fn global_and_regional_stream_urls() {
        let global = "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/{model}:generateContent";
        assert_eq!(
            gemini_url(global, "gemini-2.5-flash", true),
            "https://aiplatform.googleapis.com/v1/projects/my-proj/locations/global/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        let regional = "https://europe-west1-aiplatform.googleapis.com/v1/projects/my-proj/locations/europe-west1/publishers/google/models/{model}:generateContent";
        assert_eq!(
            gemini_url(regional, "gemini-2.5-pro", false),
            "https://europe-west1-aiplatform.googleapis.com/v1/projects/my-proj/locations/europe-west1/publishers/google/models/gemini-2.5-pro:generateContent"
        );
    }
}
