use bytes::Bytes;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use futures::TryStreamExt;
use http::HeaderValue;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::debug;

use crate::AuthManager;
use crate::client_common::Prompt;
use crate::client_common::ResponseEvent;
use crate::client_common::ResponseStream;
use crate::client_common::tools::ResponsesApiTool;
use crate::client_common::tools::ToolSpec;
use crate::default_client::build_reqwest_client;
use crate::error::CodexErr;
use crate::error::Result;
use crate::error::UnexpectedResponseError;
use crate::model_provider_info::ModelProviderInfo;

static GEMINI_CALL_ID_COUNTER: AtomicI64 = AtomicI64::new(0);

fn next_gemini_call_id() -> String {
    let id = GEMINI_CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("gemini-function-call-{id}")
}

pub(crate) async fn stream_gemini(
    provider: &ModelProviderInfo,
    model_info: &ModelInfo,
    prompt: &Prompt,
    auth_manager: Option<&AuthManager>,
) -> Result<ResponseStream> {
    let Some(base_url) = provider.base_url.as_deref() else {
        return Err(CodexErr::UnsupportedOperation(
            "Gemini providers must define a base_url".to_string(),
        ));
    };
    let base_url = normalize_gemini_base_url(base_url);

    let model = model_info.slug.as_str();
    let api_model = strip_gemini_model_suffixes(model);
    let url = format!(
        "{}/models/{api_model}:streamGenerateContent?alt=sse",
        base_url.trim_end_matches('/')
    );

    let instructions = prompt.base_instructions.text.clone();
    let system_instruction = (!instructions.trim().is_empty()).then(|| GeminiContentRequest {
        role: None,
        parts: vec![GeminiPartRequest {
            text: Some(instructions),
            inline_data: None,
            function_call: None,
            function_response: None,
            thought_signature: None,
            compat_thought_signature: None,
        }],
    });

    let formatted_input = prompt.get_formatted_input();
    let mut contents = build_gemini_contents(&formatted_input, api_model);
    if contents.is_empty() {
        return Err(CodexErr::UnsupportedOperation(
            "Gemini requests require at least one message".to_string(),
        ));
    }
    contents = ensure_active_loop_has_thought_signatures(&contents);

    let tools = build_gemini_tools(&prompt.tools);
    let tool_config = tools.as_ref().map(|_| GeminiToolConfig {
        function_calling_config: GeminiFunctionCallingConfig {
            mode: GeminiFunctionCallingMode::Auto,
            allowed_function_names: None,
            stream_function_call_arguments: is_gemini_3_model(api_model).then_some(true),
        },
    });

    let request = GeminiRequest {
        system_instruction,
        contents,
        tools,
        tool_config,
        generation_config: Some(GeminiGenerationConfig {
            temperature: Some(1.0),
            top_k: Some(64),
            top_p: Some(0.95),
            max_output_tokens: None,
        }),
    };

    let client = build_reqwest_client();
    let gemini_api_key = crate::auth::read_gemini_api_key_from_env()
        .or_else(|| auth_manager.and_then(|manager| manager.gemini_api_key_from_storage()));

    let mut headers = provider.build_header_map()?;
    if let Some(api_key) = gemini_api_key.as_deref()
        && let Ok(value) = HeaderValue::from_str(api_key)
    {
        headers.insert("x-goog-api-key", value);
    }

    let response = client
        .post(&url)
        .headers(headers)
        .json(&request)
        .send()
        .await
        .map_err(|err| CodexErr::Stream(err.to_string(), None))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CodexErr::UnexpectedStatus(UnexpectedResponseError {
            status,
            body,
            url: Some(url),
            cf_ray: None,
            request_id: None,
        }));
    }

    let idle_timeout = provider.stream_idle_timeout();
    let byte_stream = response.bytes_stream();

    Ok(spawn_gemini_sse_stream(byte_stream, idle_timeout))
}

fn strip_gemini_model_suffixes(model: &str) -> &str {
    let model = model.strip_suffix("-codex").unwrap_or(model);
    let model = model.strip_suffix("-germini").unwrap_or(model);
    model.strip_suffix("-gemini").unwrap_or(model)
}

fn is_gemini_3_model(api_model: &str) -> bool {
    api_model.starts_with("gemini-3")
}

