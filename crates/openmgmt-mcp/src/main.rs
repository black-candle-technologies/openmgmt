mod http;
mod sync_publish;
mod tools;

use anyhow::Context;
use openmgmt_core::{AppService, Database, default_database_path};
use rmcp::{ServiceExt, transport::stdio};
use tools::OpenMgmtMcp;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let database = Database::open(default_database_path()).context("open database")?;
    // Publish this process's sync outbox into the server event log so MCP
    // writes reach syncing devices. No-op unless the database is shared with
    // the sync server.
    let _sync_publisher = sync_publish::spawn_sync_publisher(database.clone());
    let service = AppService::new(database);

    // stdio (local editor/assistant clients) is the default; `http` serves
    // the same registry over streamable HTTP for remote access (issue #16).
    let transport = std::env::var("OPENMGMT_MCP_TRANSPORT").unwrap_or_default();
    if transport.eq_ignore_ascii_case("http") {
        let config = http::McpHttpConfig::from_env().context("MCP HTTP config")?;
        return http::serve_http(&config, service).await;
    }

    // Historical stdio default: writes off unless explicitly enabled.
    let writes_enabled = std::env::var("OPENMGMT_MCP_WRITE_ENABLED")
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    let server = OpenMgmtMcp::new(service, writes_enabled);

    tracing::info!(writes_enabled, "starting OpenMgmt MCP server");
    server
        .serve(stdio())
        .await
        .context("start MCP transport")?
        .waiting()
        .await
        .context("MCP server stopped")?;
    Ok(())
}
