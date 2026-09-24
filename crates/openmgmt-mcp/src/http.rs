//! Streamable-HTTP MCP transport (issue #16).
//!
//! `OPENMGMT_MCP_TRANSPORT=http` serves the same tool registry as stdio over
//! `POST /mcp` (MCP streamable HTTP), so a remote assistant — e.g. Muse
//! over the public internet — can manage this OpenMGMT replica:
//!
//! - Bearer authentication, in two flavors (issue #38 added the second):
//!   Black Candle OAuth Bearer <redacted>, validated against the configured issuer
//!   with the [`AccountAuth`] validator from #14, or a static API key
//!   (`omg_live_…`) for agent/machine clients that cannot do the interactive
//!   OAuth flow. An empty `OPENMGMT_MCP_AUTH_ISSUER` disables auth; only do
//!   that on loopback.
//! - Remote permission model: reads plus non-destructive writes by default,
//!   subject to the #15 AI settings; destructive tools are never exposed
//!   remotely (see [`OpenMgmtMcp::new_remote`]). API keys are additionally
//!   scoped (`tasks:read` vs `tasks:write`).
//! - Per-IP fixed-window rate limiting.
//! - Every tool call and auth decision is appended to the `mcp_audit_log`
//!   table with caller, time, and tool. API-key callers are recorded as
//!   `api-key:<id>` — the key itself never appears in the audit trail.
//!
//! Writes go through [`AppService`]/[`Database`] exactly like local writes,
//! so they emit sync events and converge with other replicas normally.
//!
//! Public HTTPS is terminated by a reverse proxy (Caddy + Let's Encrypt);
//! this server speaks plain HTTP on the configured bind address. See
//! `docs/MCP_HTTP.md` for the Caddy example.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{
        StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use openmgmt_core::{
    API_KEY_PREFIX, AiToolAccess, ApiKeyScope, ApiKeyValidation, AppService, Database,
    McpAuditRecord, ai::ai_tool_metadata, scopes_allow_write,
};
use openmgmt_protocol::{AccountAuth, DEFAULT_AUTH_ISSUER};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::json;

use crate::tools::OpenMgmtMcp;

pub const DEFAULT_HTTP_BIND_ADDR: &str = "127.0.0.1:8788";
const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 120;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Configuration for the HTTP transport, read from the environment.
#[derive(Debug, Clone)]
pub struct McpHttpConfig {
    pub bind_addr: SocketAddr,
    /// `None` disables bearer auth entirely (loopback-only deployments).
    pub auth_issuer: Option<String>,
    pub writes_enabled: bool,
    pub rate_limit_per_minute: u32,
    /// Passed to rmcp's Host allow-list; `None` keeps rmcp's loopback-only
    /// default (DNS-rebinding protection).
    pub allowed_hosts: Option<Vec<String>>,
}