fn normalize_gemini_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if let Some(prefix) = trimmed.strip_suffix("/v1") {
        format!("{prefix}/v1beta")
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContentRequest>,
    contents: Vec<GeminiContentRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPartRequest>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPartRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inline_data: Option<GeminiInlineData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCallPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponsePart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thought_signature: Option<String>,
    // Some proxies still emit `thought_signature` instead of `thoughtSignature`;
    // include both when we have it to maximize compatibility.
    #[serde(skip_serializing_if = "Option::is_none", rename = "thought_signature")]
    compat_thought_signature: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiInlineData {
    mime_type: String,
    data: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCallPart {
    name: String,
    args: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionResponsePart {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    name: String,
    response: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    parts: Option<Vec<GeminiPartRequest>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiToolConfig {
    function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCallingConfig {
    mode: GeminiFunctionCallingMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_function_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_function_call_arguments: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum GeminiFunctionCallingMode {
    Auto,
    #[allow(dead_code)]
    Any,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiTool {
    #[serde(skip_serializing_if = "Option::is_none")]
    function_declarations: Option<Vec<GeminiFunctionDeclaration>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionDeclaration {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<i32>,
}

fn parse_data_url(url: &str) -> Option<(String, String)> {
    let without_prefix = url.strip_prefix("data:")?;
    let (meta, data) = without_prefix.split_once(',')?;
    let (mime, encoding) = meta.split_once(';')?;
    if !encoding.eq_ignore_ascii_case("base64") {
        return None;
    }
    Some((mime.to_string(), data.to_string()))
}

fn strip_additional_properties(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("additionalProperties");
            for v in map.values_mut() {
                strip_additional_properties(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                strip_additional_properties(v);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn build_gemini_tools(tools: &[ToolSpec]) -> Option<Vec<GeminiTool>> {
    let mut functions = Vec::new();
    for tool in tools {
        if let ToolSpec::Function(ResponsesApiTool {
            name,
            description,
            parameters,
            ..
        }) = tool
        {
            let params = serde_json::to_value(parameters).ok().map(|mut v| {
                strip_additional_properties(&mut v);
                v
            });
            functions.push(GeminiFunctionDeclaration {
                name: name.clone(),
                description: Some(description.clone()),
                parameters: params,
            });
        }
    }

    if functions.is_empty() {
        None
    } else {
        Some(vec![GeminiTool {
            function_declarations: Some(functions),
        }])
    }
}

fn map_gemini_role(role: &str) -> String {
    if role.eq_ignore_ascii_case("assistant") {
        "model".to_string()
    } else {
        "user".to_string()
    }
}

fn gemini_inline_data_part(mime_type: String, data: String) -> GeminiPartRequest {
    GeminiPartRequest {
        text: None,
        inline_data: Some(GeminiInlineData { mime_type, data }),
        function_call: None,
        function_response: None,
        thought_signature: None,
        compat_thought_signature: None,
    }
}

fn content_to_gemini_parts(content: &[ContentItem]) -> Vec<GeminiPartRequest> {
    let mut parts = Vec::new();
    for entry in content {
        match entry {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if text.trim().is_empty() {
                    continue;
                }
                parts.push(GeminiPartRequest {
                    text: Some(text.clone()),
                    inline_data: None,
                    function_call: None,
                    function_response: None,
                    thought_signature: None,
                    compat_thought_signature: None,
                });
            }
            ContentItem::InputImage { image_url } => {
                if let Some((mime, data)) = parse_data_url(image_url) {
                    parts.push(gemini_inline_data_part(mime, data));
                } else if !image_url.trim().is_empty() {
                    parts.push(GeminiPartRequest {
                        text: Some(format!("Image reference: {image_url}")),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                        thought_signature: None,
                        compat_thought_signature: None,
                    });
                }
            }
        }
    }
    parts
}

fn split_function_output_content(
    items: &[FunctionCallOutputContentItem],
) -> (Vec<String>, Vec<GeminiPartRequest>) {
    let mut text_parts = Vec::new();
    let mut inline_parts = Vec::new();

    for item in items {
        match item {
            FunctionCallOutputContentItem::InputText { text } => {
                if !text.trim().is_empty() {
                    text_parts.push(text.clone());
                }
            }
            FunctionCallOutputContentItem::InputImage { image_url } => {
                if let Some((mime, data)) = parse_data_url(image_url) {
                    inline_parts.push(gemini_inline_data_part(mime, data));
                } else if !image_url.trim().is_empty() {
                    text_parts.push(format!("Image reference: {image_url}"));
                }
            }
        }
    }

    (text_parts, inline_parts)
}

fn build_gemini_function_response_payload(
    output: &FunctionCallOutputPayload,
) -> (String, Vec<GeminiPartRequest>) {
    let (text_parts, inline_parts) = if let Some(items) = output.content_items()
        && !items.is_empty()
    {
        split_function_output_content(items)
    } else {
        let mut text_parts = Vec::new();
        if let Some(content) = output.text_content()
            && !content.trim().is_empty()
        {
            text_parts.push(content.to_string());
        }
        (text_parts, Vec::new())
    };

    let mut output_text = if text_parts.is_empty() {
        String::new()
    } else {
        text_parts.join("\n")
    };
    if output_text.is_empty() && !inline_parts.is_empty() {
        output_text = format!("Binary content provided ({} item(s)).", inline_parts.len());
    }

    (output_text, inline_parts)
}

fn build_gemini_contents(items: &[ResponseItem], api_model: &str) -> Vec<GeminiContentRequest> {
    let mut contents = Vec::new();
    let mut function_calls_by_id: HashMap<String, (String, Option<String>)> = HashMap::new();

    for item in items {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let parts = content_to_gemini_parts(content);
                if parts.is_empty() {
                    continue;
                }
                contents.push(GeminiContentRequest {
                    role: Some(map_gemini_role(role)),
                    parts,
                });
            }
            ResponseItem::FunctionCall {
                name,
                arguments,
                call_id,
                thought_signature,
                ..
            } => {
                function_calls_by_id
                    .insert(call_id.clone(), (name.clone(), thought_signature.clone()));
                let args: Value = serde_json::from_str(arguments)
                    .unwrap_or_else(|_| Value::Object(Default::default()));

                let should_merge = contents.last().is_some_and(|c| {
                    c.role.as_deref() == Some("model")
                        && c.parts.iter().all(|p| p.function_call.is_some())
                });

                if should_merge {
                    if let Some(last) = contents.last_mut() {
                        last.parts.push(GeminiPartRequest {
                            text: None,
                            inline_data: None,
                            function_call: Some(GeminiFunctionCallPart {
                                name: name.clone(),
                                args,
                            }),
                            function_response: None,
                            thought_signature: None,
                            compat_thought_signature: None,
                        });
                    }
                } else {
                    let sig = thought_signature.clone();
                    contents.push(GeminiContentRequest {
                        role: Some("model".to_string()),
                        parts: vec![GeminiPartRequest {
                            text: None,
                            inline_data: None,
                            function_call: Some(GeminiFunctionCallPart {
                                name: name.clone(),
                                args,
                            }),
                            function_response: None,
                            thought_signature: sig.clone(),
                            compat_thought_signature: sig,
                        }],
                    });
                }
            }
            ResponseItem::FunctionCallOutput { call_id, output } => {
                let (function_name, _) = function_calls_by_id
                    .get(call_id)
                    .map(|(name, sig)| (name.clone(), sig.clone()))
                    .unwrap_or_else(|| ("unknown_function".to_string(), None));

                let (output_text, mut inline_parts) =
                    build_gemini_function_response_payload(output);
                let response_value = serde_json::json!({
                    "output": output_text,
                    "success": output.success.unwrap_or(true),
                });

                let supports_multimodal = is_gemini_3_model(api_model);
                let nested_parts = if supports_multimodal && !inline_parts.is_empty() {
                    Some(std::mem::take(&mut inline_parts))
                } else {
                    None
                };

                let should_merge = contents.last().is_some_and(|c| {
                    c.role.as_deref() == Some("user")
                        && c.parts
                            .iter()
                            .all(|p| p.function_response.is_some() || p.inline_data.is_some())
                });

                let response_part = GeminiPartRequest {
                    text: None,
                    inline_data: None,
                    function_call: None,
                    function_response: Some(GeminiFunctionResponsePart {
                        id: Some(call_id.clone()),
                        name: function_name,
                        response: response_value,
                        parts: nested_parts,
                    }),
                    thought_signature: None,
                    compat_thought_signature: None,
                };

                if should_merge {
                    if let Some(last) = contents.last_mut() {
                        last.parts.push(response_part);
                        if !supports_multimodal {
                            last.parts.append(&mut inline_parts);
                        }
                    }
                } else {
                    let mut parts = vec![response_part];
                    if !supports_multimodal {
                        parts.append(&mut inline_parts);
                    }
                    contents.push(GeminiContentRequest {
                        role: Some("user".to_string()),
                        parts,
                    });
                }
            }
            _ => {}
        }
    }

    if tracing::enabled!(tracing::Level::DEBUG) {
        let mut func_call_count = 0;
        let mut func_resp_count = 0;
        for content in &contents {
            for part in &content.parts {
                if part.function_call.is_some() {
                    func_call_count += 1;
                }
                if part.function_response.is_some() {
                    func_resp_count += 1;
                }
            }
        }
        debug!(
            "Gemini: built {} contents with {} function calls and {} function responses",
            contents.len(),
            func_call_count,
            func_resp_count
        );
    }

    contents
}

fn ensure_active_loop_has_thought_signatures(
    contents: &[GeminiContentRequest],
) -> Vec<GeminiContentRequest> {
    const SYNTHETIC_THOUGHT_SIGNATURE: &str = "context_engineering_is_the_way_to_go";

    let mut new_contents = contents.to_vec();
    let mut last_user_with_text: Option<usize> = None;

    for (idx, content) in new_contents.iter().enumerate() {
        if !content
            .role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("user"))
        {
            continue;
        }

        if content
            .parts
            .iter()
            .any(|part| part.text.as_deref().is_some_and(|t| !t.trim().is_empty()))
        {
            last_user_with_text = Some(idx);
        }
    }

    let start = last_user_with_text.map(|idx| idx + 1).unwrap_or(0);

    for content in &mut new_contents[start..] {
        if !content
            .role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("model"))
        {
            continue;
        }

        let mut patched_first_call = false;
        for part in &mut content.parts {
            if part.function_call.is_some() && !patched_first_call {
                patched_first_call = true;
                if part.thought_signature.is_none() {
                    let signature = part
                        .compat_thought_signature
                        .clone()
                        .unwrap_or_else(|| SYNTHETIC_THOUGHT_SIGNATURE.to_string());
                    part.thought_signature = Some(signature.clone());
                    if part.compat_thought_signature.is_none() {
                        part.compat_thought_signature = Some(signature);
                    }
                } else if part.compat_thought_signature.is_none() {
                    part.compat_thought_signature = part.thought_signature.clone();
                }
            }

            if part.inline_data.is_some() && part.thought_signature.is_none() {
                let signature = part
                    .compat_thought_signature
                    .clone()
                    .unwrap_or_else(|| SYNTHETIC_THOUGHT_SIGNATURE.to_string());
                part.thought_signature = Some(signature.clone());
                if part.compat_thought_signature.is_none() {
                    part.compat_thought_signature = Some(signature);
                }
            }
        }
    }

    new_contents
}

fn spawn_gemini_sse_stream<S>(byte_stream: S, idle_timeout: Duration) -> ResponseStream
where
    S: futures::Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Unpin + Send + 'static,
{
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent>>(1600);
    tokio::spawn(async move {
        process_gemini_sse(byte_stream, tx_event, idle_timeout).await;
    });
    ResponseStream { rx_event }
}

async fn process_gemini_sse<S>(
    stream: S,
    tx_event: mpsc::Sender<Result<ResponseEvent>>,
    idle_timeout: Duration,
) where
    S: futures::Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Unpin,
{
    if tx_event.send(Ok(ResponseEvent::Created)).await.is_err() {
        return;
    }

    let mut stream = stream
        .map_ok(|b| b)
        .map_err(|e| std::io::Error::other(e.to_string()))
        .eventsource();

    let mut accumulated_text = String::new();
    let mut assistant_item_sent = false;
    let mut reasoning_item_sent = false;
    let mut function_calls: Vec<(String, String, Option<String>, String)> = Vec::new();
    let mut last_response_id = "gemini-stream".to_string();
    let mut last_token_usage: Option<TokenUsage> = None;
    let mut last_thought_signature: Option<String> = None;
    let mut last_inline_image: Option<(String, String)> = None;

    loop {
        let response = timeout(idle_timeout, stream.next()).await;
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("Gemini SSE stream error: {e}");
                break;
            }
            Ok(None) => break,
            Err(_) => {
                debug!("Gemini SSE idle timeout");
                break;
            }
        };

        if sse.data.trim().is_empty() {
            continue;
        }
        if sse.data.trim() == "[DONE]" {
            break;
        }

        let parsed: GeminiResponse = match serde_json::from_str(&sse.data) {
            Ok(v) => v,
            Err(err) => {
                debug!("Gemini SSE parse error: {err}");
                continue;
            }
        };

        if let Some(id) = parsed.response_id {
            last_response_id = id;
        }
        if let Some(usage) = parsed.usage_metadata {
            last_token_usage = Some(TokenUsage::from(usage));
        }

        let Some(candidates) = parsed.candidates else {
            continue;
        };
        for candidate in candidates {
            if let Some(content) = candidate.content
                && let Some(parts) = content.parts
            {
                for part in parts {
                    if let Some(sig) = part.thought_signature.clone() {
                        last_thought_signature = Some(sig);
                    }
                    let is_thought = part.thought.unwrap_or(false);

                    if is_thought
                        && let Some(text) = &part.text
                        && !text.is_empty()
                    {
                        if !reasoning_item_sent {
                            let item = ResponseItem::Reasoning {
                                id: format!("gemini-thought-{last_response_id}"),
                                summary: vec![],
                                content: None,
                                encrypted_content: None,
                            };
                            if tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(item)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            reasoning_item_sent = true;
                        }

                        if tx_event
                            .send(Ok(ResponseEvent::ReasoningContentDelta {
                                delta: text.clone(),
                                content_index: 0,
                            }))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }

                    if is_thought {
                        continue;
                    }

                    if let Some(text) = part.text
                        && !text.is_empty()
                    {
                        if !assistant_item_sent {
                            let item = ResponseItem::Message {
                                id: None,
                                role: "assistant".to_string(),
                                content: vec![],
                                end_turn: None,
                                phase: None,
                            };
                            if tx_event
                                .send(Ok(ResponseEvent::OutputItemAdded(item)))
                                .await
                                .is_err()
                            {
                                return;
                            }
                            assistant_item_sent = true;
                        }

                        if tx_event
                            .send(Ok(ResponseEvent::OutputTextDelta(text.clone())))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        accumulated_text.push_str(&text);
                    }

                    if let Some(inline_data) = part.inline_data
                        && !inline_data.data.trim().is_empty()
                        && !inline_data.mime_type.is_empty()
                    {
                        last_inline_image = Some((inline_data.mime_type, inline_data.data));
                    }

                    if let Some(call) = part.function_call {
                        let name = call.name;
                        let args = if call.args.is_null() {
                            "{}".to_string()
                        } else {
                            call.args.to_string()
                        };
                        let thought_signature =
                            part.thought_signature.or(last_thought_signature.clone());
                        if let Some(last) = function_calls.last_mut()
                            && last.0 == name
                            && last.1 == args
                        {
                            last.2 = thought_signature;
                        } else {
                            function_calls.push((
                                name,
                                args,
                                thought_signature,
                                next_gemini_call_id(),
                            ));
                        }
                    }
                }
            }
        }
    }

    if assistant_item_sent || last_inline_image.is_some() {
        let mut content = Vec::new();
        if !accumulated_text.is_empty() {
            content.push(ContentItem::OutputText {
                text: accumulated_text,
            });
        }
        if let Some((mime_type, data)) = last_inline_image
            && !mime_type.is_empty()
            && !data.trim().is_empty()
        {
            let image_url = format!("data:{mime_type};base64,{data}");
            content.push(ContentItem::InputImage { image_url });
        }

        if !content.is_empty() {
            let item = ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content,
                end_turn: None,
                phase: None,
            };
            let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
        }
    }

    for (name, arguments, thought_signature, call_id) in function_calls {
        let item = ResponseItem::FunctionCall {
            id: None,
            name,
            arguments,
            call_id,
            thought_signature,
        };
        let _ = tx_event.send(Ok(ResponseEvent::OutputItemDone(item))).await;
    }

    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id: last_response_id,
            token_usage: last_token_usage,
        }))
        .await;
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    #[serde(default)]
    candidates: Option<Vec<GeminiCandidate>>,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContentResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContentResponse {
    #[serde(default)]
    parts: Option<Vec<GeminiPartResponse>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPartResponse {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    inline_data: Option<GeminiInlineData>,
    #[serde(default)]
    function_call: Option<GeminiFunctionCallPart>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default, rename = "thoughtSignature", alias = "thought_signature")]
    thought_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsageMetadata {
    #[serde(default)]
    prompt_token_count: Option<i64>,
    #[serde(default)]
    candidates_token_count: Option<i64>,
    #[serde(default)]
    total_token_count: Option<i64>,
}

impl From<GeminiUsageMetadata> for TokenUsage {
    fn from(meta: GeminiUsageMetadata) -> Self {
        let input_tokens = meta.prompt_token_count.unwrap_or_default();
        let output_tokens = meta.candidates_token_count.unwrap_or_default();
        let total_tokens = meta
            .total_token_count
            .unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
        TokenUsage {
            input_tokens,
            cached_input_tokens: 0,
            output_tokens,
            reasoning_output_tokens: 0,
            total_tokens,
        }
    }
}
