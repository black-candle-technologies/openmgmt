use anyhow::Context;
use openmgmt_core::{AppService, Database};
use openmgmt_gpt_bridge::{BridgeState, GptBridgeConfig, router};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = GptBridgeConfig::from_env()?;
    let database = Database::open(&config.database_path).context("open OpenMgmt database")?;
    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.bind_addr))?;
    let local_addr = listener.local_addr().context("read local address")?;
    if !local_addr.ip().is_loopback() {
        tracing::warn!(
            bind_addr = %local_addr,
            "GPT bridge is bound to a non-loopback address and may be reachable from other hosts; \
             ensure it is only exposed over authenticated HTTPS"
        );
        println!(
            "WARNING: OpenMgmt GPT bridge is bound to {local_addr}, which is not localhost. \
             Only expose it through an authenticated HTTPS tunnel."
        );
    }
    tracing::info!(
        bind_addr = %local_addr,
        database_path = %config.database_path.display(),
        write_enabled = config.write_enabled,
        "starting OpenMgmt GPT Action bridge"
    );
    println!("OpenMgmt GPT Action bridge listening on http://{local_addr}");

    let app = router(BridgeState {
        config: Arc::new(config),
        service: AppService::new(database),
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve OpenMgmt GPT Action bridge")
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal handler");
    }
}
