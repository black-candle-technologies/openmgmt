//! Native Dispatch transport: never expose the account bearer token to WASM.
use reqwest::{Client, Method};
use serde_json::Value;
use std::time::Duration;
use url::Url;

pub const DEFAULT_BASE_URL: &str = "https://dispatch.blackcandletech.com";

pub struct DispatchClient {
    base: Url,
    token: String,
    http: Client,
}
impl DispatchClient {
    pub fn new(base: &str, token: String) -> Result<Self, String> {
        let mut base = Url::parse(base).map_err(|_| "Invalid Dispatch URL")?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "Dispatch requires an HTTPS URL without credentials, query or fragment".into(),
            );
        }
        base.set_path(&format!("{}/", base.path().trim_end_matches('/')));
        Ok(Self {
            base,
            token,
            http: Client::builder()
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Could not initialize Dispatch client")?,
        })
    }
    pub async fn request(
        &self,
        action: &str,
        id: Option<&str>,
        body: Value,
    ) -> Result<Value, String> {
        let (method, path) = match action {
            "generate" => (Method::POST, "v1/prompts:generate".to_owned()),
            "models" => (Method::GET, "v1/models".to_owned()),
            "dispatch" => (Method::POST, "v1/dispatches".to_owned()),
            "list" => (Method::GET, "v1/dispatches".to_owned()),
            "get" | "message" | "cancel" => {
                let id = id
                    .filter(|id| {
                        !id.is_empty()
                            && id
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    })
                    .ok_or("Invalid dispatch id")?;
                let suffix = match action {
                    "message" => "/messages",
                    "cancel" => "/cancel",
                    _ => "",
                };
                (
                    if action == "get" {
                        Method::GET
                    } else {
                        Method::POST
                    },
                    format!("v1/dispatches/{id}{suffix}"),
                )
            }
            _ => return Err("Unknown Dispatch action".into()),
        };
        let mut url = self.base.join(&path).map_err(|_| "Invalid Dispatch URL")?;
        if action == "list" {
            url.query_pairs_mut().append_pair(
                "task_id",
                body["task_id"].as_str().ok_or("Task id required")?,
            );
        }
        let is_get = method == Method::GET;
        let mut request = self.http.request(method, url).bearer_auth(&self.token);
        if !is_get {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| "Dispatch service unreachable")?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("Sign in again on Sync to use Dispatch".into());
        }
        if !response.status().is_success() {
            return Err(format!(
                "Dispatch request failed ({})",
                response.status().as_u16()
            ));
        }
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        response
            .json()
            .await
            .map_err(|_| "Invalid Dispatch response".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_insecure_or_credentialed_urls() {
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com?secret=1",
        ] {
            assert!(DispatchClient::new(url, "token".into()).is_err());
        }
        assert!(DispatchClient::new(DEFAULT_BASE_URL, "token".into()).is_ok());
    }
}
