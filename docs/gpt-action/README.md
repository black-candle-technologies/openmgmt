# OpenMgmt GPT Action Bridge

The GPT Action bridge is a small authenticated HTTP API that lets a custom GPT inspect OpenMgmt data and, when explicitly enabled, create organizations, projects, and tasks, and update, start, complete, or block tasks.

It reuses the OpenMgmt core database and service layer. It opens the same repository-local SQLite database path as the desktop app unless `OPENMGMT_DATABASE_PATH` is set. It does not seed sample data.

## Run Locally

From the repository root:

```powershell
$env:OPENMGMT_GPT_API_TOKEN = "replace-with-a-long-random-token"
$env:OPENMGMT_GPT_WRITE_ENABLED = "false"
cargo run -p openmgmt-gpt-bridge
```

Default bind address:

```text
127.0.0.1:8790
```

Override it with:

```powershell
$env:OPENMGMT_GPT_BIND = "127.0.0.1:8790"
```

Override the database path with:

```powershell
$env:OPENMGMT_DATABASE_PATH = "C:\path\to\openmgmt.sqlite"
```

## Environment Variables

`OPENMGMT_GPT_API_TOKEN`

Required bearer token for all `/api/openmgmt/*` calls.

`OPENMGMT_GPT_WRITE_ENABLED`

Defaults to `false`. Set to `true` to allow task write endpoints.

`OPENMGMT_GPT_BIND`

Defaults to `127.0.0.1:8790`. Keep this on a loopback address. Binding to `0.0.0.0` makes the bridge reachable from other machines on your network; only do this behind an authenticated HTTPS tunnel, and the bridge prints a warning when it is bound to a non-loopback address.

`OPENMGMT_DATABASE_PATH`

Optional SQLite database path override.

`OPENMGMT_GPT_CORS_ORIGIN`

Optional restrictive CORS origin. Wildcard CORS is not enabled by default.

## GPT Action Setup

1. Run the bridge locally.
2. Expose it over a reachable HTTPS URL if ChatGPT needs to call it. ChatGPT cannot call private `localhost` directly.
3. In the custom GPT builder, add an Action.
4. Import `docs/gpt-action/openapi.yaml`.
5. Replace the server placeholder with your bridge URL.
6. Configure bearer authentication with the same token as `OPENMGMT_GPT_API_TOKEN`.
7. Add the contents of `docs/gpt-action/GPT_INSTRUCTIONS.md` to the GPT instructions.

## Manual Checks

```powershell
curl http://127.0.0.1:8790/health
curl -H "Authorization: Bearer replace-with-a-long-random-token" http://127.0.0.1:8790/api/openmgmt/summary
curl -H "Authorization: Bearer replace-with-a-long-random-token" http://127.0.0.1:8790/api/openmgmt/board
curl -H "Authorization: Bearer replace-with-a-long-random-token" http://127.0.0.1:8790/api/openmgmt/today
```

Write-disabled behavior:

```powershell
curl -X POST http://127.0.0.1:8790/api/openmgmt/tasks `
  -H "Authorization: Bearer replace-with-a-long-random-token" `
  -H "Content-Type: application/json" `
  -d "{}"
```

With `OPENMGMT_GPT_WRITE_ENABLED=false`, the expected response is `403`.

## Example Prompts

- "Plan my day from OpenMgmt."
- "Show blocked tasks."
- "What should I work on next?"
- "Summarize active projects."
- "Show the current board."
- "Create a P2 task in project X."
- "Create an organization called Acme."
- "Create a project named Launch in Acme."
- "Complete task `<id>`."
- "Start task `<id>`."
- "Block task `<id>` because I am waiting on vendor feedback."

## Current Limitations

- Create-only for organizations and projects: the bridge can create them but cannot rename, archive, or delete them.
- The bridge does not expose any destructive operations (no delete, archive, backup, restore, or arbitrary SQL).
- Task updates do not reassign tasks to another project.
- ChatGPT requires a reachable HTTPS bridge URL; private localhost is not enough.
