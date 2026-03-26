use anyhow::{Context, Result, bail};
use reqwest::{Response, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    codex_cli,
    config::{AuthMode, Config},
};

const DEFAULT_INSTRUCTIONS: &str =
    "You are a helpful coding assistant. Answer directly and concisely.";

#[derive(Debug, Clone)]
pub struct CodexInferRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub instructions: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodexResponseRequest {
    pub model: Option<String>,
    pub input: Value,
    pub instructions: Option<String>,
    pub reasoning_effort: Option<String>,
    pub include_raw_events: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexExecutionResponse {
    pub text: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_events: Option<Vec<Value>>,
}

#[derive(Debug, Clone)]
pub struct CodexBackend {
    config: Config,
    client: reqwest::Client,
}

impl CodexBackend {
    pub fn new(config: Config) -> Result<Self> {
        let client = config.http_client()?;
        Ok(Self { config, client })
    }

    pub async fn infer(&self, request: CodexInferRequest) -> Result<CodexExecutionResponse> {
        self.response(CodexResponseRequest {
            model: request.model,
            input: single_user_message(&request.prompt),
            instructions: request.instructions,
            reasoning_effort: request.reasoning_effort,
            include_raw_events: false,
        })
        .await
    }

    pub async fn response(&self, request: CodexResponseRequest) -> Result<CodexExecutionResponse> {
        let payload = build_payload(&self.config, &request)?;
        let auth = resolve_auth(&self.config)?;
        match self
            .perform_response_once(&auth, &payload, request.include_raw_events)
            .await
        {
            Ok(response) => Ok(response),
            Err(BackendRequestError::Unauthorized) => Err(auth.unauthorized_error()),
            Err(BackendRequestError::Other(error)) => Err(error),
        }
    }

    async fn perform_response_once(
        &self,
        auth: &ResolvedAuth,
        payload: &Value,
        include_raw_events: bool,
    ) -> Result<CodexExecutionResponse, BackendRequestError> {
        let request = self
            .client
            .post(self.config.responses_url())
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", auth.access_token()),
            )
            .header(header::ACCEPT, "text/event-stream")
            .header(header::CONTENT_TYPE, "application/json")
            .json(payload);
        let request = auth.decorate_request(request);

        let response = request
            .send()
            .await
            .map_err(|error| BackendRequestError::Other(error.into()))?;
        if matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ) {
            return Err(BackendRequestError::Unauthorized);
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendRequestError::Other(anyhow::anyhow!(
                "Codex backend returned {status}: {body}"
            )));
        }

        parse_response(response, include_raw_events)
            .await
            .map_err(BackendRequestError::Other)
    }
}

pub fn validate_auth_configuration(config: &Config) -> Result<()> {
    resolve_auth(config).map(|_| ())
}

#[derive(Debug)]
enum BackendRequestError {
    Unauthorized,
    Other(anyhow::Error),
}

#[derive(Debug, Clone)]
enum ResolvedAuth {
    OAuth(codex_cli::CodexCredential),
    ApiToken(String),
}

impl ResolvedAuth {
    fn access_token(&self) -> &str {
        match self {
            Self::OAuth(credential) => &credential.access,
            Self::ApiToken(token) => token,
        }
    }

    fn account_id(&self) -> Option<&str> {
        match self {
            Self::OAuth(credential) => credential.account_id.as_deref(),
            Self::ApiToken(_) => None,
        }
    }

    fn decorate_request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::OAuth(_) => {
                let request = request.header("OpenAI-Beta", "responses=experimental");
                if let Some(account_id) = self.account_id() {
                    request.header("ChatGPT-Account-Id", account_id)
                } else {
                    request
                }
            }
            Self::ApiToken(_) => request,
        }
    }

    fn unauthorized_error(&self) -> anyhow::Error {
        match self {
            Self::OAuth(_) => codex_cli::expired_credential_error(),
            Self::ApiToken(_) => anyhow::anyhow!(
                "OpenAI Codex API token was rejected. Check OPENAI_CODEX_MCP_API_TOKEN and try again."
            ),
        }
    }
}

