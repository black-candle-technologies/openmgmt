mod config;
mod error;
mod routes;
mod state;
mod store;

use anyhow::Context;
use config::ServerConfig;
use openmgmt_protocol::AccountAuth;
use state::AppState;
use std::sync::Arc;
use store::ServerStore;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = ServerConfig::from_env();
    let store = ServerStore::open(&config.database_path).context("open server database")?;
    let account_auth = config
        .auth_issuer
        .as_deref()
        .map(AccountAuth::new)
        .transpose()
        .context("configure account auth")?
        .map(Arc::new);
    if config.account_auth_required() {
        tracing::info!(
            issuer = config.auth_issuer.as_deref().unwrap_or_default(),
            "device registration requires a Black Candle account"
        );
    } else {
        tracing::warn!("account auth disabled: device registration is open");
    }
    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .with_context(|| format!("bind {}", config.bind_addr))?;
    let local_addr = listener.local_addr().context("read local address")?;
    tracing::info!(
        bind_addr = %local_addr,
        database_path = %config.database_path.display(),
        "starting OpenMGMT sync server"
    );

    let app = routes::router(AppState {
        config: Arc::new(config),
        store,
        account_auth,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("serve OpenMGMT sync server")
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal handler");
    }
}
