use hero_sms_mcp::api::{ApiRequest, prepare_call};
use hero_sms_mcp::protocol::{RequestContext, dispatch};
use serde_json::{Value, json};
use url::Url;

const KEY: &str = "acceptance-only-not-a-real-key";
const KNOWN_OPERATIONS: &[&str] = &["getBalance", "getCountries"];

fn request(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 17, "method": method, "params": params})
}

async fn reply(method: &str, params: Value, key: Option<&str>) -> Value {
    let context = RequestContext::new(key.map(str::to_owned));
    let response = dispatch(request(method, params), &context)
        .await
        .expect("requests with an id must receive a response");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 17);
    assert_ne!(
        response.get("result").is_some(),
        response.get("error").is_some()
    );
    assert!(
        !response.to_string().contains(KEY),
        "response leaked the API key"
    );
    response
}

fn assert_rejected(response: &Value) {
    assert!(
        response["error"].is_object() || response["result"]["isError"] == true,
        "expected a structured rejection, got {response}"
    );
}

async fn tools() -> Vec<Value> {
    let response = reply("tools/list", json!({}), None).await;
    assert!(response.get("error").is_none(), "{response}");
    response["result"]["tools"]
        .as_array()
        .expect("tools/list must return a tools array")
        .clone()
}

fn assert_no_auth_schema(value: &Value) {
    match value {
        Value::Object(object) => {
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for name in properties.keys() {
                    assert!(
                        !matches!(
                            name.as_str(),
                            "api_key" | "apiKey" | "Authorization" | "authorization"
                        ),
                        "credentials must not be public tool arguments: {name}"
                    );
                }
            }
            for child in object.values() {
                assert_no_auth_schema(child);
            }
        }
        Value::Array(array) => array.iter().for_each(assert_no_auth_schema),
        _ => {}
    }
}

fn assert_safe_request(request: &ApiRequest) {
    let url = Url::parse(&request.url).expect("request URL must be absolute");
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("hero-sms.com"));
    assert_eq!(url.port_or_known_default(), Some(443));
    assert_eq!(url.username(), "");
    assert!(url.password().is_none());
    assert!(url.fragment().is_none());
    assert!(!url.path().contains(KEY));
    assert!(!request.method.is_empty());

    let authorization: Vec<_> = request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .collect();
    let query: Vec<_> = url.query_pairs().collect();
    let compat_keys: Vec<_> = query.iter().filter(|(name, _)| name == "api_key").collect();
    let body_key = request.body.as_ref().and_then(|body| body.get("api_key"));
    if authorization.is_empty() {
        assert_eq!(compat_keys.len() + usize::from(body_key.is_some()), 1);
        if let Some((_, value)) = compat_keys.first() {
            assert_eq!(value.as_ref(), KEY);
        }
        if let Some(value) = body_key {
            assert_eq!(value, KEY);
        }
    } else {
        assert_eq!(authorization.len(), 1);
        assert_eq!(authorization[0].1, format!("ApiKey {KEY}"));
        assert!(compat_keys.is_empty());
        assert!(body_key.is_none());
    }
    for (name, value) in &query {
        if value.contains(KEY) {
            assert_eq!(name, "api_key");
        }
    }
    for (name, value) in &request.headers {
        if value.contains(KEY) {
            assert!(name.eq_ignore_ascii_case("authorization"));
            assert_eq!(value, &format!("ApiKey {KEY}"));
        }
    }
    if let Some(body) = &request.body {
        let mut without_key = body.clone();
        if let Some(object) = without_key.as_object_mut() {
            object.remove("api_key");
        }
        assert!(!without_key.to_string().contains(KEY));
    }
}

#[tokio::test]
async fn discovery_and_ping_work_without_credentials() {
    let initialized = reply(
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "acceptance", "version": "0.0.0"}
        }),
        None,
    )
    .await;
    assert!(initialized.get("error").is_none(), "{initialized}");
    assert!(initialized["result"]["protocolVersion"].is_string());
    assert!(initialized["result"]["capabilities"].is_object());
    assert!(initialized["result"]["serverInfo"]["name"].is_string());
    assert_eq!(reply("ping", json!({}), None).await["result"], json!({}));

    let catalog = tools().await;
    for operation in KNOWN_OPERATIONS {
        assert!(catalog.iter().any(|tool| tool["name"] == *operation));
    }
    for tool in catalog {
        assert!(tool["name"].is_string());
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_no_auth_schema(&tool["inputSchema"]);
    }
}

