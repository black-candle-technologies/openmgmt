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

Always enabled:

- `list_organizations`
- `list_projects`
- `get_project`
- `list_tasks`
- `get_task`
- `get_board_state`
- `get_today_plan`

Disabled and hidden unless `OPENMGMT_MCP_WRITE_ENABLED=true`:

- `create_task`
- `update_task`
- `complete_task`
- `create_project`

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
trusted MCP bridge or custom app. OpenMgmt does not expose a network bridge in
the MVP.

## Enable writes

For a trusted PowerShell session:

```powershell
$env:OPENMGMT_MCP_WRITE_ENABLED = "true"
cargo run -p openmgmt-mcp
```

Writes permit creation and updates only. Archive and delete operations remain
unavailable through MCP.
