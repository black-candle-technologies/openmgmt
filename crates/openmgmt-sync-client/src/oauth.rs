//! Native-app OAuth login against a Black Candle (authd) issuer.
//!
//! Implements the RFC 8252 native-app flow: the system browser opens the
//! issuer's authorization endpoint with PKCE S256, and the issuer redirects
//! back to a loopback callback (`http://127.0.0.1:<port>/callback`) with an
//! authorization code that is exchanged for an access token.
//!
//! The client identifies itself with a CIMD client-metadata document
//! (RFC 9728): the `client_id` is the `https://` URL of a JSON document the
//! issuer fetches and validates. Public clients must use PKCE, and the
//! redirect URI must exactly match one listed in the document, so the
//! loopback listener binds the first free port from a fixed candidate set
//! that the published document enumerates.
//!
//! Access tokens are kept in the OS secure storage (OS keychain / credential
//! manager / Secret Service) via the [`TokenStore`] abstraction and are
//! never written to the OpenMGMT SQLite database. The access token is only
//! needed at device-registration time (two-tier auth); day-to-day sync uses
//! the device token, so no refresh flow is required.

use crate::{SyncClientError, SyncClientResult};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{TryRngCore, rngs::OsRng};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// Keychain account under which the Black Candle access token is stored.
pub const ACCESS_TOKEN_ACCOUNT: &str = "black-candle-access-token";

/// Service name used for OS secure storage entries.
pub const KEYRING_SERVICE: &str = "openmgmt";

/// Native OAuth configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Issuer base URL, e.g. `https://auth.blackcandletech.com`.
    pub issuer: String,
    /// CIMD client-metadata document URL used as the OAuth `client_id`.
    pub client_id: String,
    /// Candidate loopback ports for the OAuth callback, tried in order.
    /// Every entry must appear verbatim in the metadata document's
    /// `redirect_uris`.
    pub redirect_ports: Vec<u16>,
    /// How long to wait for the user to finish the browser flow.
    pub login_timeout: Duration,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            issuer: "https://auth.blackcandletech.com".to_string(),
            client_id: "https://blackcandletech.com/oauth/openmgmt-desktop.json".to_string(),
            redirect_ports: vec![18441, 18442, 18443, 18444, 18445],
            login_timeout: Duration::from_secs(300),
        }
    }
}

/// Secure token storage abstraction. The production implementation uses
/// the OS keychain; tests use an in-memory store.
pub trait TokenStore: Send + Sync {
    fn get(&self, account: &str) -> SyncClientResult<Option<String>>;
    fn set(&self, account: &str, token: &str) -> SyncClientResult<()>;
    fn delete(&self, account: &str) -> SyncClientResult<()>;
}

/// OS keychain / credential manager / Secret Service token store.
pub struct KeyringTokenStore {
    service: String,
}

impl KeyringTokenStore {
    pub fn new() -> Self {
        Self {
            service: KEYRING_SERVICE.to_string(),
        }
    }

    fn entry(&self, account: &str) -> SyncClientResult<keyring::Entry> {
        keyring::Entry::new(&self.service, account)
            .map_err(|e| SyncClientError::Other(format!("keychain unavailable: {e}")))
    }
}

impl Default for KeyringTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStore for KeyringTokenStore {
    fn get(&self, account: &str) -> SyncClientResult<Option<String>> {
        match self.entry(account)?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SyncClientError::Other(format!("keychain read failed: {e}"))),
        }
    }

    fn set(&self, account: &str, token: &str) -> SyncClientResult<()> {
        self.entry(account)?
            .set_password(token)
            .map_err(|e| SyncClientError::Other(format!("keychain write failed: {e}")))
    }

    fn delete(&self, account: &str) -> SyncClientResult<()> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SyncClientError::Other(format!(
                "keychain delete failed: {e}"
            ))),
        }
    }
}

/// In-memory token store for tests.
#[derive(Debug, Default)]
pub struct MemoryTokenStore {
    inner: Mutex<HashMap<String, String>>,
}

