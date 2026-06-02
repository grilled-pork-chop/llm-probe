//! Wire types for the OpenAI-compatible chat-completions API.
//!
//! Only the fields we actually need are modeled (A.5/A.6). Request types borrow
//! (`&'a str`) to avoid cloning the prompt per request; response types own their
//! data since they outlive the borrowed request frame.

use serde::{Deserialize, Serialize};

/// Request body for `POST /v1/chat/completions`.
#[derive(Debug, Serialize)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<Message<'a>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Only set when streaming, to request a final usage-bearing chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Serialize)]
pub struct Message<'a> {
    pub role: &'a str,
    pub content: &'a str,
}

#[derive(Debug, Serialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

/// Non-streaming response.
#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    #[serde(default)]
    pub choices: Vec<Choice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: AssistantMsg,
}

#[derive(Debug, Deserialize)]
pub struct AssistantMsg {
    pub content: Option<String>,
}

/// Token accounting from the API's `usage` field — our only token source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Usage {
    pub completion_tokens: u32,
    pub prompt_tokens: u32,
}

/// A single streaming chunk (`data:` payload, excluding `[DONE]`).
///
/// `choices` may be **empty** on the final usage-bearing chunk when
/// `include_usage` is set — never index it unconditionally (A.6).
#[derive(Debug, Deserialize)]
pub struct ChatChunk {
    #[serde(default)]
    pub choices: Vec<DeltaChoice>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct DeltaChoice {
    pub delta: Delta,
}

#[derive(Debug, Deserialize)]
pub struct Delta {
    pub content: Option<String>,
}
