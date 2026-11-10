use crate::catalog;
use serde_json::Value;
use url::Url;

// Requests contain credentials and intentionally do not implement Debug or Serialize.
pub struct ApiRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Value>,
}

pub fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 4096 && key.bytes().all(|b| (33..=126).contains(&b))
}

pub fn prepare_call(
    operation: &str,
    args: Value,
    api_key: &str,
) -> Result<ApiRequest, &'static str> {
    let op = catalog::find(operation).ok_or("Unknown operation")?;
    if !op.valid(&args) {
        return Err("Arguments do not match the operation schema");
    }
    if !valid_key(api_key) {
        return Err("A valid HeroSMS API key is required");
    }
    let compat = op.path.starts_with("?action=");
    let mut path = op.path.clone();
    for param in &op.parameters {
        if param["in"] == "path" {
            let name = param["name"].as_str().unwrap();
            path = path.replace(&format!("{{{name}}}"), &scalar(&args[name]));
        }
    }
    let mut url = if compat {
        Url::parse("https://hero-sms.com/stubs/handler_api.php").unwrap()
    } else {
        Url::parse(&format!("https://hero-sms.com/api/v1{path}")).unwrap()
    };
    let mut headers = Vec::new();
    if compat {
        url.query_pairs_mut()
            .append_pair("action", op.path.trim_start_matches("?action="))
            .append_pair("api_key", api_key);
    } else {
        headers.push(("Authorization".into(), format!("ApiKey {api_key}")));
    }
    for param in &op.parameters {
        let name = param["name"].as_str().unwrap();
        if param["in"] != "query" {
            continue;
        }
        let Some(value) = args.get(name) else {
            continue;
        };
        match value {
            Value::Array(items) => {
                for item in items {
                    url.query_pairs_mut().append_pair(name, &scalar(item));
                }
            }
            Value::Object(fields) => {
                for (field, value) in fields {
                    url.query_pairs_mut()
                        .append_pair(&format!("{name}[{field}]"), &scalar(value));
                }
            }
            _ => {
                url.query_pairs_mut().append_pair(name, &scalar(value));
            }
        }
    }
    let body = args.get("body").cloned();
    if body.is_some() {
        headers.push(("Content-Type".into(), "application/json".into()));
    }
    Ok(ApiRequest {
        method: op.method.clone(),
        url: url.into(),
        headers,
        body,
    })
}

fn scalar(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn compat_error(text: &str) -> bool {
    let code = text.trim().split(':').next().unwrap_or_default();
    code.starts_with("BAD_")
        || code.starts_with("ERROR_")
        || code.starts_with("NO_")
        || matches!(
            code,
            "WRONG_SERVICE"
                | "WRONG_ACTIVATION_ID"
                | "WRONG_STATUS"
                | "BANNED"
                | "SQL_ERROR"
                | "EARLY_CANCEL_DENIED"
                | "OPERATORS_NOT_FOUND"
                | "RENT_ACTIVATION_NOT_FOUND"
                | "INVALID_PHONE"
                | "ACTION_NOT_FOUND"
        )
}

pub struct ApiResponse {
    pub status: u16,
    pub text: String,
}

pub fn tool_result(response: Result<ApiResponse, &'static str>, key: &str) -> Value {
    match response {
        Ok(response) => serde_json::json!({
            "content": [{"type": "text", "text": response.text.replace(key, "[REDACTED]")}],
            "isError": !(200..300).contains(&response.status) || compat_error(&response.text)
        }),
        Err(message) => {
            serde_json::json!({"content": [{"type": "text", "text": message}], "isError": true})
        }
    }
}