impl TokenStore for MemoryTokenStore {
    fn get(&self, account: &str) -> SyncClientResult<Option<String>> {
        Ok(self.inner.lock().unwrap().get(account).cloned())
    }

    fn set(&self, account: &str, token: &str) -> SyncClientResult<()> {
        self.inner
            .lock()
            .unwrap()
            .insert(account.to_string(), token.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> SyncClientResult<()> {
        self.inner.lock().unwrap().remove(account);
        Ok(())
    }
}

/// Load the stored Black Candle access token, if the user has signed in.
pub fn load_access_token(store: &dyn TokenStore) -> SyncClientResult<Option<String>> {
    store.get(ACCESS_TOKEN_ACCOUNT)
}

/// Forget the stored Black Candle access token (sign out).
pub fn clear_access_token(store: &dyn TokenStore) -> SyncClientResult<()> {
    store.delete(ACCESS_TOKEN_ACCOUNT)
}

/// Generate a PKCE code verifier / S256 challenge pair (RFC 7636).
pub fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .expect("OS RNG unavailable");
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    OsRng
        .try_fill_bytes(&mut bytes)
        .expect("OS RNG unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Build the issuer authorization URL for the browser redirect.
pub fn authorize_url(
    config: &OAuthConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> SyncClientResult<String> {
    let mut url = url::Url::parse(config.issuer.trim_end_matches('/'))
        .map_err(|e| SyncClientError::Other(format!("invalid OAuth issuer: {e}")))?;
    url.path_segments_mut()
        .map_err(|_| SyncClientError::Other("invalid OAuth issuer".to_string()))?
        .push("oauth")
        .push("authorize");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", "identity")
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url.to_string())
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[allow(dead_code)]
    token_type: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<u64>,
}

/// Exchange an authorization code for an access token.
pub async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> SyncClientResult<String> {
    let endpoint = format!("{}/oauth/token", config.issuer.trim_end_matches('/'));
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let response: TokenResponse = http
        .post(&endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", config.client_id.as_str()),
            ("code_verifier", verifier),
        ])
        .send()
        .await?
        .error_for_status()
        .map_err(|e| SyncClientError::Other(format!("token exchange failed: {e}")))?
        .json()
        .await
        .map_err(|e| SyncClientError::Other(format!("token exchange failed: {e}")))?;
    if response.access_token.is_empty() {
        return Err(SyncClientError::Other(
            "token endpoint returned an empty access token".to_string(),
        ));
    }
    Ok(response.access_token)
}

/// Bind the loopback callback listener on the first free candidate port.
pub async fn bind_callback(config: &OAuthConfig) -> SyncClientResult<TcpListener> {
    let mut last_error = None;
    for port in &config.redirect_ports {
        match TcpListener::bind(("127.0.0.1", *port)).await {
            Ok(listener) => return Ok(listener),
            Err(e) => last_error = Some(e),
        }
    }
    Err(SyncClientError::Other(format!(
        "no free OAuth callback port in {:?}: {}",
        config.redirect_ports,
        last_error.map(|e| e.to_string()).unwrap_or_default()
    )))
}

/// Await the single OAuth redirect on the loopback listener and return the
/// authorization code. Validates `state`; any error is rendered to the
/// browser so the user is not left staring at a hanging tab.
async fn await_callback(
    listener: TcpListener,
    state: &str,
    wait: Duration,
) -> SyncClientResult<String> {
    let (mut stream, _) = timeout(wait, listener.accept())
        .await
        .map_err(|_| {
            SyncClientError::Other("sign-in timed out waiting for the browser".to_string())
        })?
        .map_err(|e| SyncClientError::Other(format!("callback listener failed: {e}")))?;

    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| SyncClientError::Other(format!("callback read failed: {e}")))?;
    // Drain headers so the connection stays well-formed.
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| SyncClientError::Other(format!("callback read failed: {e}")))?;
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
    }

    let result = callback_code(&request_line, state);
    let (status, title, body): (&str, &str, String) = match &result {
        Ok(_) => (
            "200 OK",
            "Signed in",
            "Signed in to OpenMGMT. You can close this tab and return to the app.".to_string(),
        ),
        Err(message) => ("400 Bad Request", "Sign-in failed", message.to_string()),
    };
    let page = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>{title}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:40em;margin:4em auto;padding:0 1em\">\
         <h1>{title}</h1><p>{body}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    result
}

