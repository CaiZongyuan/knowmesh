use std::{
    error::Error,
    io::Read,
    time::{Duration, SystemTime},
};

use knowmesh_core::{
    canonical::workspace::ResolvedCompilerSettings,
    domain::sha256,
    error::{AppError, AppResult, ErrorType},
    ingest::cache::ModelIdentity,
    model::{
        CompletionTokenParameter, ModelRequest, ModelResponse, ResponseFormat, StopReason,
        TokenUsage,
    },
    ports::ModelProvider,
};
use reqwest::{blocking::Client, header, redirect::Policy};
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

#[derive(Debug, Clone)]
pub struct TransportOptions {
    pub connect_timeout_ms: u64,
    pub max_response_bytes: u64,
}
impl Default for TransportOptions {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 10_000,
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub struct OpenAiCompatible {
    client: Client,
    endpoint: Url,
    settings: ResolvedCompilerSettings,
    options: TransportOptions,
    identity: ModelIdentity,
}

impl OpenAiCompatible {
    pub fn new(settings: ResolvedCompilerSettings, options: TransportOptions) -> AppResult<Self> {
        if settings.provider != "openai-compatible"
            || settings.model.trim().is_empty()
            || settings.model.len() > 1024
            || settings.api_key.expose_secret().trim().is_empty()
            || options.connect_timeout_ms == 0
            || options.max_response_bytes == 0
            || options.max_response_bytes > 64 * 1024 * 1024
        {
            return Err(AppError::new(
                ErrorType::Configuration,
                "INVALID_MODEL_PROFILE",
                "Model provider settings or transport limits are invalid.",
            ));
        }
        let mut endpoint = Url::parse(&settings.base_url).map_err(|_| invalid_url())?;
        if !["http", "https"].contains(&endpoint.scheme())
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(invalid_url());
        }
        endpoint
            .path_segments_mut()
            .map_err(|_| invalid_url())?
            .pop_if_empty()
            .push("chat")
            .push("completions");
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_millis(options.connect_timeout_ms))
            .user_agent(concat!("KnowMesh/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| network_error(&error))?;
        let config = serde_json::to_vec(&(
            "openai-compatible-v1",
            endpoint.as_str(),
            settings.response_format,
            settings.max_tokens_parameter,
            options.connect_timeout_ms,
            options.max_response_bytes,
        ))
        .map_err(|_| invalid_response())?;
        let identity = ModelIdentity {
            provider: settings.provider.clone(),
            model: settings.model.clone(),
            config_sha256: sha256(&config),
        };
        Ok(Self {
            client,
            endpoint,
            settings,
            options,
            identity,
        })
    }

    pub fn identity(&self) -> &ModelIdentity {
        &self.identity
    }
}

impl ModelProvider for OpenAiCompatible {
    fn complete(&self, request: &ModelRequest) -> AppResult<ModelResponse> {
        if request.timeout_ms == 0 || request.max_output_tokens == 0 {
            return Err(AppError::new(
                ErrorType::Validation,
                "INVALID_MODEL_OPTIONS",
                "A model request requires positive output and timeout limits.",
            ));
        }
        let mut body =
            json!({"model": self.settings.model, "messages": request.messages, "stream": false});
        let token_parameter = match self.settings.max_tokens_parameter {
            CompletionTokenParameter::MaxTokens => "max_tokens",
            CompletionTokenParameter::MaxCompletionTokens => "max_completion_tokens",
        };
        body[token_parameter] = request.max_output_tokens.into();
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        match self.settings.response_format {
            ResponseFormat::JsonObject => body["response_format"] = json!({"type":"json_object"}),
            ResponseFormat::JsonSchema => {
                body["response_format"] = json!({"type":"json_schema","json_schema":{"name":request.schema_name,"strict":true,"schema":request.output_schema}})
            }
            ResponseFormat::SchemaPrompt => {}
        }
        let encoded = serde_json::to_vec(&body).map_err(|_| invalid_response())?;
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.settings.api_key.expose_secret())
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .timeout(Duration::from_millis(request.timeout_ms))
            .body(encoded)
            .send()
            .map_err(|error| network_error(&error))?;
        let status = response.status().as_u16();
        if status != 200 {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(retry_after_ms);
            return Err(http_error(status, retry_after));
        }
        if response
            .content_length()
            .is_some_and(|size| size > self.options.max_response_bytes)
        {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        response
            .take(self.options.max_response_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| network_error(&error))?;
        if bytes.len() as u64 > self.options.max_response_bytes {
            return Err(too_large());
        }
        let envelope: Completion =
            serde_json::from_slice(&bytes).map_err(|_| invalid_response())?;
        if envelope.choices.len() != 1 {
            return Err(invalid_response());
        }
        let choice = envelope
            .choices
            .into_iter()
            .next()
            .ok_or_else(invalid_response)?;
        let stop_reason = if choice
            .message
            .refusal
            .as_ref()
            .is_some_and(|value| !value.is_empty())
        {
            StopReason::Refusal
        } else if choice
            .message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
            || choice.message.function_call.is_some()
        {
            StopReason::ToolCall
        } else {
            match choice.finish_reason.as_deref() {
                Some("stop") => StopReason::Complete,
                Some("length") => StopReason::Length,
                Some("content_filter") => StopReason::ContentFilter,
                Some("tool_calls" | "function_call") => StopReason::ToolCall,
                _ => return Err(invalid_response()),
            }
        };
        let text = choice
            .message
            .content
            .and_then(|value| value.as_str().map(str::to_owned));
        if stop_reason == StopReason::Complete && text.is_none() {
            return Err(invalid_response());
        }
        let usage = envelope
            .usage
            .map(|usage| {
                if usage.total_tokens.is_some_and(|total| {
                    total != usage.prompt_tokens.saturating_add(usage.completion_tokens)
                }) {
                    return Err(invalid_response());
                }
                Ok(TokenUsage {
                    input_tokens: usage.prompt_tokens,
                    output_tokens: usage.completion_tokens,
                })
            })
            .transpose()?;
        Ok(ModelResponse {
            text: text.unwrap_or_default(),
            stop_reason,
            usage,
        })
    }
}

#[derive(Deserialize)]
struct Completion {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice {
    message: Reply,
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct Reply {
    content: Option<Value>,
    refusal: Option<String>,
    tool_calls: Option<Vec<Value>>,
    function_call: Option<Value>,
}
#[derive(Deserialize)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: Option<u64>,
}

fn retry_after_ms(value: &str) -> Option<u64> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(seconds.saturating_mul(1000));
    }
    httpdate::parse_http_date(value).ok().map(|date| {
        date.duration_since(SystemTime::now())
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    })
}