fn resolve_auth(config: &Config) -> Result<ResolvedAuth> {
    match config.auth_mode {
        AuthMode::OAuth => {
            let credential = codex_cli::read_required_codex_cli_credential()?;
            if codex_cli::credential_is_expired(&credential) {
                return Err(codex_cli::expired_credential_error());
            }
            Ok(ResolvedAuth::OAuth(credential))
        }
        AuthMode::ApiToken => {
            let token = config
                .api_token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "OPENAI_CODEX_MCP_API_TOKEN must be set when auth mode is `api_token`."
                    )
                })?;
            Ok(ResolvedAuth::ApiToken(token.to_owned()))
        }
    }
}

impl From<BackendRequestError> for anyhow::Error {
    fn from(value: BackendRequestError) -> Self {
        match value {
            BackendRequestError::Unauthorized => {
                anyhow::anyhow!("Codex backend request was unauthorized")
            }
            BackendRequestError::Other(error) => error,
        }
    }
}

fn build_payload(config: &Config, request: &CodexResponseRequest) -> Result<Value> {
    if !(request.input.is_string() || request.input.is_array()) {
        bail!("`input` must be a string or JSON array");
    }

    let mut payload = Map::new();
    payload.insert(
        "model".into(),
        Value::String(
            request
                .model
                .clone()
                .unwrap_or_else(|| config.model_name().to_owned()),
        ),
    );
    payload.insert("input".into(), request.input.clone());
    payload.insert("store".into(), Value::Bool(false));
    payload.insert("stream".into(), Value::Bool(true));

    payload.insert(
        "instructions".into(),
        Value::String(
            request
                .instructions
                .clone()
                .unwrap_or_else(|| DEFAULT_INSTRUCTIONS.to_owned()),
        ),
    );
    if let Some(reasoning_effort) = request.reasoning_effort.clone() {
        payload.insert(
            "reasoning".into(),
            json!({
                "effort": reasoning_effort,
            }),
        );
    }
    Ok(Value::Object(payload))
}

fn single_user_message(prompt: &str) -> Value {
    json!([
        {
            "type": "message",
            "role": "user",
            "content": [
                {
                    "type": "input_text",
                    "text": prompt,
                }
            ]
        }
    ])
}

async fn parse_response(
    mut response: Response,
    include_raw_events: bool,
) -> Result<CodexExecutionResponse> {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    if content_type.contains("application/json") {
        let json: Value = response
            .json()
            .await
            .context("failed to parse JSON response from Codex backend")?;
        return parse_json_response(json, include_raw_events);
    }

    let mut parser = SseParser::default();
    let mut state = StreamState::new(include_raw_events);

    while let Some(chunk) = response.chunk().await.context("failed to read SSE chunk")? {
        for frame in parser.push(&chunk)? {
            if frame.data.trim() == "[DONE]" {
                continue;
            }
            let event_json: Value =
                serde_json::from_str(&frame.data).context("failed to parse SSE event JSON")?;
            state.apply_event(event_json)?;
        }
    }

    state.finish()
}

fn parse_json_response(json: Value, include_raw_events: bool) -> Result<CodexExecutionResponse> {
    if let Some(output_text) = json.get("output_text").and_then(Value::as_str) {
        return Ok(CodexExecutionResponse {
            text: output_text.to_owned(),
            model: json
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            response_id: json
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            finish_reason: json
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            usage: json.get("usage").cloned(),
            raw_events: include_raw_events.then(|| vec![json]),
        });
    }

    let text = extract_output_text(&json);
    Ok(CodexExecutionResponse {
        text,
        model: json
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        response_id: json
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        finish_reason: json
            .get("status")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        usage: json.get("usage").cloned(),
        raw_events: include_raw_events.then(|| vec![json]),
    })
}

#[derive(Debug, Default)]
struct StreamState {
    text: String,
    model: Option<String>,
    response_id: Option<String>,
    finish_reason: Option<String>,
    usage: Option<Value>,
    raw_events: Option<Vec<Value>>,
}

impl StreamState {
    fn new(include_raw_events: bool) -> Self {
        Self {
            raw_events: include_raw_events.then(Vec::new),
            ..Self::default()
        }
    }

