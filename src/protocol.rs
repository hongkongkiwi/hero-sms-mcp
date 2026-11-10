use crate::{
    api::{self, ApiRequest, ApiResponse},
    catalog,
};
use serde_json::{Value, json};
use std::future::Future;

pub const VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
pub struct RequestContext {
    api_key: Option<String>,
    modern_transport: bool,
}
impl RequestContext {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            api_key,
            modern_transport: false,
        }
    }

    pub fn modern_transport(mut self) -> Self {
        self.modern_transport = true;
        self
    }
}

pub fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code,"message":message}})
}

pub async fn dispatch(request: Value, context: &RequestContext) -> Option<Value> {
    dispatch_with(request, context, |request| async {
        #[cfg(feature = "native")]
        {
            crate::native::execute(request).await
        }
        #[cfg(not(feature = "native"))]
        {
            let _ = request;
            Err("No transport configured")
        }
    })
    .await
}

pub async fn dispatch_with<F, Fut>(
    request: Value,
    context: &RequestContext,
    execute: F,
) -> Option<Value>
where
    F: FnOnce(ApiRequest) -> Fut,
    Fut: Future<Output = Result<ApiResponse, &'static str>>,
{
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !request.is_object()
        || request["jsonrpc"] != "2.0"
        || !request["method"].is_string()
        || !(id.is_null() || id.is_string() || id.is_number())
    {
        return Some(error(Value::Null, -32600, "Invalid Request"));
    }
    // Notifications cannot initiate upstream spending or receive an RPC response.
    request.get("id")?;
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Some(error(id, -32602, "Invalid params"));
    }
    let result = match request["method"].as_str().unwrap() {
        "initialize" => {
            let Some(version) = params["protocolVersion"].as_str() else {
                return Some(error(id, -32602, "protocolVersion is required"));
            };
            let version = if VERSIONS.contains(&version)
                && (!context.modern_transport
                    || crate::security::modern_version_allowed(Some(version)))
            {
                version
            } else {
                "2025-06-18"
            };
            json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"hero-sms-mcp","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":catalog::operations().iter().map(|op| json!({
            "name":op.name,"description":format!("HeroSMS {} {}. {}",op.method,op.path,
                if op.mutating {"Changes account state and may spend balance."} else {"Reads HeroSMS data."}),
            "inputSchema":op.input_schema,
            "annotations":{"readOnlyHint":!op.mutating,"destructiveHint":op.mutating,
                "idempotentHint":!op.mutating,"openWorldHint":true}
        })).collect::<Vec<_>>()}),
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return Some(error(id, -32602, "Tool name is required"));
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let Some(key) = context.api_key.as_deref() else {
                return Some(error(id, -32602, "HeroSMS API key is required"));
            };
            match api::prepare_call(name, args, key) {
                Ok(prepared) => api::tool_result(execute(prepared).await, key),
                Err(message) => return Some(error(id, -32602, message)),
            }
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