impl McpHttpConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind_addr = env_or("OPENMGMT_MCP_BIND_ADDR", DEFAULT_HTTP_BIND_ADDR)
            .parse::<SocketAddr>()
            .context("parse OPENMGMT_MCP_BIND_ADDR")?;
        // Empty disables auth; otherwise the issuer defaults to the Black
        // Candle issuer shared with the sync server.
        let auth_issuer = match std::env::var("OPENMGMT_MCP_AUTH_ISSUER") {
            Ok(value) if value.trim().is_empty() => None,
            Ok(value) => Some(value),
            Err(_) => Some(DEFAULT_AUTH_ISSUER.to_string()),
        };
        // #16: the default remote permission set is reads plus non-destructive
        // writes, subject to the #15 AI settings. An explicit "false" locks
        // writes down. (The stdio launcher keeps the historical default of
        // off; see main.rs.)
        let writes_enabled = match std::env::var("OPENMGMT_MCP_WRITE_ENABLED") {
            Ok(value) => value.eq_ignore_ascii_case("true"),
            Err(_) => true,
        };
        let rate_limit_per_minute = env_or(
            "OPENMGMT_MCP_RATE_LIMIT_PER_MINUTE",
            &DEFAULT_RATE_LIMIT_PER_MINUTE.to_string(),
        )
        .parse::<u32>()
        .context("parse OPENMGMT_MCP_RATE_LIMIT_PER_MINUTE")?;
        let allowed_hosts = std::env::var("OPENMGMT_MCP_ALLOWED_HOSTS")
            .ok()
            .map(|value| {
                value
                    .split(',')
                    .map(|host| host.trim().to_string())
                    .filter(|host| !host.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|hosts| !hosts.is_empty());
        Ok(Self {
            bind_addr,
            auth_issuer,
            writes_enabled,
            rate_limit_per_minute,
            allowed_hosts,
        })
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Shared state for the HTTP middleware.
#[derive(Clone)]
struct HttpState {
    auth: Option<Arc<AccountAuth>>,
    service: AppService,
    rate_limiter: RateLimiter,
}

/// Refuse to start when Bearer <redacted> is disabled on a non-loopback bind:
// an unauthenticated MCP endpoint on a reachable interface would expose
/// task data and (non-destructive) writes to the network.
fn check_bind_safety(config: &McpHttpConfig) -> anyhow::Result<()> {
    if config.auth_issuer.is_none() && !config.bind_addr.ip().is_loopback() {
        anyhow::bail!(
            "refusing to serve MCP HTTP without Bearer <redacted> on non-loopback {}: \
             set OPENMGMT_MCP_AUTH_ISSUER or bind to 127.0.0.1",
            config.bind_addr
        );
    }
    Ok(())
}

/// Validated caller identity, placed in request extensions by the auth
/// middleware for the audit middleware to consume.
#[derive(Clone)]
struct CallerIdentity {
    caller: Caller,
}

/// Who a validated request belongs to.
#[derive(Clone, Debug)]
enum Caller {
    /// Black Candle OAuth user id (the #14 path).
    User(String),
    /// API key (the #38 path): key id plus granted scopes.
    ApiKey {
        id: String,
        scopes: Vec<ApiKeyScope>,
    },
    Unauthenticated,
}

impl Caller {
    /// Audit-log name: the user id, `api-key:<id>`, or `unauthenticated`.
    /// The key itself never appears in audit rows.
    fn audit_name(&self) -> String {
        match self {
            Caller::User(id) => id.clone(),
            Caller::ApiKey { id, .. } => format!("api-key:{id}"),
            Caller::Unauthenticated => "unauthenticated".to_string(),
        }
    }
}

/// Serve the MCP registry over streamable HTTP.
pub async fn serve_http(config: &McpHttpConfig, service: AppService) -> anyhow::Result<()> {
    check_bind_safety(config)?;
    if config.auth_issuer.is_none() {
        tracing::warn!(
            "OPENMGMT_MCP_AUTH_ISSUER is empty: MCP HTTP auth is DISABLED. \
             Bound to loopback ({}), which is the only sane configuration.",
            config.bind_addr
        );
    }

    let auth = config
        .auth_issuer
        .as_deref()
        .map(AccountAuth::new)
        .transpose()
        .context("configure account auth")?
        .map(Arc::new);
    let state = HttpState {
        auth,
        service: service.clone(),
        rate_limiter: RateLimiter::new(config.rate_limit_per_minute, Duration::from_secs(60)),
    };

    let session_manager = Arc::new(LocalSessionManager::default());
    let writes_enabled = config.writes_enabled;
    let mcp_service = StreamableHttpService::new(
        move || Ok(OpenMgmtMcp::new_remote(service.clone(), writes_enabled)),
        session_manager,
        streamable_config(config),
    );

    // Layer order: the last `layer` call is outermost, so requests flow
    // rate limiter -> auth -> audit -> MCP service.
    let protected = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            audit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
    let app = Router::new()
        .merge(protected)
        .route("/health", get(health))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ));

    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("bind MCP HTTP to {}", config.bind_addr))?;
    tracing::info!(
        bind_addr = %config.bind_addr,
        writes_enabled = config.writes_enabled,
        auth_enabled = state.auth.is_some(),
        rate_limit_per_minute = config.rate_limit_per_minute,
        "serving OpenMgmt MCP over streamable HTTP at /mcp"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("serve MCP HTTP")?;
    Ok(())
}

