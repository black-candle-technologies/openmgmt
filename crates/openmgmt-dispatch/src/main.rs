use anyhow::{Context, Result, bail};
use openmgmt_dispatch::{
    App, DEFAULT_MODELS,
    clients::{Authd, HttpCpClient, http_client},
    prompts::MiniMax,
    registry::Registry,
    router, watch,
};
use std::{env, sync::Arc, time::Duration};
fn setting(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.into())
}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(setting("RUST_LOG", "openmgmt_dispatch=info"))
        .init();
    let models = match env::var("MODELS_JSON") {
        Ok(path) => std::fs::read_to_string(path)?,
        Err(_) => DEFAULT_MODELS.into(),
    };
    let models: serde_json::Value = serde_json::from_str(&models)?;
    let entries = models.as_array().context("catalog must be an array")?;
    for m in entries {
        if !m["provider"].is_string()
            || !m["model"].is_string()
            || !m["label"].is_string()
            || !m["available"].is_boolean()
        {
            bail!("invalid model catalog entry");
        }
    }
    let require_scope = setting("REQUIRE_DISPATCH_SCOPE", "false").parse::<bool>()?;
    let interval = setting("POLL_INTERVAL_SECS", "15").parse::<u64>()?;
    if interval == 0 {
        bail!("poll interval must be positive");
    }
    let app = App {
        db: Arc::new(Registry::open(&setting(
            "DISPATCH_DB",
            "./dispatch.sqlite",
        ))?),
        auth: Arc::new(Authd {
            client: http_client()?,
            base: setting("AUTHD_BASE", "https://auth.blackcandletech.com"),
            require_scope,
        }),
        cp: Arc::new(HttpCpClient::new(
            &setting("CP_EXEC_URL", "https://mcp.internal.blackcandletech.com"),
            env::var("CP_SERVICE_CREDENTIAL").context("CP_SERVICE_CREDENTIAL required")?,
        )?),
        generator: Arc::new(MiniMax {
            client: http_client()?,
            base: setting("MINIMAX_BASE", "https://api.minimax.io/v1"),
            model: setting("MINIMAX_MODEL", "MiniMax-M3"),
            key: env::var("MINIMAX_API_KEY").ok().filter(|s| !s.is_empty()),
        }),
        models,
        lifecycle: Arc::new(tokio::sync::Mutex::new(())),
    };
    let listener = tokio::net::TcpListener::bind(format!(
        "{}:{}",
        setting("BIND", "127.0.0.1"),
        setting("PORT", "8790")
    ))
    .await?;
    let watcher = tokio::spawn(watch(app.clone(), Duration::from_secs(interval)));
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                if let Ok(mut term) =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                {
                    tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
                } else {
                    let _ = tokio::signal::ctrl_c().await;
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
        })
        .await?;
    watcher.abort();
    Ok(())
}
