# AI Integration

OpenMgmt includes a separate stdio MCP server built with rmcp. It opens the same
SQLite database as the desktop application; no HTTP API or cloud account is
required.

## Run

```powershell
cargo build -p openmgmt-mcp --release
cargo run -p openmgmt-mcp
```

The server writes protocol messages to stdout and logs to stderr.

## Tools

Tool availability is governed by the core AI permission model
(`openmgmt-core/src/ai.rs`): the persisted `AiSettings` (read/write/
destructive toggles, shared with the desktop app's AI settings) plus the
per-launcher `OPENMGMT_MCP_WRITE_ENABLED` env gate. Writes require both.

Always enabled (read tools):

- `list_organizations`
- `list_projects`
- `get_project`
- `list_tasks`
- `get_task`
- `query_tasks` (filtered/sorted task queries)
- `get_board_state`
- `get_today_plan`
- `plan_today`
- `suggest_next_task`
- `triage_backlog`
- `summarize_project`
- `list_saved_task_views`
- `list_timer_sessions`
- `get_scoring_settings`

Disabled and hidden unless `OPENMGMT_MCP_WRITE_ENABLED=true`:

- `create_task`
- `update_task`
- `complete_task`
- `create_project`
- `start_task_timer`
- `pause_task_timer`
- `resume_task_timer`
- `stop_task_timer`

The MVP exposes no destructive delete or archive tools.

## Claude Desktop

Build the release binary, then add it to Claude Desktop's MCP configuration.
Replace the path with the absolute path to this clone:

```json
{
  "mcpServers": {
    "openmgmt": {
      "command": "C:\\Users\\YOUR_NAME\\openmgmt\\target\\release\\openmgmt-mcp.exe",
      "env": {
        "OPENMGMT_DATABASE_PATH": "C:\\Users\\YOUR_NAME\\openmgmt\\data\\openmgmt.sqlite",
        "OPENMGMT_MCP_WRITE_ENABLED": "false"
      }
    }
  }
}
```

Restart Claude Desktop after changing its configuration. Set the database path
explicitly because desktop clients may start MCP processes in a different
working directory.

## ChatGPT-compatible clients

ChatGPT integration depends on the MCP or custom-app support available in the
specific deployment. Point a supported local MCP launcher or bridge at the
compiled `openmgmt-mcp` binary and set `OPENMGMT_DATABASE_PATH`.

Some hosted clients cannot directly start a local stdio process. They require a
trusted MCP bridge or custom app. For remote access, `openmgmt-mcp` also
serves the registry over MCP streamable HTTP — see [MCP_HTTP.md](MCP_HTTP.md).

## Enable writes

For a trusted PowerShell session:

```powershell
$env:OPENMGMT_MCP_WRITE_ENABLED = "true"
cargo run -p openmgmt-mcp
```

Writes permit creation and updates only. Archive and delete operations remain
unavailable through MCP.
