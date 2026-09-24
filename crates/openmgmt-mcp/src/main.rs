mod http;
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

    // Operator key management (issue #38) runs before anything else: it must
    // not start any transport.
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("apikey") {
        return apikey_cli(args);
    }

    let database = Database::open(default_database_path()).context("open database")?;
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

/// Manage MCP API keys (issue #38):
///
/// ```sh
/// openmgmt-mcp apikey create --name <name> [--scopes tasks:read,tasks:write]
/// openmgmt-mcp apikey list
/// openmgmt-mcp apikey revoke <id>
/// ```
///
/// Key management is a local operator action on purpose: it never widens the
/// remote write surface. The plaintext secret is printed once, at creation.
fn apikey_cli(args: impl Iterator<Item = String>) -> anyhow::Result<()> {
    let database = Database::open(default_database_path()).context("open database")?;
    let mut args = args.peekable();
    match args.next().as_deref() {
        Some("create") => apikey_create(&database, &mut args),
        Some("list") => apikey_list(&database),
        Some("revoke") => apikey_revoke(&database, &mut args),
        Some(other) => {
            anyhow::bail!("unknown apikey subcommand '{other}'; expected create, list, or revoke")
        }
        None => anyhow::bail!("missing apikey subcommand; expected create, list, or revoke"),
    }
}

fn apikey_create(
    database: &Database,
    args: &mut impl Iterator<Item = String>,
) -> anyhow::Result<()> {
    let mut name: Option<String> = None;
    let mut scopes_raw: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => {
                name = Some(args.next().context("missing value for --name")?);
            }
            "--scopes" => {
                scopes_raw = Some(args.next().context("missing value for --scopes")?);
            }
            other => anyhow::bail!("unexpected argument '{other}'"),
        }
    }
    let name = name
        .filter(|value| !value.trim().is_empty())
        .context("--name <name> is required")?;
    // Least privilege by default: read-only unless the operator opts into
    // write explicitly.
    let scopes = parse_scopes(scopes_raw.as_deref().unwrap_or("tasks:read"))?;
    let (key, plaintext) = database.create_api_key(&name, &scopes)?;
    println!("API key created. The secret is shown once — store it now:\n");
    println!("  {plaintext}\n");
    println!("  id:     {}", key.id);
    println!("  name:   {}", key.name);
    println!("  prefix: {}", key.key_prefix);
    println!(
        "  scopes: {}",
        scopes
            .iter()
            .map(openmgmt_core::ApiKeyScope::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

fn parse_scopes(raw: &str) -> anyhow::Result<Vec<openmgmt_core::ApiKeyScope>> {
    let mut scopes = Vec::new();
    for part in raw.split(',') {
        if part.trim().is_empty() {
            continue;
        }
        match openmgmt_core::ApiKeyScope::parse(part) {
            Some(scope) => scopes.push(scope),
            None => anyhow::bail!("unknown scope '{part}'; valid scopes: tasks:read, tasks:write"),
        }
    }
    if scopes.is_empty() {
        anyhow::bail!("at least one scope is required; valid scopes: tasks:read, tasks:write");
    }
    Ok(scopes)
}

fn apikey_list(database: &Database) -> anyhow::Result<()> {
    let keys = database.list_api_keys()?;
    if keys.is_empty() {
        println!("no API keys");
        return Ok(());
    }
    println!(
        "{:<36} {:<24} {:<18} {:<24} STATUS",
        "ID", "NAME", "PREFIX", "SCOPES"
    );
    for key in keys {
        let status = match key.revoked_at {
            Some(at) => format!("revoked {}", at.format("%Y-%m-%d %H:%M UTC")),
            None => "active".to_string(),
        };
        println!(
            "{:<36} {:<24} {:<18} {:<24} {}",
            key.id,
            truncate(&key.name, 24),
            key.key_prefix,
            key.scopes
                .iter()
                .map(openmgmt_core::ApiKeyScope::as_str)
                .collect::<Vec<_>>()
                .join(","),
            status
        );
    }
    Ok(())
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max - 1).collect::<String>())
    }
}

fn apikey_revoke(
    database: &Database,
    args: &mut impl Iterator<Item = String>,
) -> anyhow::Result<()> {
    let id = args
        .next()
        .context("usage: openmgmt-mcp apikey revoke <id>")?;
    if database.revoke_api_key(&id)? {
        println!("revoked API key {id}");
        Ok(())
    } else {
        anyhow::bail!("no active API key with id '{id}'")
    }
}