fn streamable_config(config: &McpHttpConfig) -> StreamableHttpServerConfig {
    let mut mcp_config = StreamableHttpServerConfig::default();
    mcp_config.stateful_mode = true;
    if let Some(hosts) = &config.allowed_hosts {
        mcp_config.allowed_hosts = hosts.clone();
    }
    mcp_config
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "transport": "http", "endpoint": "/mcp" }))
}

/// Reject with 401 and record the denial in the audit trail.
async fn auth_middleware(
    State(state): State<HttpState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(auth) = state.auth.as_ref() else {
        // Auth disabled: every caller is anonymous. The loud startup warning
        // (serve_http) is the guardrail; only loopback binds are sane here.
        request.extensions_mut().insert(CallerIdentity {
            caller: Caller::Unauthenticated,
        });
        return next.run(request).await;
    };

    let token = bearer_token(request.headers());
    let Some(token) = token else {
        deny(
            &state.service,
            "unauthenticated",
            "missing or malformed Authorization bearer token",
        )
        .await;
        return unauthorized("missing or malformed Authorization bearer token");
    };
    match authenticate(auth, &state.service.database(), &token).await {
        Ok(caller) => {
            request.extensions_mut().insert(CallerIdentity { caller });
            next.run(request).await
        }
        Err(detail) => {
            deny(&state.service, "unauthenticated", &detail).await;
            unauthorized(&detail)
        }
    }
}

/// Validate a Bearer <redacted> Two credential types, routed by prefix:
///
/// - `omg_live_…` → API-key validation against the local `mcp_api_keys`
///   table (issue #38). Cheap, offline, no network.
/// - anything else → the existing `AccountAuth` OAuth validation (issue
///   #14), byte-for-byte unchanged.
///
/// On failure the returned string is safe to expose: it names the reason
/// and, for revoked keys, the key id — never the key.
async fn authenticate(
    auth: &AccountAuth,
    database: &Database,
    token: &str,
) -> Result<Caller, String> {
    if token.starts_with(API_KEY_PREFIX) {
        return match database.validate_api_key(token) {
            Ok(ApiKeyValidation::Valid(key)) => Ok(Caller::ApiKey {
                id: key.id,
                scopes: key.scopes,
            }),
            Ok(ApiKeyValidation::Revoked { id }) => Err(format!("API key {id} has been revoked")),
            Ok(ApiKeyValidation::Unknown) => Err("unknown API key".to_string()),
            Err(error) => Err(format!("API key validation failed: {error}")),
        };
    }
    match auth.validate(token).await {
        Ok(identity) => Ok(Caller::User(identity.user_id)),
        Err(error) => Err(error.to_string()),
    }
}

/// Enforce per-key scopes (issue #38): a key without `tasks:write` may only
/// invoke read tools. Tool classification reuses the #15 AI registry — the
/// single source of truth for read vs write — so scopes can never drift
/// from the registry. Non-tool MCP methods (`initialize`, `tools/list`,
/// `ping`) carry no data access and are always allowed; unknown tool names
/// fail closed downstream at the MCP router, so they pass the scope check.
fn check_api_key_scope(scopes: &[ApiKeyScope], tool_names: &[String]) -> Result<(), String> {
    if scopes_allow_write(scopes) {
        return Ok(());
    }
    for tool_name in tool_names {
        let Some(name) = tool_name.strip_prefix("tools/call:") else {
            continue;
        };
        let is_write =
            ai_tool_metadata(name).is_some_and(|meta| meta.access == AiToolAccess::Write);
        if is_write {
            return Err(format!(
                "API key lacks the 'tasks:write' scope required by tool '{name}'"
            ));
        }
    }
    Ok(())
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = parts.next()?;
    // Reject anything after the token (no room for parameter smuggling).
    if parts.next().is_some() {
        return None;
    }
    Some(token.to_string())
}

fn unauthorized(detail: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(WWW_AUTHENTICATE, "Bearer")],
        Json(json!({ "error": "unauthorized", "detail": detail })),
    )
        .into_response()
}

/// 403 for authenticated callers that lack permission (e.g. an API key
/// without the scope a tool requires). Distinct from 401: the credential
/// was valid, the action is not permitted.
fn forbidden(detail: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "forbidden", "detail": detail })),
    )
        .into_response()
}