    fn apply_event(&mut self, event: Value) -> Result<()> {
        if let Some(raw_events) = &mut self.raw_events {
            raw_events.push(event.clone());
        }

        match event.get("type").and_then(Value::as_str) {
            Some("response.output_text.delta") => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.text.push_str(delta);
                }
            }
            Some("response.output_text.done") => {
                if self.text.is_empty() {
                    if let Some(text) = event.get("text").and_then(Value::as_str) {
                        self.text = text.to_owned();
                    }
                }
            }
            Some("response.completed") => {
                let response = event
                    .get("response")
                    .context("response.completed event was missing response")?;
                self.model = response
                    .get("model")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                self.response_id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                self.finish_reason = response
                    .get("status")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                self.usage = response.get("usage").cloned();
                if self.text.is_empty() {
                    self.text = extract_output_text(response);
                }
            }
            Some("response.failed") => {
                bail!(
                    "Codex backend reported a failed response: {}",
                    serde_json::to_string(&event).unwrap_or_else(|_| "<invalid-json>".into())
                );
            }
            _ => {}
        }

        Ok(())
    }

    fn finish(self) -> Result<CodexExecutionResponse> {
        Ok(CodexExecutionResponse {
            text: self.text,
            model: self.model.unwrap_or_default(),
            response_id: self.response_id,
            finish_reason: self.finish_reason,
            usage: self.usage,
            raw_events: self.raw_events,
        })
    }
}

#[derive(Debug, Default)]
struct SseParser {
    buffer: String,
}

#[derive(Debug)]
struct SseFrame {
    data: String,
}

impl SseParser {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>> {
        self.buffer
            .push_str(&String::from_utf8_lossy(chunk).replace("\r\n", "\n"));
        let mut frames = Vec::new();
        while let Some(index) = self.buffer.find("\n\n") {
            let frame = self.buffer[..index].to_owned();
            self.buffer.drain(..index + 2);
            if let Some(parsed) = parse_sse_frame(&frame) {
                frames.push(parsed);
            }
        }
        Ok(frames)
    }
}

fn parse_sse_frame(frame: &str) -> Option<SseFrame> {
    let mut data_lines = Vec::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_owned());
        }
    }
    if data_lines.is_empty() {
        None
    } else {
        Some(SseFrame {
            data: data_lines.join("\n"),
        })
    }
}

fn extract_output_text(value: &Value) -> String {
    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{SseParser, build_payload, extract_output_text};

    #[test]
    fn payload_includes_reasoning_and_streaming() {
        let config = crate::config::Config::default();
        let payload = build_payload(
            &config,
            &super::CodexResponseRequest {
                model: Some("gpt-test".into()),
                input: json!("hello"),
                instructions: Some("be concise".into()),
                reasoning_effort: Some("high".into()),
                include_raw_events: false,
            },
        )
        .expect("payload");

        assert_eq!(payload["stream"], json!(true));
        assert_eq!(payload["store"], json!(false));
        assert_eq!(payload["instructions"], json!("be concise"));
        assert_eq!(payload["reasoning"]["effort"], json!("high"));
    }

    #[test]
    fn payload_defaults_instructions_when_missing() {
        let config = crate::config::Config::default();
        let payload = build_payload(
            &config,
            &super::CodexResponseRequest {
                model: None,
                input: json!("hello"),
                instructions: None,
                reasoning_effort: None,
                include_raw_events: false,
            },
        )
        .expect("payload");

        assert_eq!(
            payload["instructions"],
            json!("You are a helpful coding assistant. Answer directly and concisely.")
        );
        assert_eq!(payload["store"], json!(false));
        assert_eq!(payload["stream"], json!(true));
    }

    #[test]
    fn payload_uses_api_token_default_model_when_needed() {
        let mut config = crate::config::Config::default();
        config.auth_mode = crate::config::AuthMode::ApiToken;
        let payload = build_payload(
            &config,
            &super::CodexResponseRequest {
                model: None,
                input: json!("hello"),
                instructions: None,
                reasoning_effort: None,
                include_raw_events: false,
            },
        )
        .expect("payload");

        assert_eq!(payload["model"], json!("gpt-5-mini"));
    }

    #[test]
    fn sse_parser_extracts_frames() {
        let mut parser = SseParser::default();
        let frames = parser
            .push(
                b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi\"}\n\n",
            )
            .expect("frames");
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.contains("\"delta\":\"Hi\""));
    }

    #[test]
    fn extracts_output_text_from_response_resource() {
        let value = json!({
            "output": [{
                "content": [
                    {"type": "output_text", "text": "Hello"},
                    {"type": "output_text", "text": " world"}
                ]
            }]
        });
        assert_eq!(extract_output_text(&value), "Hello world");
    }
}
