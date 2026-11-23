use std::collections::BTreeMap;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::ToolContext;
use crate::commands::model_provider::Protocol;

const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn frame(kind: &str, value: Value) -> String {
    format!("event: {kind}\ndata: {value}\n\n")
}

#[derive(Default)]
struct Tool {
    id: String,
    name: String,
    args: String,
    context: Value,
}

impl Tool {
    fn block(self, context: &ToolContext) -> Result<Value, &'static str> {
        let args = if self.args.is_empty() {
            "{}"
        } else {
            &self.args
        };
        let input = serde_json::from_str::<Value>(args)
            .ok()
            .filter(Value::is_object)
            .ok_or("The upstream provider returned invalid tool arguments")?;
        if self.name.is_empty() {
            return Err("The upstream provider returned an unnamed tool call");
        }
        let id = if self.id.is_empty() {
            format!("call_{}", uuid::Uuid::new_v4().simple())
        } else {
            self.id
        };
        context.remember(&id, self.context);
        Ok(json!({"type":"tool_use","id":id,"name":self.name,"input":input}))
    }
}

#[derive(Default)]
struct Reply {
    tools: BTreeMap<u64, Tool>,
    reason: Option<&'static str>,
    input: u64,
    output: Option<u64>,
    text_bytes: usize,
    text_open: bool,
    thinking_open: bool,
}