async fn deny(service: &AppService, caller: &str, detail: &str) {
    tracing::warn!("MCP HTTP auth denied: {detail}");
    let record = McpAuditRecord {
        caller: caller.to_string(),
        transport: "http".to_string(),
        tool_name: "auth".to_string(),
        success: false,
        detail: Some(detail.to_string()),
    };
    if let Err(error) = service.record_mcp_audit(&record) {
        tracing::warn!("failed to record MCP auth audit: {error}");
    }
}

/// Parse the JSON-RPC body, run the request, then append one audit row per
/// tool call (or MCP method) with caller, time, and tool.
///
/// API-key callers are additionally scope-checked here (issue #38): the
/// parsed tool names are known before the request runs, so a read-only key
/// attempting a write tool is rejected with 403 and never reaches the MCP
/// service.
async fn audit_middleware(
    State(state): State<HttpState>,
    request: Request,
    next: Next,
) -> Response {
    let caller = request
        .extensions()
        .get::<CallerIdentity>()
        .map(|identity| identity.caller.clone())
        .unwrap_or(Caller::Unauthenticated);
    let caller_name = caller.audit_name();

    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({ "error": "request body too large" })),
            )
                .into_response();
        }
    };
    let tool_names = extract_tool_names(&bytes);

    // Scope-check API-key callers before the request runs; a read-only key
    // attempting a write tool is rejected here and never reaches the MCP
    // service.
    let scope_denial = match &caller {
        Caller::ApiKey { scopes, .. } => check_api_key_scope(scopes, &tool_names).err(),
        _ => None,
    };
    if let Some(reason) = scope_denial {
        deny(&state.service, &caller_name, &reason).await;
        return forbidden(&reason);
    }

    let request = Request::from_parts(parts, Body::from(bytes));

    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    // MCP tool failures come back as HTTP 200 with a JSON-RPC error object,
    // so the audit trail looks inside plain JSON bodies. SSE streams cannot
    // be inspected without breaking them, so those keep HTTP-status success.
    let is_sse = parts
        .headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.contains("text/event-stream"));
    if is_sse {
        let success = parts.status.is_success();
        audit_tool_calls(
            &state.service,
            &caller_name,
            &tool_names,
            success,
            parts.status,
        );
        return Response::from_parts(parts, body);
    }
    let bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            audit_tool_calls(
                &state.service,
                &caller_name,
                &tool_names,
                false,
                StatusCode::BAD_GATEWAY,
            );
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "could not read MCP response" })),
            )
                .into_response();
        }
    };
    let success = parts.status.is_success() && !is_jsonrpc_error(&bytes);
    audit_tool_calls(
        &state.service,
        &caller_name,
        &tool_names,
        success,
        parts.status,
    );
    Response::from_parts(parts, Body::from(bytes))
}

/// Append one audit row per tool call (or MCP method) with caller, time,
/// tool, and whether the call actually succeeded.
fn audit_tool_calls(
    service: &AppService,
    caller: &str,
    tool_names: &[String],
    success: bool,
    status: StatusCode,
) {
    for tool_name in tool_names {
        let record = McpAuditRecord {
            caller: caller.to_string(),
            transport: "http".to_string(),
            tool_name: tool_name.clone(),
            success,
            detail: (!success).then(|| format!("HTTP {status}")),
        };
        if let Err(error) = service.record_mcp_audit(&record) {
            tracing::warn!("failed to record MCP audit: {error}");
        }
    }
}

/// True when a JSON-RPC response body carries an error object, either as a
/// single response or inside a batch.
fn is_jsonrpc_error(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let messages: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    messages
        .iter()
        .any(|message| message.get("error").is_some())
}

/// Extract auditable names from a JSON-RPC request body. `tools/call`
/// becomes `tools/call:<name>`; other methods are recorded by method name;
/// notifications carry no tool call and are skipped.
fn extract_tool_names(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return vec!["unparseable".to_string()];
    };
    let messages: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    messages
        .into_iter()
        .filter_map(|message| {
            let method = message.get("method")?.as_str()?;
            if method.starts_with("notifications/") {
                return None;
            }
            if method == "tools/call" {
                let name = message.get("params")?.get("name")?.as_str()?;
                Some(format!("tools/call:{name}"))
            } else {
                Some(method.to_string())
            }
        })
        .collect()
}