fn http_error(status: u16, retry_after: Option<u64>) -> AppError {
    let (kind, code, retryable) = match status {
        401 => (ErrorType::Configuration, "MODEL_AUTH_FAILED", false),
        403 => (ErrorType::Policy, "MODEL_ACCESS_DENIED", false),
        408 | 504 => (ErrorType::Network, "MODEL_TIMEOUT", true),
        429 => (ErrorType::Network, "MODEL_RATE_LIMIT", true),
        500..=599 => (ErrorType::Network, "MODEL_UNAVAILABLE", true),
        _ => (ErrorType::Model, "MODEL_REQUEST_REJECTED", false),
    };
    let mut details = json!({"http_status": status});
    if let Some(delay) = retry_after {
        details["retry_after_ms"] = delay.into();
    }
    AppError::new(
        kind,
        code,
        "The configured model service rejected the request.",
    )
    .retryable(retryable)
    .with_details(details)
}

fn network_error(error: &(dyn Error + 'static)) -> AppError {
    let mut current = Some(error);
    while let Some(error) = current {
        if error
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.is_timeout())
            || error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
        {
            return AppError::new(
                ErrorType::Network,
                "MODEL_TIMEOUT",
                "The model request exceeded its timeout.",
            )
            .retryable(true);
        }
        current = error.source();
    }
    AppError::new(
        ErrorType::Network,
        "MODEL_NETWORK_ERROR",
        "The model service could not be reached.",
    )
    .retryable(true)
}
fn invalid_url() -> AppError {
    AppError::new(
        ErrorType::Configuration,
        "INVALID_MODEL_URL",
        "Use an HTTP(S) API root without credentials, query parameters, or fragments.",
    )
}
fn invalid_response() -> AppError {
    AppError::new(
        ErrorType::Model,
        "MODEL_INVALID_RESPONSE",
        "The model service returned an invalid completion envelope.",
    )
}
fn too_large() -> AppError {
    AppError::new(
        ErrorType::Model,
        "MODEL_RESPONSE_TOO_LARGE",
        "The model response exceeds the configured byte limit.",
    )
}
