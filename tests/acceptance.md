# Frozen acceptance contract

Status: authored before implementation, not executed, baseline qualification blocked. The assigned worktree contained only `.git` when inspected on 2026-11-10. No Cargo manifest, implementation, existing tests, or project-local instructions were present. Root has not cleared test execution. No build or test command was run; no exit status or failing test name exists yet. These tests are not a qualified failing baseline.

Only `tests/acceptance_contract.rs` and this file are owned by the test author. Both are frozen on handoff. Implementation must not weaken or skip their assertions. A genuine contract conflict goes to the coordinator.

## Required public interface

Crate import name: `hero_sms_mcp`.

- `protocol::RequestContext::new(api_key: Option<String>) -> RequestContext`.
- `protocol::dispatch(request: serde_json::Value, context: &RequestContext)` is async and returns `Option<serde_json::Value>`.
- `api::prepare_call(operation: &str, args: serde_json::Value, api_key: &str) -> Result<ApiRequest, E>`. Tests place no trait requirements on `E`.
- Public `api::ApiRequest` fields: `method: String`, `url: String`, `headers: Vec<(String, String)>`, `body: Option<serde_json::Value>`.
- Test dependencies: `serde_json`, `url`, and Tokio with the `macros` and runtime features supporting `#[tokio::test]` (for example `rt-multi-thread`). No test executes `ApiRequest`.

## Cases pinned

1. `discovery_and_ping_work_without_credentials`: initialize, ping, and tools/list succeed with no key. Initialization returns protocolVersion, capabilities, and serverInfo.name. Ping returns an empty object. Tools include the verified operationIds `getBalance` and `getCountries`; all advertised tool schemas are object schemas and expose no api_key, apiKey, Authorization, or authorization properties, including nested schemas.
2. `tool_calls_require_context_credentials_and_reject_unknown_tools`: calls to both known read operations reject without a context key; an unknown operation rejects with a synthetic key. Rejection can be a JSON-RPC error or MCP tool result with isError=true. Responses must not echo the synthetic credential.
3. `json_rpc_reports_standard_errors_and_preserves_ids`: invalid request objects produce -32600, unknown methods -32601, and a tools/call missing name -32602. Integer and string ids are preserved for valid request envelopes. No parse-error assertion exists because dispatch receives an already parsed Value.
4. `notifications_never_receive_responses_even_when_the_method_fails`: ping, notifications/initialized, unknown-method, and unauthenticated tools/call notifications return None.
5. `verified_read_operations_build_only_fixed_host_authenticated_requests`: getBalance/getCountries accept empty argument objects; unknown operations reject. Built requests use HTTPS, exact host hero-sms.com, port 443, no URL userinfo or fragment. Authentication is either one Authorization header with exact value `ApiKey <key>` or one compat api_key query/body field. A REST header cannot coexist with a compat key. Credentials cannot appear elsewhere in the path, query, headers, or body.
6. `every_catalog_operation_rejects_transport_and_credential_overrides`: every advertised operation rejects top-level api_key, apiKey, Authorization, authorization, headers, base_url, baseUrl, and url overrides. Operations advertising required fields reject an empty argument object.

There are six tests. All inputs are deterministic and local. The synthetic credential is `acceptance-only-not-a-real-key`, not a usable credential. No supported tools/call is dispatched with a key. The only dispatch carrying a key targets an unknown tool or omits the required tool name. Supported unauthenticated calls must stop before transport. No purchases or network calls are intended or authorized.

## Resolved assumptions and coverage limits

- getBalance and getCountries are the only independently verified operation names available at authoring time. Both are assumed to require no caller arguments. Other catalog names and valid required-argument fixtures remain to be coordinated by root with the writer.
- Arguments use flat query/path parameters and an optional body object, per the root clarification. No wrapper format is assumed.
- Credentials are context-owned, not tool arguments. Overrides reject rather than being silently ignored. Reserved spellings above are part of this contract.
- The exact fixed endpoint paths and operation-to-REST/compat mapping were not supplied. Tests pin scheme, host, port, and auth isolation, not guessed path strings or HTTP methods. Full fixed-path and all-operation successful-request coverage is not established by these tests.
- Catalog-wide override assertions use incomplete arguments for operations with required fields. Those assertions establish rejection, but cannot alone distinguish missing-argument validation from override validation. The two known no-required-argument operations provide the unambiguous override guards.
- Missing-required coverage is driven by advertised required fields. This cannot detect an implementation that omits required fields from both its schema and builder validation. Independent spec-derived fixtures for those operations are still needed to close that gap.
- JSON-RPC batches are out of scope: an array is treated as an invalid single request. Unknown tool rejection is intentionally not pinned to a specific error code because MCP permits tool-level errors. Authentication rejection likewise allows either structured error form.
- Tests use standalone dispatch calls without an initialization session. This follows the provided stateless RequestContext interface; no session lifecycle was specified.
- Builder failures must be local. Successful builder calls only prepare requests, never execute them. A network attempt during these scenarios is an implementation defect, not permission to access the service.

## Qualification handoff

After root grants resources and a manifest exists, run from this worktree:

`cargo test --test acceptance_contract`

Record exit status, failing test names, and actual failure output. A missing manifest, unresolved import, dependency error, or runtime setup error is blocked/unqualified, not the intended behavioral baseline. With no implementation yet, there is no demonstrated defect reproduction. If all cases pass when first runnable, report not reproduced rather than manufacturing a failure. These assertions remain frozen during repair.
