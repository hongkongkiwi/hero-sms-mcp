use futures_util::{
    StreamExt,
    future::{Either, select},
};
use hero_sms_mcp::{
    MAX_BODY, MAX_RESPONSE,
    api::{ApiRequest, ApiResponse},
    protocol::{self, RequestContext},
    security,
};
use std::time::Duration;
use worker::*;

async fn read_limited(
    mut stream: ByteStream,
    limit: usize,
) -> std::result::Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Transport failed")?;
        if bytes.len() + chunk.len() > limit {
            return Err("Body too large");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn execute(request: ApiRequest) -> std::result::Result<ApiResponse, &'static str> {
    let headers = Headers::new();
    for (name, value) in request.headers {
        headers
            .set(&name, &value)
            .map_err(|_| "Upstream transport failed")?;
    }
    let mut init = RequestInit::new();
    init.with_method(match request.method.as_str() {
        "GET" => Method::Get,
        "POST" => Method::Post,
        "DELETE" => Method::Delete,
        _ => return Err("Unsupported upstream method"),
    })
    .with_headers(headers)
    .with_redirect(RequestRedirect::Manual);
    if let Some(body) = request.body {
        init.with_body(Some(body.to_string().into()));
    }
    let request =
        Request::new_with_init(&request.url, &init).map_err(|_| "Upstream transport failed")?;
    let controller = AbortController::default();
    let signal = controller.signal();
    let work = async {
        let mut response = Fetch::Request(request)
            .send_with_signal(&signal)
            .await
            .map_err(|_| "Upstream transport failed")?;
        let status = response.status_code();
        if (300..400).contains(&status) {
            return Err("Upstream redirect rejected");
        }
        let bytes = read_limited(
            response.stream().map_err(|_| "Upstream transport failed")?,
            MAX_RESPONSE,
        )
        .await?;
        Ok(ApiResponse {
            status,
            text: String::from_utf8_lossy(&bytes).into_owned(),
        })
    };
    match select(
        Box::pin(work),
        Box::pin(Delay::from(Duration::from_secs(30))),
    )
    .await
    {
        Either::Left((result, _)) => result,
        Either::Right(_) => {
            controller.abort();
            Err("Upstream request timed out")
        }
    }
}

#[event(fetch)]
pub async fn fetch(mut request: Request, _env: Env, _ctx: Context) -> Result<Response> {
    let url = request.url()?;
    if url.path() != "/mcp" {
        return Response::error("Not found", 404);
    }
    if request.method() != Method::Post {
        return Ok(Response::empty()?.with_status(405));
    }
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    if !security::origin_allowed(
        request.headers().get("origin")?.as_deref(),
        authority,
        url.scheme(),
    ) {
        return Response::error("Forbidden", 403);
    }
    if !security::modern_version_allowed(request.headers().get("mcp-protocol-version")?.as_deref())
    {
        return Response::error("Unsupported protocol version", 400);
    }
    if request
        .headers()
        .get("content-type")?
        .is_none_or(|v| v.split(';').next() != Some("application/json"))
    {
        return Response::error("Expected application/json", 415);
    }
    let key = request.headers().get("x-herosms-api-key")?;
    if key
        .as_deref()
        .is_some_and(|k| !hero_sms_mcp::api::valid_key(k))
    {
        return Response::error("Invalid credential", 401);
    }
    let body = match select(
        Box::pin(read_limited(request.stream()?, MAX_BODY)),
        Box::pin(Delay::from(Duration::from_secs(30))),
    )
    .await
    {
        Either::Left((Ok(body), _)) => body,
        _ => return Response::error("Request body rejected", 413),
    };
    let response = match serde_json::from_slice(&body) {
        Ok(value) => {
            protocol::dispatch_with(value, &RequestContext::new(key).modern_transport(), execute)
                .await
        }
        Err(_) => Some(protocol::error(
            serde_json::Value::Null,
            -32700,
            "Parse error",
        )),
    };
    let response = match response {
        Some(value) => Response::from_json(&value)?,
        None => Response::empty()?.with_status(202),
    };
    let headers = response.headers().clone();
    headers.set("Cache-Control", "no-store")?;
    Ok(response.with_headers(headers))
}
