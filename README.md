# hero-sms-mcp

Rust CLI and MCP server for HeroSMS. One shared catalog and request builder cover 50 documented operations: 25 REST and 25 SMS-Activate compatibility operations, including SMS activations, rental, email activations, classifiers, prices and statistics.

This is an unofficial client. Calls go directly to the fixed `hero-sms.com` origin with the caller's API key. Mutating calls can spend real account balance. There is no simulation mode, automatic retry, server-owned key, or upstream URL override.

## Native CLI and stdio

Install Rust, then run `cargo build --locked --release`. The binary is `target/release/hero-sms-mcp`.

```sh
hero-sms-mcp list
hero-sms-mcp describe buyActivationBatch
hero-sms-mcp call getCountries
hero-sms-mcp call getActivationOtpLast --args '{"activationId":123}'
hero-sms-mcp stdio
hero-sms-mcp serve --port 3000
```

Set `HEROSMS_API_KEY` through your secret manager or process environment for CLI calls and stdio. No API-key argument flag is accepted. Stdio reads one JSON-RPC object per line and writes only responses to stdout. An MCP client configuration uses `hero-sms-mcp` as command and `["stdio"]` as arguments, with `HEROSMS_API_KEY` in that subprocess's environment.

CLI mutation requires `--yes`. For example, after inspecting the schema, a purchase uses `call buyActivationBatch --yes --args '{"body":{"service":"tg","country":2,"amount":1}}'`. This spends balance. MCP tools have truthful mutation annotations but no extra consent gate: the MCP client controls authorization.

`describe` prints the exact input schema. Parameters are flat arguments, JSON request payloads go under `body`. Query names such as `services[]` retain their documented spelling. Arrays serialize as repeated query parameters; `sort` serializes as deep-object keys such as `sort[id]`. Original operation IDs remain aliases, including opaque IDs replaced by meaningful tool names.

## HTTP transports

Native HTTP binds only `127.0.0.1`. Use the exact authority `127.0.0.1:3000`, not a different Host alias. Every remote call supplies its own `X-HeroSMS-API-Key` header. HTTP never reads `HEROSMS_API_KEY`. Initialize, ping and tools/list do not need credentials.

- Streamable HTTP: POST `/mcp` with `Content-Type: application/json`; clients should advertise `Accept: application/json, text/event-stream`. Responses are JSON, notifications receive 202. This stateless server has no resumable event stream or session ID; GET `/mcp` returns 405. Supported protocol versions: `2025-03-26`, `2025-06-18`. Set `MCP-Protocol-Version` on subsequent requests. Unknown header versions reject. Initialization negotiates one of the implemented versions, not a newer protocol.
- Legacy HTTP+SSE (native only): GET `/sse` with the key header, consume the `endpoint` event, then POST JSON-RPC to its `/messages?sessionId=...` URL with the same key header. Responses arrive as `message` events on the original stream. Use protocol `2024-11-05`. Sessions expire after five minutes, have bounded queues, and are isolated by credential. There are at most 128 sessions per process. Reconnect after expiry. Disconnected credentials can remain in memory until cleanup or expiry.

If a legacy SSE connection drops during an upstream call, the action may have completed even though its response cannot be recovered. Check activation history or account state before retrying a purchase or other mutation; reconnecting does not replay lost responses.

HTTP validates same-origin Origin headers when present. No browser CORS access is granted. Use a non-browser MCP client capable of custom headers. Request bodies are limited to 1 MiB; upstream responses to 4 MiB, upstream time to 30 seconds. Redirects are rejected. Transport errors omit request URLs and credentials. Output redacts literal key echoes; upstream data can still contain sensitive phone numbers and OTPs. Avoid logging tool outputs or credential headers in proxies and clients.

## Cloudflare Workers

The Worker uses the same Rust core and implements only modern HTTP `/mcp`. It does not implement legacy SSE. No account API key is stored in Worker secrets; callers supply `X-HeroSMS-API-Key` on each request.

```sh
rustup target add wasm32-unknown-unknown
cargo install worker-build --version 0.8.7 --locked
worker-build --release worker
wrangler dev
```

`wrangler.toml` has the prefixed Worker name `hero-sms-mcp`, a build command, and request observability disabled. Deploy only to your selected Cloudflare account after reviewing its access and logging settings. A public deployment accepts any caller-supplied HeroSMS key; add edge access control/rate limiting if needed. This project does not provision it or perform deployments. Do not put credentials in URLs. TLS is provided by the Worker host; the native listener is local HTTP and is not a public server.

## Contract provenance and license

`reference/catalog.json` contains extracted endpoint, parameter and validation facts, not an upstream implementation. It was generated from `docs/openapi.json` in [yikedu7/hero-sms-skill](https://github.com/yikedu7/hero-sms-skill) at commit `63f2c2c9923eda422cd518ae3c78d98b4dd3d005`. The declared official source is <https://hero-sms.com/docs/v1/openapi.json>. Direct retrieval of the official URL returned HTTP 403 during research; a separately fetched official reference visibly agreed with the examined contract, but this is not proof of byte equality or live API conformance. The third-party repository did not declare a license. Its full documentation and implementation are not included or claimed to be MIT.

To refresh facts, obtain and review the reference in a separate data directory, then run `python3 -I scripts/extract_catalog.py /absolute/reference/openapi.json reference/catalog.json`. Review all schema and mutation changes before building. This extractor resolves only local references, and no live upstream calls are made in tests.

Original project code is MIT licensed. HeroSMS trademarks and upstream documentation are not relicensed.

## Checks

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked -p hero-sms-mcp-worker --target wasm32-unknown-unknown
```

Tests cover the frozen six-case acceptance contract, all 50 builders and aliases, required arguments, selected range/enum/path and payload rules, an injected transport response, HTTP wire behavior, and native SSE credential isolation. Worker WASM compilation and JS/WASM packaging are separate checks; neither proves deployed runtime behavior. No real HeroSMS key, purchase, live upstream request or cloud deployment is needed or included.
