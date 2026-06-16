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
    let writes_enabled = std::env::var("OPENMGMT_MCP_WRITE_ENABLED")
        .is_ok_and(|value| value.eq_ignore_ascii_case("true"));
    let server = OpenMgmtMcp::new(AppService::new(database), writes_enabled);

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
