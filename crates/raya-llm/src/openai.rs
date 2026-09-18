//! OpenAI-compatible HTTP provider.

use std::time::Duration;

use async_trait::async_trait;
use raya_core::{CompletionRequest, CompletionResponse, MessageRole, TokenUsage, redact_secrets};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::provider::{LlmError, LlmProvider, LlmResult};

/// OpenAI-compatible chat completions client.
pub struct OpenAiCompatibleProvider {
    client: Client,
    base_url: String,
    api_key: String,
    default_model: String,
    timeout: Duration,
    cancel: CancellationToken,
}

impl std::fmt::Debug for OpenAiCompatibleProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleProvider")
            .field("base_url", &self.base_url)
            .field("default_model", &self.default_model)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

impl OpenAiCompatibleProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        default_model: impl Into<String>,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            default_model: default_model.into(),
            timeout,
            cancel,
        }
    }

    /// Build from config + environment (`RAYA_LLM_API_KEY` or `OPENAI_API_KEY`).
    pub fn from_env(
        base_url: &str,
        model: &str,
        timeout_secs: u64,
        cancel: CancellationToken,
    ) -> LlmResult<Self> {
        let key = std::env::var("RAYA_LLM_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .map_err(|_| LlmError::Auth)?;
        Ok(Self::new(
            base_url,
            key,
            model,
            Duration::from_secs(timeout_secs),
            cancel,
        ))
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<Usage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    async fn complete(&self, request: CompletionRequest) -> LlmResult<CompletionResponse> {
        if self.cancel.is_cancelled() {
            return Err(LlmError::Cancelled);
        }

        let model = if request.model.is_empty() {
            self.default_model.clone()
        } else {
            request.model.clone()
        };

        let messages: Vec<ChatMessage> = request
            .messages
            .iter()
            .map(|m| ChatMessage {
                role: match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                }
                .into(),
                content: m.content.clone(),
            })
            .collect();

        let body = ChatRequest {
            model,
            messages,
            response_format: if request.structured_json {
                Some(json!({"type": "json_object"}))
            } else {
                None
            },
            temperature: request.temperature,
            max_tokens: request.max_tokens,
        };

        let url = format!("{}/chat/completions", self.base_url);
        debug!(%url, "openai-compatible request");

        let req = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .timeout(self.timeout);

        let response = tokio::select! {
            _ = self.cancel.cancelled() => return Err(LlmError::Cancelled),
            res = req.send() => res.map_err(|e| {
                if e.is_timeout() {
                    LlmError::Timeout
                } else {
                    LlmError::Http(redact_secrets(&e.to_string()))
                }
            })?,
        };

        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(LlmError::Auth);
        }
        if status.as_u16() == 429 {
            return Err(LlmError::RateLimited);
        }
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(LlmError::Http(format!(
                "status {status}: {}",
                redact_secrets(&text)
            )));
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| LlmError::InvalidResponse(e.to_string()))?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::InvalidResponse("no choices".into()))?;
        let content = choice.message.content.unwrap_or_default();
        let structured = if request.structured_json {
            serde_json::from_str(&content).ok()
        } else {
            None
        };
        let usage = parsed.usage.unwrap_or_default();

        Ok(CompletionResponse {
            content,
            structured,
            usage: TokenUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            },
            model: parsed.model,
            finish_reason: choice.finish_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::Message;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn completes_via_http() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "gpt-test",
                "choices": [{
                    "message": {"role": "assistant", "content": "{\"type\":\"finish\",\"summary\":\"ok\"}"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
            })))
            .mount(&server)
            .await;

        let provider = OpenAiCompatibleProvider::new(
            server.uri(),
            "test-key",
            "gpt-test",
            Duration::from_secs(5),
            CancellationToken::new(),
        );

        let resp = provider
            .complete(CompletionRequest {
                model: "gpt-test".into(),
                messages: vec![Message::user("hi")],
                tools: None,
                structured_json: true,
                temperature: None,
                max_tokens: None,
            })
            .await
            .unwrap();

        assert!(resp.content.contains("finish"));
        assert_eq!(resp.usage.total_tokens, 3);
        let dbg = format!("{provider:?}");
        assert!(!dbg.contains("test-key"));
        assert!(dbg.contains("[REDACTED]"));
    }
}