#[tokio::test]
async fn tool_calls_require_context_credentials_and_reject_unknown_tools() {
    for operation in KNOWN_OPERATIONS {
        let response = reply(
            "tools/call",
            json!({"name": operation, "arguments": {}}),
            None,
        )
        .await;
        assert_rejected(&response);
    }
    let response = reply(
        "tools/call",
        json!({"name": "acceptanceUnknownOperation", "arguments": {}}),
        Some(KEY),
    )
    .await;
    assert_rejected(&response);
}

#[tokio::test]
async fn json_rpc_reports_standard_errors_and_preserves_ids() {
    let context = RequestContext::new(None);
    for invalid in [
        Value::Null,
        json!([]),
        json!({"id": 17, "method": "ping"}),
        json!({"jsonrpc": "1.0", "id": 17, "method": "ping"}),
        json!({"jsonrpc": "2.0", "id": 17, "method": 7}),
    ] {
        let response = dispatch(invalid, &context)
            .await
            .expect("invalid request response");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["error"]["code"], -32600);
        assert!(response.get("result").is_none());
    }
    let unknown = reply("acceptance/unknown", json!({}), None).await;
    assert_eq!(unknown["error"]["code"], -32601);
    let missing_name = reply("tools/call", json!({"arguments": {}}), Some(KEY)).await;
    assert_eq!(missing_name["error"]["code"], -32602);
    let response = dispatch(
        json!({"jsonrpc": "2.0", "id": "client-id", "method": "ping"}),
        &context,
    )
    .await
    .expect("string id response");
    assert_eq!(response["id"], "client-id");
}

#[tokio::test]
async fn notifications_never_receive_responses_even_when_the_method_fails() {
    let context = RequestContext::new(None);
    for (method, params) in [
        ("ping", json!({})),
        ("notifications/initialized", json!({})),
        ("acceptance/unknown", json!({})),
        ("tools/call", json!({"name": "getBalance", "arguments": {}})),
    ] {
        assert!(
            dispatch(
                json!({"jsonrpc": "2.0", "method": method, "params": params}),
                &context,
            )
            .await
            .is_none(),
            "notification {method} received a response"
        );
    }
}

#[test]
fn verified_read_operations_build_only_fixed_host_authenticated_requests() {
    for operation in KNOWN_OPERATIONS {
        let request = prepare_call(operation, json!({}), KEY)
            .unwrap_or_else(|_| panic!("{operation} must accept empty arguments"));
        assert_safe_request(&request);
    }
    assert!(prepare_call("acceptanceUnknownOperation", json!({}), KEY).is_err());
}

#[tokio::test]
async fn every_catalog_operation_rejects_transport_and_credential_overrides() {
    let overrides = [
        json!({"api_key": "caller-key"}),
        json!({"apiKey": "caller-key"}),
        json!({"Authorization": "ApiKey caller-key"}),
        json!({"authorization": "ApiKey caller-key"}),
        json!({"headers": {"Authorization": "ApiKey caller-key"}}),
        json!({"base_url": "https://attacker.invalid"}),
        json!({"baseUrl": "https://attacker.invalid"}),
        json!({"url": "https://attacker.invalid"}),
    ];
    for tool in tools().await {
        let name = tool["name"].as_str().expect("tool name");
        for args in &overrides {
            assert!(
                prepare_call(name, args.clone(), KEY).is_err(),
                "{name} accepted caller-controlled authentication or transport: {args}"
            );
        }
        let required = tool["inputSchema"]["required"].as_array();
        if required.is_some_and(|fields| !fields.is_empty()) {
            assert!(
                prepare_call(name, json!({}), KEY).is_err(),
                "{name} accepted arguments missing its advertised required fields"
            );
        }
    }
}