/// Extract and validate the authorization code from the callback request line.
fn callback_code(request_line: &str, expected_state: &str) -> SyncClientResult<String> {
    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| SyncClientError::Other("malformed callback request".to_string()))?;
    if !path.starts_with("/callback") {
        return Err(SyncClientError::Other(
            "unexpected callback path".to_string(),
        ));
    }
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    if let Some(error) = params.get("error") {
        let description = params
            .get("error_description")
            .map(String::as_str)
            .unwrap_or("the sign-in was not completed");
        return Err(SyncClientError::Other(format!(
            "authorization failed ({error}): {description}"
        )));
    }
    match params.get("state") {
        Some(state) if *state == expected_state => {}
        _ => {
            return Err(SyncClientError::Other(
                "callback state mismatch; possible cross-site request".to_string(),
            ));
        }
    }
    params
        .get("code")
        .map(|s| s.to_string())
        .ok_or_else(|| SyncClientError::Other("callback carried no authorization code".to_string()))
}

/// Run the interactive native-app sign-in: open the system browser, wait for
/// the loopback callback, exchange the code, and persist the access token in
/// the OS keychain. Returns the access token.
pub async fn login(config: &OAuthConfig, store: &dyn TokenStore) -> SyncClientResult<String> {
    login_with_opener(config, store, &|url| {
        open::that(url).map_err(|e| SyncClientError::Other(format!("could not open browser: {e}")))
    })
    .await
}