impl Reply {
    fn text(&mut self, text: &str) -> String {
        if text.is_empty() {
            return String::new();
        }
        self.text_bytes += text.len();
        let mut frames = String::new();
        if !self.text_open {
            if self.thinking_open {
                frames.push_str(&frame(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":0}),
                ));
            }
            let index = if self.thinking_open { 1 } else { 0 };
            frames.push_str(&frame("content_block_start", json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}})));
            self.text_open = true;
        }
        let index = if self.thinking_open { 1 } else { 0 };
        frames.push_str(&frame("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":text}})));
        frames
    }

    fn thinking(&mut self, text: &str) -> String {
        if text.is_empty() || self.text_open {
            return String::new();
        }
        let mut frames = String::new();
        if !self.thinking_open {
            frames.push_str(&frame("content_block_start", json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}})));
            self.thinking_open = true;
        }
        frames.push_str(&frame("content_block_delta", json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":text}})));
        frames
    }

    fn chat(&mut self, value: &Value, protocol: Protocol) -> Result<String, &'static str> {
        if !value["error"].is_null() {
            return Err("The upstream provider rejected the completion");
        }
        if let Some(tokens) = value["usage"]["prompt_tokens"].as_u64() {
            self.input = tokens;
        }
        if let Some(tokens) = value["usage"]["completion_tokens"].as_u64() {
            self.output = Some(tokens);
        }
        let choice = &value["choices"][0];
        let delta = if choice["delta"].is_object() {
            &choice["delta"]
        } else {
            &choice["message"]
        };
        if let Some(calls) = delta["tool_calls"].as_array() {
            for (index, call) in calls.iter().enumerate() {
                let tool = self
                    .tools
                    .entry(call["index"].as_u64().unwrap_or(index as u64))
                    .or_default();
                if let Some(id) = call["id"].as_str() {
                    tool.id = id.to_owned();
                }
                if let Some(name) = call["function"]["name"].as_str().filter(|s| !s.is_empty()) {
                    if tool.name.is_empty() {
                        tool.name = name.to_owned();
                    } else if tool.name != name {
                        tool.name.push_str(name);
                    }
                }
                if let Some(args) = call["function"]["arguments"].as_str() {
                    tool.args.push_str(args);
                }
                if protocol == Protocol::Gemini
                    && call["extra_content"]["google"]["thought_signature"].is_string()
                {
                    tool.context = call["extra_content"].clone();
                }
            }
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.reason = Some(match reason {
                "length" => "max_tokens",
                "tool_calls" | "function_call" => "tool_use",
                "content_filter" => "refusal",
                _ => "end_turn",
            });
        }
        Ok(self.text(
            delta["content"]
                .as_str()
                .or_else(|| delta["refusal"].as_str())
                .unwrap_or_default(),
        ))
    }

    fn response_item(&mut self, index: u64, item: &Value) {
        if item["type"] != "function_call" {
            return;
        }
        let tool = self.tools.entry(index).or_default();
        if let Some(id) = item["call_id"].as_str() {
            tool.id = id.to_owned();
        }
        if let Some(name) = item["name"].as_str() {
            tool.name = name.to_owned();
        }
        if let Some(args) = item["arguments"].as_str().filter(|s| !s.is_empty()) {
            tool.args = args.to_owned();
        }
    }

    fn responses(&mut self, value: &Value) -> Result<(String, bool), &'static str> {
        let index = value["output_index"].as_u64().unwrap_or_default();
        match value["type"].as_str() {
            Some("response.reasoning_summary_text.delta") => Ok((
                self.thinking(value["delta"].as_str().unwrap_or_default()),
                false,
            )),
            Some("response.output_text.delta" | "response.refusal.delta") => Ok((
                self.text(value["delta"].as_str().unwrap_or_default()),
                false,
            )),
            Some("response.output_item.added" | "response.output_item.done") => {
                self.response_item(index, &value["item"]);
                Ok((String::new(), false))
            }
            Some("response.function_call_arguments.delta") => {
                if let Some(args) = value["delta"].as_str() {
                    self.tools.entry(index).or_default().args.push_str(args);
                }
                Ok((String::new(), false))
            }
            Some("response.function_call_arguments.done") => {
                if let Some(args) = value["arguments"].as_str() {
                    self.tools.entry(index).or_default().args = args.to_owned();
                }
                Ok((String::new(), false))
            }
            Some("response.completed" | "response.incomplete") => {
                Ok((self.complete_response(&value["response"]), true))
            }
            Some("response.failed" | "error") => Err("The upstream Responses API failed"),
            _ if value["object"] == "response"
                && matches!(value["status"].as_str(), Some("completed" | "incomplete")) =>
            {
                Ok((self.complete_response(value), true))
            }
            _ => Ok((String::new(), false)),
        }
    }

    fn complete_response(&mut self, response: &Value) -> String {
        let mut text = String::new();
        let include_text = !self.text_open;
        let mut reasoning = Vec::new();
        if let Some(items) = response["output"].as_array() {
            for (index, item) in items.iter().enumerate() {
                self.response_item(index as u64, item);
                if item["type"] == "reasoning" && item["encrypted_content"].is_string() {
                    reasoning.push(item.clone());
                }
                if item["type"] == "function_call" && !reasoning.is_empty() {
                    if let Some(tool) = self.tools.get_mut(&(index as u64)) {
                        tool.context = json!(std::mem::take(&mut reasoning));
                    }
                }
                if include_text && item["type"] == "message" {
                    if let Some(content) = item["content"].as_array() {
                        for part in content {
                            if let Some(value) =
                                part["text"].as_str().or_else(|| part["refusal"].as_str())
                            {
                                text.push_str(&self.text(value));
                            }
                        }
                    }
                }
            }
        }
        if let Some(tokens) = response["usage"]["input_tokens"].as_u64() {
            self.input = tokens;
        }
        self.output = response["usage"]["output_tokens"].as_u64();
        self.reason = Some(match response["incomplete_details"]["reason"].as_str() {
            Some("max_output_tokens") => "max_tokens",
            Some("content_filter") => "refusal",
            _ => "end_turn",
        });
        text
    }

    fn stop_reason(&self) -> Result<&'static str, &'static str> {
        let reported_reason = self
            .reason
            .ok_or("The upstream stream ended without a completion event")?;
        let use_tools =
            !self.tools.is_empty() && !matches!(reported_reason, "max_tokens" | "refusal");
        if reported_reason == "tool_use" && !use_tools {
            return Err("The upstream provider returned no tool call");
        }
        Ok(if use_tools {
            "tool_use"
        } else {
            reported_reason
        })
    }

    fn finish(self, context: &ToolContext) -> Result<String, &'static str> {
        let reason = self.stop_reason()?;
        let mut frames = String::new();
        let mut index = 0;
        let mut output_bytes = self.text_bytes;
        if self.thinking_open && !self.text_open {
            frames.push_str(&frame(
                "content_block_stop",
                json!({"type":"content_block_stop","index":0}),
            ));
            index = 1;
        } else if self.text_open {
            frames.push_str(&frame(
                "content_block_stop",
                json!({"type":"content_block_stop","index":if self.thinking_open { 1 } else { 0 }}),
            ));
            index = if self.thinking_open { 2 } else { 1 };
        }
        if reason == "tool_use" {
            for tool in self.tools.into_values() {
                let mut block = tool.block(context)?;
                let args = std::mem::replace(&mut block["input"], json!({})).to_string();
                output_bytes += args.len();
                frames.push_str(&frame(
                    "content_block_start",
                    json!({"type":"content_block_start","index":index,"content_block":block}),
                ));
                frames.push_str(&frame("content_block_delta", json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":args}})));
                frames.push_str(&frame(
                    "content_block_stop",
                    json!({"type":"content_block_stop","index":index}),
                ));
                index += 1;
            }
        }
        frames.push_str(&frame("message_delta", json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},"usage":{"input_tokens":self.input,"output_tokens":self.output.unwrap_or(output_bytes.div_ceil(4) as u64)}})));
        frames.push_str(&frame("message_stop", json!({"type":"message_stop"})));
        Ok(frames)
    }
}

