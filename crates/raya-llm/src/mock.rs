//! Scripted mock LLM provider for tests and offline runs.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use raya_core::{CompletionRequest, CompletionResponse, TokenUsage};
use serde_json::json;

use crate::provider::{LlmError, LlmProvider, LlmResult};

/// Mock provider that returns scripted responses in order.
pub struct MockProvider {
    responses: Mutex<VecDeque<CompletionResponse>>,
}

impl MockProvider {
    pub fn new(responses: Vec<CompletionResponse>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
        }
    }

    /// Default script: plan → write file → run test → finish.
    pub fn default_script() -> Self {
        let plan = CompletionResponse {
            content: json!({
                "type": "plan",
                "plan": {
                    "steps": [{
                        "id": "1",
                        "description": "Write hello file",
                        "expected_tools": ["filesystem.write"]
                    }],
                    "verification": "none",
                    "summary": "Write a hello file and verify"
                }
            })
            .to_string(),
            structured: None,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
            model: Some("mock".into()),
            finish_reason: Some("stop".into()),
        };

        let write = CompletionResponse {
            content: json!({
                "type": "tool_call",
                "call": {
                    "id": "00000000-0000-4000-8000-000000000001",
                    "name": "filesystem.write",
                    "input": {
                        "path": "hello.txt",
                        "content": "hello from raya\n"
                    }
                }
            })
            .to_string(),
            structured: None,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 15,
                total_tokens: 25,
            },
            model: Some("mock".into()),
            finish_reason: Some("stop".into()),
        };

        let finish = CompletionResponse {
            content: json!({
                "type": "finish",
                "summary": "Wrote hello.txt successfully"
            })
            .to_string(),
            structured: None,
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
            model: Some("mock".into()),
            finish_reason: Some("stop".into()),
        };

        Self::new(vec![plan, write, finish])
    }

    pub fn push(&self, response: CompletionResponse) {
        if let Ok(mut q) = self.responses.lock() {
            q.push_back(response);
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn complete(&self, _request: CompletionRequest) -> LlmResult<CompletionResponse> {
        let mut q = self
            .responses
            .lock()
            .map_err(|_| LlmError::Message("mock mutex poisoned".into()))?;
        q.pop_front().ok_or_else(|| {
            LlmError::InvalidResponse("mock provider has no more scripted responses".into())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raya_core::{Message, MessageRole};

    #[tokio::test]
    async fn script_returns_in_order() {
        let mock = MockProvider::default_script();
        let req = CompletionRequest {
            model: "mock".into(),
            messages: vec![Message {
                role: MessageRole::User,
                content: "hi".into(),
                name: None,
                tool_call_id: None,
            }],
            tools: None,
            structured_json: true,
            temperature: None,
            max_tokens: None,
        };
        let r1 = mock.complete(req.clone()).await.unwrap();
        assert!(r1.content.contains("plan"));
        let r2 = mock.complete(req.clone()).await.unwrap();
        assert!(r2.content.contains("tool_call"));
        let r3 = mock.complete(req).await.unwrap();
        assert!(r3.content.contains("finish"));
    }
}