/// Fixed-window per-IP rate limiter. This blunts accidental client loops
/// against a single-owner endpoint; the reverse proxy owns real DDoS
/// defense.
#[derive(Clone)]
struct RateLimiter {
    inner: Arc<tokio::sync::Mutex<HashMap<IpAddr, Window>>>,
    limit: u32,
    window: Duration,
}

struct Window {
    start: Instant,
    count: u32,
}

impl RateLimiter {
    fn new(limit: u32, window: Duration) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            limit,
            window,
        }
    }

    /// Returns true when the request is within budget.
    async fn check(&self, ip: IpAddr) -> bool {
        let mut buckets = self.inner.lock().await;
        let now = Instant::now();
        // Bound memory: drop buckets idle for two windows once the map grows.
        if buckets.len() > 4096 {
            buckets.retain(|_, window| now.duration_since(window.start) < self.window * 2);
        }
        let window = buckets.entry(ip).or_insert(Window {
            start: now,
            count: 0,
        });
        if now.duration_since(window.start) >= self.window {
            *window = Window {
                start: now,
                count: 1,
            };
            return true;
        }
        if window.count < self.limit {
            window.count += 1;
            return true;
        }
        false
    }
}

async fn rate_limit_middleware(
    State(state): State<HttpState>,
    request: Request,
    next: Next,
) -> Response {
    let ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    if !state.rate_limiter.check(ip).await {
        tracing::warn!(%ip, "MCP HTTP rate limit exceeded");
        // The rate limiter runs before auth, so the caller is unknown; the
        // rejection is still security-relevant and belongs in the audit log.
        let record = McpAuditRecord {
            caller: "unauthenticated".to_string(),
            transport: "http".to_string(),
            tool_name: "rate_limit".to_string(),
            success: false,
            detail: Some(format!("rate limit exceeded for {ip}")),
        };
        if let Err(error) = state.service.record_mcp_audit(&record) {
            tracing::warn!("failed to record MCP rate-limit audit: {error}");
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({ "error": "rate limit exceeded" })),
        )
            .into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use openmgmt_core::Database;
    use std::sync::Mutex;

    /// Env-var tests mutate process-global state, so they serialize on this.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn config_defaults_to_auth_and_remote_writes() {
        let _guard = ENV_LOCK.lock().unwrap();
        let config = McpHttpConfig::from_env().unwrap();
        assert_eq!(
            config.bind_addr,
            DEFAULT_HTTP_BIND_ADDR.parse::<SocketAddr>().unwrap()
        );
        assert_eq!(config.auth_issuer.as_deref(), Some(DEFAULT_AUTH_ISSUER));
        // #16: the default remote permission set is reads plus
        // non-destructive writes.
        assert!(config.writes_enabled);
        assert_eq!(config.rate_limit_per_minute, DEFAULT_RATE_LIMIT_PER_MINUTE);
        assert!(config.allowed_hosts.is_none());
    }

    #[test]
    fn empty_issuer_disables_auth() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("OPENMGMT_MCP_AUTH_ISSUER", "") };
        let config = McpHttpConfig::from_env().unwrap();
        assert!(config.auth_issuer.is_none());
        unsafe { std::env::remove_var("OPENMGMT_MCP_AUTH_ISSUER") };
    }

    #[test]
    fn explicit_write_env_is_honored() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::set_var("OPENMGMT_MCP_WRITE_ENABLED", "false") };
        assert!(!McpHttpConfig::from_env().unwrap().writes_enabled);
        unsafe { std::env::set_var("OPENMGMT_MCP_WRITE_ENABLED", "true") };
        assert!(McpHttpConfig::from_env().unwrap().writes_enabled);
        unsafe { std::env::remove_var("OPENMGMT_MCP_WRITE_ENABLED") };
    }

    #[test]
    fn extract_tool_names_handles_calls_lists_and_batches() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_tasks","arguments":{}}}"#;
        assert_eq!(extract_tool_names(body), vec!["tools/call:list_tasks"]);

        let body = br#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        assert_eq!(extract_tool_names(body), vec!["tools/list"]);

        let body = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(extract_tool_names(body).is_empty());

        let body = br#"[{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_task"}},{"jsonrpc":"2.0","id":2,"method":"ping"}]"#;
        assert_eq!(
            extract_tool_names(body),
            vec!["tools/call:get_task", "ping"]
        );

        assert_eq!(extract_tool_names(b"not json"), vec!["unparseable"]);
    }

    #[test]
    fn bearer_token_parsing() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer abc123".parse().unwrap());
        assert_eq!(bearer_token(&headers).as_deref(), Some("abc123"));

        headers.insert(AUTHORIZATION, "bearer xyz".parse().unwrap());
        assert_eq!(bearer_token(&headers).as_deref(), Some("xyz"));

        headers.insert(AUTHORIZATION, "Basic abc".parse().unwrap());
        assert!(bearer_token(&headers).is_none());

        headers.insert(AUTHORIZATION, "Bearer a b".parse().unwrap());
        assert!(bearer_token(&headers).is_none());

        headers.remove(AUTHORIZATION);
        assert!(bearer_token(&headers).is_none());
    }

    #[tokio::test]
    async fn rate_limiter_allows_then_denies_then_resets() {
        let limiter = RateLimiter::new(2, Duration::from_millis(50));
        let ip = IpAddr::from([127, 0, 0, 1]);
        assert!(limiter.check(ip).await);
        assert!(limiter.check(ip).await);
        assert!(!limiter.check(ip).await);
        // A different IP has its own budget.
        assert!(limiter.check(IpAddr::from([127, 0, 0, 2])).await);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(limiter.check(ip).await);
    }

    #[test]
    fn jsonrpc_error_detection() {
        // Single success response: no error.
        assert!(!is_jsonrpc_error(
            br#"{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}"#
        ));
        // Single error response.
        assert!(is_jsonrpc_error(
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"not found"}}"#
        ));
        // Batch with one error.
        assert!(is_jsonrpc_error(
            br#"[{"jsonrpc":"2.0","id":1,"result":{}},{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"boom"}}]"#
        ));
        // Unparseable bodies are not treated as errors.
        assert!(!is_jsonrpc_error(b"not json"));
        assert!(!is_jsonrpc_error(b""));
    }

    #[test]
    fn bind_safety_requires_loopback_when_auth_disabled() {
        let loopback = McpHttpConfig {
            bind_addr: "127.0.0.1:8788".parse().unwrap(),
            auth_issuer: None,
            ..config_for_bind_test()
        };
        assert!(check_bind_safety(&loopback).is_ok());

        let public = McpHttpConfig {
            bind_addr: "0.0.0.0:8788".parse().unwrap(),
            auth_issuer: None,
            ..config_for_bind_test()
        };
        assert!(check_bind_safety(&public).is_err());

        // Auth enabled: any bind is allowed (TLS + Bearer <redacted> the reverse proxy).
        let public_authed = McpHttpConfig {
            bind_addr: "0.0.0.0:8788".parse().unwrap(),
            auth_issuer: Some("https://auth.example.com".to_string()),
            ..config_for_bind_test()
        };
        assert!(check_bind_safety(&public_authed).is_ok());
    }

    fn config_for_bind_test() -> McpHttpConfig {
        McpHttpConfig {
            bind_addr: "127.0.0.1:8788".parse().unwrap(),
            auth_issuer: None,
            writes_enabled: true,
            allowed_hosts: None,
            rate_limit_per_minute: 120,
        }
    }

    /// `AccountAuth::new` builds an HTTP client but performs no I/O, so the
    /// API-key path can be tested without network: any `omg_live_` token is
    /// validated locally and never reaches the OAuth issuer.
    fn dummy_auth() -> AccountAuth {
        AccountAuth::new("https://auth.invalid").unwrap()
    }

    #[tokio::test]
    async fn authenticate_accepts_valid_api_key() {
        let database = Database::in_memory().unwrap();
        let (key, plaintext) = database
            .create_api_key("agent", &[ApiKeyScope::TasksRead, ApiKeyScope::TasksWrite])
            .unwrap();
        let caller = authenticate(&dummy_auth(), &database, &plaintext)
            .await
            .unwrap();
        let (id, scopes) = match caller.clone() {
            Caller::ApiKey { id, scopes } => (id, scopes),
            other => panic!("expected ApiKey caller, got {other:?}"),
        };
        assert_eq!(id, key.id);
        assert_eq!(
            scopes,
            vec![ApiKeyScope::TasksRead, ApiKeyScope::TasksWrite]
        );
        // The audit name carries the key id, never the key.
        assert_eq!(caller.audit_name(), format!("api-key:{}", key.id));
        assert!(!caller.audit_name().contains(&plaintext));
    }

    #[tokio::test]
    async fn authenticate_rejects_unknown_and_revoked_keys() {
        let database = Database::in_memory().unwrap();
        let (key, plaintext) = database
            .create_api_key("agent", &[ApiKeyScope::TasksRead])
            .unwrap();
        let missing = format!("{API_KEY_PREFIX}0000000000000000000000000000000000000000000");
        let detail = authenticate(&dummy_auth(), &database, &missing)
            .await
            .unwrap_err();
        assert!(detail.contains("unknown API key"), "{detail}");

        database.revoke_api_key(&key.id).unwrap();
        let detail = authenticate(&dummy_auth(), &database, &plaintext)
            .await
            .unwrap_err();
        // Revocation names the key id (safe for audit) without the key.
        assert!(detail.contains(&key.id), "{detail}");
        assert!(detail.contains("revoked"), "{detail}");
        assert!(!detail.contains(&plaintext));
    }

    #[test]
    fn api_key_scope_enforcement_uses_registry_classification() {
        let read = [ApiKeyScope::TasksRead];
        let read_write = [ApiKeyScope::TasksRead, ApiKeyScope::TasksWrite];

        // Read tools pass on a read-only key.
        assert!(check_api_key_scope(&read, &["tools/call:query_tasks".into()]).is_ok());
        assert!(check_api_key_scope(&read, &["tools/call:list_tasks".into()]).is_ok());
        assert!(check_api_key_scope(&read, &["tools/call:get_board_state".into()]).is_ok());
        // Non-tool MCP methods carry no data access.
        assert!(check_api_key_scope(&read, &["initialize".into()]).is_ok());
        assert!(check_api_key_scope(&read, &["tools/list".into()]).is_ok());

        // Write tools are denied without the write scope.
        let detail = check_api_key_scope(&read, &["tools/call:update_task".into()]).unwrap_err();
        assert!(detail.contains("tasks:write"), "{detail}");
        assert!(detail.contains("update_task"), "{detail}");
        let detail = check_api_key_scope(&read, &["tools/call:create_task".into()]).unwrap_err();
        assert!(detail.contains("tasks:write"), "{detail}");

        // A batch is denied if any call needs write.
        let detail = check_api_key_scope(
            &read,
            &[
                "tools/call:query_tasks".into(),
                "tools/call:complete_task".into(),
            ],
        )
        .unwrap_err();
        assert!(detail.contains("complete_task"), "{detail}");

        // The write scope permits both.
        assert!(check_api_key_scope(&read_write, &["tools/call:update_task".into()]).is_ok());
        assert!(check_api_key_scope(&read_write, &["tools/call:query_tasks".into()]).is_ok());

        // Unknown tool names cannot execute (the MCP router rejects them),
        // so they pass the scope check and fail closed downstream.
        assert!(check_api_key_scope(&read, &["tools/call:nope_not_a_tool".into()]).is_ok());
    }

    #[test]
    fn caller_audit_names() {
        assert_eq!(
            Caller::User("user-123".to_string()).audit_name(),
            "user-123"
        );
        assert_eq!(
            Caller::ApiKey {
                id: "key-id".to_string(),
                scopes: vec![ApiKeyScope::TasksRead],
            }
            .audit_name(),
            "api-key:key-id"
        );
        assert_eq!(Caller::Unauthenticated.audit_name(), "unauthenticated");
    }
}
