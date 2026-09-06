use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    JsonObject,
    JsonSchema,
    SchemaPrompt,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletionTokenParameter {
    #[default]
    MaxTokens,
    MaxCompletionTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModelRequest {
    pub messages: Vec<Message>,
    pub output_schema: Value,
    pub schema_name: String,
    pub max_output_tokens: u32,
    pub temperature: Option<f64>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Complete,
    Length,
    Refusal,
    ContentFilter,
    ToolCall,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct GenerationOptions {
    pub schema_name: String,
    pub max_output_tokens: u32,
    pub max_total_tokens: u64,
    pub max_calls: u32,
    pub max_repairs: u32,
    pub max_retries: u32,
    pub timeout_ms: u64,
    pub retry_backoff_ms: u64,
    pub temperature: Option<f64>,
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            schema_name: "result".into(),
            max_output_tokens: 1024,
            max_total_tokens: 100_000,
            max_calls: 9,
            max_repairs: 2,
            max_retries: 2,
            timeout_ms: 60_000,
            retry_backoff_ms: 250,
            temperature: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct UsageSummary {
    pub requests: u32,
    pub retries: u32,
    pub repairs: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutputDiagnostic {
    pub attempt: u32,
    pub reason: String,
    pub response_sha256: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Generation<T> {
    pub data: T,
    pub usage: UsageSummary,
    pub diagnostics: Vec<OutputDiagnostic>,
}
