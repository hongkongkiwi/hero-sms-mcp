use hero_sms_mcp::{
    api::{self, ApiResponse, prepare_call},
    catalog,
    protocol::{self, RequestContext},
};
use serde_json::{Value, json};

fn fixture(schema: &Value) -> Value {
    if let Some(value) = schema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return value.clone();
    }
    if let Some(schema) = schema
        .get("oneOf")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        return fixture(schema);
    }
    let kind = schema["type"].as_str().or_else(|| {
        schema["type"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str)
    });
    match kind {
        Some("object") => {
            let mut object = serde_json::Map::new();
            for name in schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                object.insert(name.into(), fixture(&schema["properties"][name]));
            }
            Value::Object(object)
        }
        Some("integer" | "number") => schema.get("minimum").cloned().unwrap_or(json!(1)),
        Some("boolean") => json!(false),
        Some("array") => json!([fixture(&schema["items"])]),
        Some("string") => match schema["format"].as_str() {
            Some("date-time") => json!("2026-11-10T00:00:00Z"),
            Some("date") => json!("2026-11-10"),
            _ => json!("tg"),
        },
        _ => panic!("unhandled schema {schema}"),
    }
}
#[test]
fn all_fifty_operations_build_and_aliases_match() {
    assert_eq!(catalog::operations().len(), 50);
    assert_eq!(
        catalog::operations()
            .iter()
            .filter(|op| op.path.starts_with('?'))
            .count(),
        25
    );
    for op in catalog::operations() {
        let args = fixture(&op.input_schema);
        let request = prepare_call(&op.name, args.clone(), "synthetic")
            .unwrap_or_else(|error| panic!("{} {args}: {error}", op.name));
        let alias = prepare_call(&op.operation_id, args, "synthetic").unwrap();
        assert_eq!(request.url, alias.url);
        assert_eq!(request.method, alias.method);
        assert_eq!(request.body, alias.body);
        let url = url::Url::parse(&request.url).unwrap();
        assert_eq!(url.host_str(), Some("hero-sms.com"));
        assert_eq!(request.method, op.method);
        for field in op.input_schema["required"].as_array().unwrap() {
            let mut incomplete = fixture(&op.input_schema);
            incomplete
                .as_object_mut()
                .unwrap()
                .remove(field.as_str().unwrap());
            assert!(prepare_call(&op.name, incomplete, "synthetic").is_err());
        }
    }
}
#[test]
fn typed_bodies_paths_arrays_and_compat_posts() {
    let args = json!({"body":{"service":"tg","country":2,"amount":1}});
    let request = prepare_call("buyActivationBatch", args.clone(), "synthetic").unwrap();
    assert_eq!(request.body, Some(args["body"].clone()));
    for amount in [json!(0), json!(11), json!("1")] {
        assert!(
            prepare_call(
                "buyActivationBatch",
                json!({"body":{"service":"tg","country":2,"amount":amount}}),
                "synthetic"
            )
            .is_err()
        );
    }
    assert!(
        prepare_call(
            "getActivationOtpLast",
            json!({"activationId":"../secret"}),
            "synthetic"
        )
        .is_err()
    );
    let request=prepare_call("getActivationHistory",json!({"from":"2026-11-10T00:00:00Z","to":"2026-11-11T00:00:00Z","services[]":["tg","wa"],"sort":{"id":"asc"}}),"synthetic").unwrap();
    let url = url::Url::parse(&request.url).unwrap();
    let query: Vec<_> = url.query_pairs().collect();
    assert!(query.iter().any(|(k, v)| k == "sort[id]" && v == "asc"));
    assert_eq!(query.iter().filter(|(k, _)| k == "services[]").count(), 2);
    let request = prepare_call("reactivate", json!({"id":123,"duration":24}), "synthetic").unwrap();
    assert_eq!(request.method, "POST");
    assert!(request.url.contains("action=reactivate"));
    assert!(request.url.contains("id=123"));
    assert!(request.body.is_none());
    assert!(prepare_call("setStatus", json!({"id":123,"status":1}), "synthetic").is_err());
}
#[tokio::test]
async fn injected_transport_forwards_and_redacts() {
    let context = RequestContext::new(Some("synthetic".into()));
    let result=protocol::dispatch_with(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"getBalance","arguments":{}}}),&context,|request|async move {
        assert!(request.url.contains("api_key=synthetic"));
        Ok(ApiResponse {status:200,text:"BAD_KEY:synthetic".into()})
    }).await.unwrap();
    assert_eq!(result["result"]["isError"], true);
    assert!(!result.to_string().contains("synthetic"));
    for text in ["STATUS_WAIT_CODE", "STATUS_OK:1234", "ACCESS_BALANCE:5"] {
        assert_eq!(
            api::tool_result(
                Ok(ApiResponse {
                    status: 200,
                    text: text.into()
                }),
                "synthetic"
            )["isError"],
            false
        );
    }
}
#[tokio::test]
async fn native_http_and_legacy_sse_isolation() {
    use futures_util::StreamExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        axum::serve(listener, hero_sms_mcp::native::router(port))
            .await
            .unwrap()
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let ping = json!({"jsonrpc":"2.0","id":3,"method":"ping"});
    let initialized: Value = client
        .post(format!("{base}/mcp"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"wire-test","version":"1"}}}))
        .send().await.unwrap().json().await.unwrap();
    let version = initialized["result"]["protocolVersion"].as_str().unwrap();
    assert!(matches!(version, "2025-03-26" | "2025-06-18"));
    let subsequent = client
        .post(format!("{base}/mcp"))
        .header("mcp-protocol-version", version)
        .json(&ping)
        .send()
        .await
        .unwrap();
    assert_eq!(subsequent.status(), 200);
    assert_eq!(
        subsequent.json::<Value>().await.unwrap()["result"],
        json!({})
    );
    let response = client
        .post(format!("{base}/mcp"))
        .json(&ping)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.json::<Value>().await.unwrap()["result"], json!({}));
    assert_eq!(
        client
            .get(format!("{base}/mcp"))
            .send()
            .await
            .unwrap()
            .status(),
        405
    );
    assert_eq!(
        client
            .post(format!("{base}/mcp"))
            .header("origin", "https://attacker.invalid")
            .json(&ping)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(format!("{base}/mcp"))
            .header(
                "origin",
                reqwest::header::HeaderValue::from_bytes(&[0xff]).unwrap()
            )
            .json(&ping)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(format!("{base}/mcp"))
            .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    let response = client
        .get(format!("{base}/sse"))
        .header("x-herosms-api-key", "first-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut stream = response.bytes_stream();
    let first = String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap();
    let endpoint = first
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    assert!(first.contains("event: endpoint"));
    assert_eq!(
        client
            .post(format!("{base}{endpoint}"))
            .header("x-herosms-api-key", "second-key")
            .json(&ping)
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .post(format!("{base}{endpoint}"))
            .header("x-herosms-api-key", "first-key")
            .json(&ping)
            .send()
            .await
            .unwrap()
            .status(),
        202
    );
    let message = String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(message.contains("event: message"));
    assert!(message.contains("\"id\":3"));
    server.abort();
}