pub(super) async fn json_message(
    mut upstream: reqwest::Response,
    protocol: Protocol,
    model: &str,
    count: u64,
    context: &ToolContext,
) -> Result<Value, &'static str> {
    let mut bytes = Vec::new();
    while let Some(chunk) = upstream.chunk().await.map_err(|error| {
        if error.is_timeout() {
            "The upstream provider timed out while reading the reply"
        } else {
            "The upstream reply was interrupted by a network error"
        }
    })? {
        if bytes.len() + chunk.len() > MAX_EVENT_BYTES {
            return Err("The upstream reply exceeded the size limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The upstream provider sent an invalid reply")?;
    let mut reply = Reply {
        input: count,
        ..Reply::default()
    };
    let mut content = Vec::new();
    if protocol == Protocol::OpenAiResponses {
        reply.responses(&value)?;
        if let Some(items) = value["output"].as_array() {
            for item in items.iter().filter(|item| item["type"] == "message") {
                if let Some(parts) = item["content"].as_array() {
                    for part in parts {
                        if let Some(text) =
                            part["text"].as_str().or_else(|| part["refusal"].as_str())
                        {
                            content.push(json!({"type":"text","text":text}));
                        }
                    }
                }
            }
        }
    } else {
        reply.chat(&value, protocol)?;
        let message = &value["choices"][0]["message"];
        if let Some(text) = message["content"]
            .as_str()
            .or_else(|| message["refusal"].as_str())
        {
            content.push(json!({"type":"text","text":text}));
        }
    }
    let reason = reply.stop_reason()?;
    if reason == "tool_use" {
        for tool in reply.tools.into_values() {
            content.push(tool.block(context)?);
        }
    }
    let id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    Ok(
        json!({"id":id,"model":model,"role":"assistant","type":"message","content":content,"stop_reason":reason,"stop_sequence":null,"usage":{"input_tokens":reply.input,"output_tokens":reply.output.unwrap_or(reply.text_bytes.div_ceil(4) as u64)}}),
    )
}

// Decode SSE by lines, so neither split UTF-8 nor split CRLF corrupts deltas.
#[derive(Default)]
struct Sse {
    line: Vec<u8>,
    data: Vec<u8>,
}

impl Sse {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, &'static str> {
        let mut events = Vec::new();
        for piece in chunk.split_inclusive(|byte| *byte == b'\n') {
            self.line.extend_from_slice(piece);
            if self.line.len() + self.data.len() > MAX_EVENT_BYTES {
                return Err("An upstream event exceeded the size limit");
            }
            if !piece.ends_with(b"\n") {
                continue;
            }
            self.line.pop();
            if self.line.ends_with(b"\r") {
                self.line.pop();
            }
            if self.line.is_empty() {
                if !self.data.is_empty() {
                    self.data.pop();
                    events.push(std::mem::take(&mut self.data));
                }
            } else if let Some(value) = self.line.strip_prefix(b"data:") {
                self.data
                    .extend_from_slice(value.strip_prefix(b" ").unwrap_or(value));
                self.data.push(b'\n');
            }
            self.line.clear();
        }
        Ok(events)
    }
}