async fn login_with_opener(
    config: &OAuthConfig,
    store: &dyn TokenStore,
    open_browser: &dyn Fn(&str) -> SyncClientResult<()>,
) -> SyncClientResult<String> {
    let (verifier, challenge) = pkce_pair();
    let state = random_state();
    let listener = bind_callback(config).await?;
    let port = listener
        .local_addr()
        .map_err(|e| SyncClientError::Other(format!("callback listener failed: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let auth_url = authorize_url(config, &redirect_uri, &challenge, &state)?;

    open_browser(&auth_url)?;
    let code = await_callback(listener, &state, config.login_timeout).await?;
    let access_token = exchange_code(config, &code, &redirect_uri, &verifier).await?;
    store.set(ACCESS_TOKEN_ACCOUNT, &access_token)?;
    Ok(access_token)
}

/// Resolve the Bearer <redacted> for device registration: an explicitly
/// configured token wins, otherwise the keychain-held token from a previous
/// interactive sign-in.
pub fn resolve_bearer_token(
    configured: Option<&str>,
    store: &dyn TokenStore,
) -> SyncClientResult<Option<String>> {
    if let Some(token) = configured {
        return Ok(Some(token.to_string()));
    }
    load_access_token(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> OAuthConfig {
        OAuthConfig {
            issuer: "https://auth.example.test".to_string(),
            client_id: "https://client.example.test/meta.json".to_string(),
            redirect_ports: vec![0],
            login_timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let (verifier, challenge) = pkce_pair();
        assert!((43..=128).contains(&verifier.len()));
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn authorize_url_has_required_parameters() {
        let config = test_config();
        let url =
            authorize_url(&config, "http://127.0.0.1:18441/callback", "CHAL", "STATE").unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("auth.example.test"));
        assert_eq!(parsed.path(), "/oauth/authorize");
        let params: HashMap<String, String> = parsed.query_pairs().into_owned().collect();
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["client_id"], config.client_id);
        assert_eq!(params["redirect_uri"], "http://127.0.0.1:18441/callback");
        assert_eq!(params["scope"], "identity");
        assert_eq!(params["state"], "STATE");
        assert_eq!(params["code_challenge"], "CHAL");
        assert_eq!(params["code_challenge_method"], "S256");
    }

    #[test]
    fn callback_code_accepts_valid_redirect() {
        let line = "GET /callback?code=abc123&state=xyz HTTP/1.1\r\n";
        assert_eq!(callback_code(line, "xyz").unwrap(), "abc123");
    }

    #[test]
    fn callback_code_rejects_state_mismatch() {
        let line = "GET /callback?code=abc123&state=evil HTTP/1.1\r\n";
        assert!(callback_code(line, "xyz").is_err());
    }

    #[test]
    fn callback_code_surfaces_provider_error() {
        let line = "GET /callback?error=access_denied&state=xyz HTTP/1.1\r\n";
        let err = callback_code(line, "xyz").unwrap_err();
        assert!(err.to_string().contains("access_denied"));
    }

    #[tokio::test]
    async fn memory_token_store_round_trip() {
        let store = MemoryTokenStore::default();
        assert_eq!(store.get("acct").unwrap(), None);
        store.set("acct", "tok-1").unwrap();
        assert_eq!(store.get("acct").unwrap().as_deref(), Some("tok-1"));
        store.delete("acct").unwrap();
        assert_eq!(store.get("acct").unwrap(), None);
    }

    #[test]
    fn resolve_bearer_token_prefers_configured() {
        let store = MemoryTokenStore::default();
        store.set(ACCESS_TOKEN_ACCOUNT, "keychain-token").unwrap();
        let resolved = resolve_bearer_token(Some("explicit-token"), &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("explicit-token"));
        let resolved = resolve_bearer_token(None, &store).unwrap();
        assert_eq!(resolved.as_deref(), Some("keychain-token"));
    }

    #[tokio::test]
    async fn login_flow_against_mock_issuer() {
        // Mock token endpoint.
        let token_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let token_port = token_listener.local_addr().unwrap().port();
        let token_task = tokio::spawn(async move {
            let (mut stream, _) = token_listener.accept().await.unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut body = Vec::new();
            // Read request line + headers, then the exact form body.
            let mut content_length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                let lower = line.to_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut buf = vec![0u8; content_length];
            tokio::io::AsyncReadExt::read_exact(&mut reader, &mut buf)
                .await
                .unwrap();
            body.extend_from_slice(&buf);
            let body = String::from_utf8(body).unwrap();
            assert!(body.contains("grant_type=authorization_code"));
            assert!(body.contains("code_verifier="));
            let payload =
                r#"{"access_token":"mock-access","token_type":"bearer","expires_in":3600}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let mut config = test_config();
        config.issuer = format!("http://127.0.0.1:{token_port}");
        config.redirect_ports = vec![0];

        // Simulate the browser by hitting the callback ourselves once the
        // login task is listening: we hook the opener to capture the URL,
        // parse out state, and drive the loopback callback directly.
        let store = MemoryTokenStore::default();

        // Exercise the pieces — bind a callback listener, simulate
        // the browser redirect, and complete the exchange.
        let listener = bind_callback(&config).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let redirect_uri = format!("http://127.0.0.1:{port}/callback");
        let state = "test-state";

        let drive = tokio::spawn(async move {
            // Give the callback waiter a moment to start accepting.
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let request =
                format!("GET /callback?code=mock-code&state={state} HTTP/1.1\r\nHost: x\r\n\r\n");
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut response)
                .await
                .unwrap();
            String::from_utf8(response).unwrap()
        });

        let code = await_callback(listener, state, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(code, "mock-code");
        let page = drive.await.unwrap();
        assert!(page.contains("200 OK"));
        assert!(page.contains("Signed in"));

        let (verifier, _) = pkce_pair();
        let token = exchange_code(&config, &code, &redirect_uri, &verifier)
            .await
            .unwrap();
        assert_eq!(token, "mock-access");
        store.set(ACCESS_TOKEN_ACCOUNT, &token).unwrap();
        assert_eq!(
            load_access_token(&store).unwrap().as_deref(),
            Some("mock-access")
        );
        token_task.await.unwrap();
    }
}
