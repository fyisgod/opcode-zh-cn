use serde_json::{json, Value};

use super::ToolContext;
use crate::commands::model_provider::Protocol;

type Translation<T> = Result<T, &'static str>;

fn required<'a>(value: &'a Value, key: &str) -> Translation<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Missing message or tool field")
}

fn image_url(block: &Value) -> Translation<String> {
    let source = &block["source"];
    match source["type"].as_str() {
        Some("base64") => Ok(format!(
            "data:{};base64,{}",
            required(source, "media_type")?,
            required(source, "data")?
        )),
        Some("url") => Ok(required(source, "url")?.to_owned()),
        _ => Err("Unsupported image format"),
    }
}

fn tool_result(block: &Value) -> Translation<(String, Vec<String>)> {
    let mut text = Vec::new();
    let mut images = Vec::new();
    match &block["content"] {
        Value::String(value) => text.push(value.clone()),
        Value::Array(parts) => {
            for part in parts {
                match part["type"].as_str() {
                    Some("text") => text.push(part["text"].as_str().unwrap_or_default().to_owned()),
                    Some("image") => images.push(image_url(part)?),
                    _ => return Err("Unsupported content in tool result"),
                }
            }
        }
        Value::Null => (),
        _ => return Err("Invalid tool result"),
    }
    let content = text.join("\n");
    let content = if block["is_error"] == true {
        format!("Tool error: {content}")
    } else {
        content
    };
    Ok((content, images))
}

fn system_text(system: &Value) -> String {
    match system {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .filter(|text| !text.starts_with("x-anthropic-billing-header:"))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn content_part(role: &str, text: &str, responses: bool) -> Value {
    if responses {
        let kind = if role == "assistant" {
            "output_text"
        } else {
            "input_text"
        };
        json!({"type":kind,"text":text})
    } else {
        json!({"type":"text","text":text})
    }
}

fn image_part(url: String, responses: bool) -> Value {
    if responses {
        json!({"type":"input_image","image_url":url,"detail":"auto"})
    } else {
        json!({"type":"image_url","image_url":{"url":url}})
    }
}

fn flush_message(target: &mut Vec<Value>, role: &str, content: &mut Vec<Value>) {
    if !content.is_empty() {
        target.push(json!({"role":role,"content":std::mem::take(content)}));
    }
}

fn messages(body: &Value, protocol: Protocol, context: &ToolContext) -> Translation<Vec<Value>> {
    let responses = protocol == Protocol::OpenAiResponses;
    let mut result = Vec::new();
    let messages = body["messages"].as_array().ok_or("Missing messages")?;
    let mut system = system_text(&body["system"]);
    // Claude Code can append system instructions after the user message.
    for message in messages
        .iter()
        .filter(|message| message["role"] == "system")
    {
        let text = system_text(&message["content"]);
        if !text.is_empty() {
            if !system.is_empty() {
                system.push_str("\n\n");
            }
            system.push_str(&text);
        }
    }
    if !system.is_empty() {
        result.push(json!({"role":if responses {"developer"} else {"system"},"content":system}));
    }
    for message in messages {
        let role = required(message, "role")?;
        if role == "system" {
            continue;
        }
        if !matches!(role, "user" | "assistant") {
            return Err("Unsupported message role");
        }
        let blocks = match &message["content"] {
            Value::String(text) => {
                result.push(json!({"role":role,"content":text}));
                continue;
            }
            Value::Array(blocks) => blocks,
            _ => return Err("Invalid message content"),
        };
        let mut parts = Vec::new();
        let mut calls = Vec::new();
        // Chat Completions requires all tool replies immediately after the assistant's tool calls.
        let mut tool_replies = Vec::new();
        for block in blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(text) = block["text"].as_str().filter(|s| !s.is_empty()) {
                        parts.push(content_part(role, text, responses));
                    }
                }
                Some("image") if role == "user" => {
                    parts.push(image_part(image_url(block)?, responses))
                }
                Some("tool_use") if role == "assistant" => {
                    let id = required(block, "id")?;
                    let name = required(block, "name")?;
                    let args = block["input"].to_string();
                    if responses {
                        flush_message(&mut result, role, &mut parts);
                        if let Some(Value::Array(items)) = context.get(id) {
                            result.extend(items);
                        }
                        result.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":args}));
                    } else {
                        let mut call = json!({"id":id,"type":"function","function":{"name":name,"arguments":args}});
                        if protocol == Protocol::Gemini {
                            if let Some(extra) = context.get(id) {
                                call["extra_content"] = extra;
                            }
                        }
                        calls.push(call);
                    }
                }
                Some("tool_result") if role == "user" => {
                    let id = required(block, "tool_use_id")?;
                    let (text, images) = tool_result(block)?;
                    if responses {
                        flush_message(&mut result, role, &mut parts);
                        let output = if images.is_empty() {
                            json!(text)
                        } else {
                            let mut output = vec![content_part("user", &text, true)];
                            output.extend(images.into_iter().map(|url| image_part(url, true)));
                            json!(output)
                        };
                        result.push(
                            json!({"type":"function_call_output","call_id":id,"output":output}),
                        );
                    } else {
                        tool_replies.push(json!({"role":"tool","tool_call_id":id,"content":text}));
                        parts.extend(images.into_iter().map(|url| image_part(url, false)));
                    }
                }
                Some("thinking" | "redacted_thinking") => (),
                _ => {
                    return Err(
                        "This provider does not support a content block in this conversation",
                    )
                }
            }
        }
        result.extend(tool_replies);
        if calls.is_empty() {
            flush_message(&mut result, role, &mut parts);
        } else {
            let content = if parts.is_empty() {
                Value::Null
            } else {
                json!(parts)
            };
            result.push(json!({"role":role,"content":content,"tool_calls":calls}));
        }
    }
    Ok(result)
}