pub(super) async fn forward(
    mut upstream: reqwest::Response,
    protocol: Protocol,
    model: String,
    count: u64,
    sse: bool,
    sender: &mpsc::Sender<String>,
    context: &ToolContext,
) -> Result<(), &'static str> {
    let id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let mut reply = Reply {
        input: count,
        ..Reply::default()
    };
    let start = frame(
        "message_start",
        json!({"type":"message_start","message":{"id":id,"model":model,"role":"assistant","type":"message","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":count,"output_tokens":0}}}),
    );
    if sender.send(start).await.is_err() {
        return Ok(());
    }
    let mut decoder = Sse::default();
    let mut plain = Vec::new();
    let mut done = false;
    while !done {
        let chunk = tokio::select! {
            _ = sender.closed() => return Ok(()),
            result = upstream.chunk() => result.map_err(|error| if error.is_timeout() { "The upstream provider timed out while streaming" } else { "The upstream stream was interrupted by a network error" })?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if !sse {
            if plain.len() + chunk.len() > MAX_EVENT_BYTES {
                return Err("The upstream reply exceeded the size limit");
            }
            plain.extend_from_slice(&chunk);
            continue;
        }
        for data in decoder.push(&chunk)? {
            if data == b"[DONE]" {
                done = true;
                break;
            }
            let value = serde_json::from_slice(&data)
                .map_err(|_| "The upstream provider sent an invalid event")?;
            let (frames, finished) = if protocol == Protocol::OpenAiResponses {
                reply.responses(&value)?
            } else {
                (reply.chat(&value, protocol)?, false)
            };
            if !frames.is_empty() && sender.send(frames).await.is_err() {
                return Ok(());
            }
            if finished {
                done = true;
                break;
            }
        }
    }
    if !sse {
        let value = serde_json::from_slice(&plain)
            .map_err(|_| "The upstream provider sent an invalid reply")?;
        let frames = if protocol == Protocol::OpenAiResponses {
            reply.responses(&value)?.0
        } else {
            reply.chat(&value, protocol)?
        };
        if !frames.is_empty() && sender.send(frames).await.is_err() {
            return Ok(());
        }
    }
    let _ = sender.send(reply.finish(context)?).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_summary_is_emitted_as_a_visible_thinking_block() {
        let mut reply = Reply::default();
        let (frames, finished) = reply
            .responses(&json!({
                "type":"response.reasoning_summary_text.delta",
                "delta":"I will inspect the relevant files."
            }))
            .unwrap();

        assert!(!finished);
        assert!(frames.contains("\"type\":\"thinking\""));
        assert!(frames.contains("\"type\":\"thinking_delta\""));
        assert!(frames.contains("I will inspect the relevant files."));
    }

    #[test]
    fn encrypted_reasoning_is_not_emitted_as_user_visible_text() {
        let mut reply = Reply::default();
        let (frames, _) = reply
            .responses(&json!({
                "type":"response.completed",
                "response":{"status":"completed","output":[{
                    "type":"reasoning",
                    "encrypted_content":"opaque-private-payload"
                }]}
            }))
            .unwrap();

        assert!(!frames.contains("opaque-private-payload"));
        assert!(!frames.contains("thinking_delta"));
    }

    #[test]
    fn provider_error_events_are_not_treated_as_normal_stream_pauses() {
        let mut reply = Reply::default();
        assert!(reply
            .responses(&json!({"type":"error","error":{"message":"rate limited"}}))
            .is_err());
        assert!(reply.responses(&json!({"type":"response.failed"})).is_err());
        assert!(reply
            .responses(&json!({"type":"response.unknown.progress"}))
            .unwrap()
            .0
            .is_empty());
    }
}
