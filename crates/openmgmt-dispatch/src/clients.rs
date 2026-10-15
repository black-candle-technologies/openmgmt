use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex, time::Duration};

pub fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?)
}

// async-trait adds #[must_use] to an already must-use boxed Future.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Auth: Send + Sync {
    async fn subject(&self, token: &str) -> Result<String>;
}
pub struct Authd {
    pub client: Client,
    pub base: String,
    pub require_scope: bool,
}
#[async_trait]
impl Auth for Authd {
    async fn subject(&self, token: &str) -> Result<String> {
        let v: Value = self
            .client
            .get(format!(
                "{}/oauth/userinfo",
                self.base.trim_end_matches('/')
            ))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let scopes = v.get("scope").or_else(|| v.get("scopes"));
        if let Some(scopes) = scopes {
            tracing::info!(%scopes, "authd token scopes");
        }
        let allowed = scopes.is_some_and(|s| {
            s.as_str()
                .is_some_and(|s| s.split_whitespace().any(|s| s == "openmgmt:dispatch"))
                || s.as_array()
                    .is_some_and(|a| a.iter().any(|s| s == "openmgmt:dispatch"))
        });
        if self.require_scope && !allowed {
            bail!("missing dispatch scope");
        }
        Ok(v["sub"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("missing subject")?
            .to_owned())
    }
}

#[allow(clippy::double_must_use)]
#[async_trait]
pub trait CpClient: Send + Sync {
    async fn call(&self, method: &str, args: Value) -> Result<Value>;
}
pub struct HttpCpClient {
    client: Client,
    url: String,
    credential: String,
}
impl HttpCpClient {
    pub fn new(base: &str, credential: String) -> Result<Self> {
        let mut url = reqwest::Url::parse(base)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            bail!("CP_EXEC_URL must be an HTTPS URL without userinfo");
        }
        if url.path() == "/" {
            url.set_path("/v1/execute");
        }
        Ok(Self {
            client: http_client()?,
            url: url.to_string(),
            credential,
        })
    }
}
#[async_trait]
impl CpClient for HttpCpClient {
    async fn call(&self, method: &str, args: Value) -> Result<Value> {
        if !["start", "status", "resume", "cancel"].contains(&method) {
            bail!("unsupported Pi method");
        }
        let code = format!("pi.{method}({args})");
        let v: Value = self
            .client
            .post(&self.url)
            .bearer_auth(&self.credential)
            .json(&json!({"code":code,"modules":["pi"]}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if v.get("error").is_some_and(|e| !e.is_null()) {
            bail!("Control Plane execution failed");
        }
        v.get("result")
            .filter(|v| !v.is_null())
            .cloned()
            .context("missing Control Plane result")
    }
}

#[derive(Default)]
pub struct FakeCpClient {
    pub replies: Mutex<VecDeque<Value>>,
    pub calls: Mutex<Vec<(String, Value)>>,
}
#[async_trait]
impl CpClient for FakeCpClient {
    async fn call(&self, method: &str, args: Value) -> Result<Value> {
        self.calls
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned fake"))?
            .push((method.into(), args));
        self.replies
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned fake"))?
            .pop_front()
            .context("no fake response")
    }
}