pub(super) fn translate(
    body: &Value,
    protocol: Protocol,
    model: &str,
    official_openai: bool,
    context: &ToolContext,
) -> Translation<Value> {
    let responses = protocol == Protocol::OpenAiResponses;
    let messages = messages(body, protocol, context)?;
    let mut result = json!({"model":model,"stream":true});
    if responses {
        result["input"] = json!(messages);
        result["store"] = json!(false);
        if official_openai {
            result["include"] = json!(["reasoning.encrypted_content"]);
        }
    } else {
        result["messages"] = json!(messages);
    }
    if let Some(limit) = body["max_tokens"].as_u64() {
        let name = if responses {
            "max_output_tokens"
        } else if official_openai {
            "max_completion_tokens"
        } else {
            "max_tokens"
        };
        // Compatible chat gateways often reject Claude's larger default output budget.
        result[name] = json!(if name == "max_tokens" {
            limit.min(8192)
        } else {
            limit
        });
    }
    if let Some(tools) = body["tools"].as_array().filter(|tools| !tools.is_empty()) {
        let normalized = tools.iter().map(|tool| {
            let Some(schema) = tool["input_schema"].as_object() else { return Ok(None) };
            let mut info = json!({"name":required(tool,"name")?,"description":tool["description"].as_str().unwrap_or_default(),"parameters":schema});
            if responses {
                info["type"] = json!("function");
                info["strict"] = json!(false);
                Ok(Some(info))
            } else {
                Ok(Some(json!({"type":"function","function":info})))
            }
        }).collect::<Translation<Vec<_>>>()?.into_iter().flatten().collect::<Vec<_>>();
        if !normalized.is_empty() {
            result["tools"] = json!(normalized);
            let choice = &body["tool_choice"];
            match choice["type"].as_str() {
                Some("any") => result["tool_choice"] = json!("required"),
                Some("none") => result["tool_choice"] = json!("none"),
                Some("tool") => {
                    let name = required(choice, "name")?;
                    result["tool_choice"] = if responses {
                        json!({"type":"function","name":name})
                    } else {
                        json!({"type":"function","function":{"name":name}})
                    };
                }
                _ => (),
            }
            if choice["disable_parallel_tool_use"] == true && protocol != Protocol::Gemini {
                result["parallel_tool_calls"] = json!(false);
            }
        } else if matches!(body["tool_choice"]["type"].as_str(), Some("any" | "tool")) {
            return Err("This provider cannot execute the requested hosted tool");
        }
    }
    Ok(result)
}
