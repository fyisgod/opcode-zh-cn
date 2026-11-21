use super::*;

#[test]
fn routes_standard_and_prefixed_endpoints() {
    for (protocol, base, path) in [
        (
            Protocol::OpenAiChat,
            "https://example.com",
            "/v1/chat/completions",
        ),
        (
            Protocol::OpenAiChat,
            "https://example.com/tenant/v1",
            "/tenant/v1/chat/completions",
        ),
        (
            Protocol::OpenAiResponses,
            "https://example.com/v1/responses",
            "/v1/responses",
        ),
        (
            Protocol::Gemini,
            "https://example.com",
            "/v1beta/openai/chat/completions",
        ),
    ] {
        assert_eq!(endpoint(protocol, base).unwrap().path(), path);
    }
    assert!(endpoint(Protocol::OpenAiChat, "https://example.com/v1/responses").is_err());
}

async fn mock_upstream(
    protocol: Protocol,
    path: axum::http::Uri,
    headers: HeaderMap,
    body: Value,
) -> Response {
    assert_eq!(
        headers[header::AUTHORIZATION],
        "Bearer upstream-test-secret"
    );
    assert_eq!(body["model"], "test-model");
    let responses = protocol == Protocol::OpenAiResponses;
    let items = body[if responses { "input" } else { "messages" }]
        .as_array()
        .unwrap();
    assert_eq!(
        items[0]["role"],
        if responses { "developer" } else { "system" }
    );
    assert_eq!(
        items[0]["content"],
        "Top-level instruction\n\nLate system instruction"
    );
    let tool_result = items.iter().find(|item| {
        if responses {
            item["type"] == "function_call_output"
        } else {
            item["role"] == "tool"
        }
    });
    assert_eq!(
        path.path(),
        if responses {
            "/v1/responses"
        } else if protocol == Protocol::Gemini {
            "/v1beta/openai/chat/completions"
        } else {
            "/v1/chat/completions"
        }
    );
    if body["stream"] == false {
        return Json(if responses {
            json!({"object":"response","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"title"}]}]})
        } else {
            json!({"choices":[{"message":{"content":"title"},"finish_reason":"stop"}]})
        }).into_response();
    }
    let events = if let Some(result) = tool_result {
        if responses {
            assert_eq!(result["call_id"], "call_test");
            assert_eq!(result["output"], "file contents");
            assert!(items
                .iter()
                .any(|item| item["encrypted_content"] == "opaque-reasoning"));
            vec![
                json!({"type":"response.output_text.delta","delta":"done"}),
                json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
            ]
        } else {
            assert_eq!(result["tool_call_id"], "call_test");
            assert_eq!(result["content"], "file contents");
            if protocol == Protocol::Gemini {
                assert!(items
                    .iter()
                    .any(|item| item["tool_calls"][0]["extra_content"]["google"]
                        ["thought_signature"]
                        == "opaque-signature"));
            }
            vec![json!({"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]})]
        }
    } else if responses {
        assert_eq!(body["tools"][0]["name"], "Read");
        let function = json!({"type":"function_call","call_id":"call_test","name":"Read","arguments":"{\"file_path\":\"test.txt\"}"});
        vec![
            json!({"type":"response.output_text.delta","delta":"reading"}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_test","name":"Read","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"file_path\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":1,"delta":"\"test.txt\"}"}),
            json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"reasoning","encrypted_content":"opaque-reasoning"},function]}}),
        ]
    } else {
        assert_eq!(body["tools"][0]["function"]["name"], "Read");
        let mut call = json!({"index":0,"id":"call_test","function":{"name":"Read","arguments":"{\"file_path\":"}});
        if protocol == Protocol::Gemini {
            call["extra_content"] = json!({"google":{"thought_signature":"opaque-signature"}});
        }
        vec![
            json!({"choices":[{"delta":{"content":"reading"}}]}),
            json!({"choices":[{"delta":{"tool_calls":[call]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"test.txt\"}"}}]},"finish_reason":"tool_calls"}]}),
        ]
    };
    let wire: String = events
        .into_iter()
        .map(|event| format!("data: {event}\r\n\r\n"))
        .collect();
    ([(header::CONTENT_TYPE, "text/event-stream")], wire).into_response()
}

#[tokio::test]
async fn bridge_streams_three_protocols_and_replays_tool_results() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::Gemini,
    ] {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", upstream.local_addr().unwrap());
        let mock = Router::new().fallback(
            move |path: axum::http::Uri, headers: HeaderMap, Json(body): Json<Value>| async move {
                mock_upstream(protocol, path, headers, body).await
            },
        );
        let task = tokio::spawn(async move {
            axum::serve(upstream, mock).await.unwrap();
        });
        let bridge = ProviderBridge::start(
            protocol,
            &base,
            "test-model",
            Auth::Bearer,
            "upstream-test-secret",
        )
        .await
        .unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/v1/messages", bridge.base_url);
        let mut body = json!({"stream":true,"model":"requested-model","max_tokens":512,
            "system":"Top-level instruction","messages":[{"role":"user","content":"Read test.txt"},
            {"role":"system","content":"Late system instruction"}],"tools":[{"name":"Read","input_schema":{"type":"object"}}]});
        assert_eq!(
            client.post(&url).json(&body).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let first = client
            .post(&url)
            .bearer_auth(&bridge.token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(
            first.contains("reading")
                && first.contains("input_json_delta")
                && first.contains("test.txt")
        );
        assert!(first.contains("\"stop_reason\":\"tool_use\""));
        assert!(!first.contains("opaque-"));
        body["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":[{"type":"tool_use","id":"call_test","name":"Read","input":{"file_path":"test.txt"}}]}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"call_test","content":"file contents"}]}),
        ]);
        let second = client
            .post(&url)
            .bearer_auth(&bridge.token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(second.contains("done") && second.contains("message_stop"));
        body["stream"] = json!(false);
        let plain: Value = client
            .post(&url)
            .bearer_auth(&bridge.token)
            .json(&body)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(plain["content"][0]["text"], "title");
        task.abort();
    }
}
