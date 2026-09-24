# OpenMGMT MCP over HTTPS (issue #16)

The `openmgmt-mcp` binary has two transports. `stdio` (the default) is for
local editor/assistant clients. `OPENMGMT_MCP_TRANSPORT=http` serves the same
tool registry over MCP streamable HTTP at `POST /mcp`, so a remote assistant
— e.g. Muse over the public internet — can manage any synced OpenMGMT
replica.

Writes go through `AppService`/`Database` exactly like local writes: they
emit sync events and converge with other replicas normally.

## Quick start (loopback)

```sh
OPENMGMT_MCP_TRANSPORT=http \
OPENMGMT_MCP_BIND_ADDR=127.0.0.1:8788 \
OPENMGMT_MCP_AUTH_ISSUER="" \
  openmgmt-mcp
```

`OPENMGMT_MCP_AUTH_ISSUER=""` disables bearer auth — only ever do this on
loopback. With auth enabled (the default), every `/mcp` request needs:

```
Authorization: Bearer <Black Candle access token>
```

Tokens are validated against the issuer's `/oauth/userinfo` endpoint using
the same `AccountAuth` validator as sync registration (#14); the stable
authd user id becomes the audit caller. `/health` stays unauthenticated.

## Environment

| Variable | Default | Notes |
|---|---|---|
| `OPENMGMT_MCP_TRANSPORT` | `stdio` | `http` enables this transport |
| `OPENMGMT_MCP_BIND_ADDR` | `127.0.0.1:8788` | Plain HTTP; TLS is the proxy's job |
| `OPENMGMT_MCP_AUTH_ISSUER` | `https://auth.blackcandletech.com` | Shared default with the sync server; empty disables auth |
| `OPENMGMT_MCP_WRITE_ENABLED` | `true` (http) / `false` (stdio) | Per-launcher write gate; the persisted AI settings still apply |
| `OPENMGMT_MCP_RATE_LIMIT_PER_MINUTE` | `120` | Per-IP fixed window |
| `OPENMGMT_MCP_ALLOWED_HOSTS` | rmcp loopback default | Comma-separated `Host` allow-list, e.g. `mcp.example.com` |

## Remote permission model

The default remote permission set is **reads plus non-destructive writes**,
subject to the #15 AI settings (`AiSettings.read_enabled` /
`write_enabled` still gate the registry, and `OPENMGMT_MCP_WRITE_ENABLED`
remains a kill switch). Destructive tools are **never exposed remotely**,
even if the persisted settings would allow them.

## Rate limiting and audit

- Per-IP fixed-window rate limiting (`429` when exceeded).
- Every tool call and auth decision is appended to the `mcp_audit_log`
  table in the replica's database (caller, timestamp, tool, success) —
  local to the replica, never synced:

```sql
SELECT called_at, caller, tool_name, success
FROM mcp_audit_log ORDER BY id DESC LIMIT 20;
```

## Public HTTPS via Caddy

Terminate TLS at the edge; the MCP server keeps speaking plain HTTP on
loopback:

```caddy
mcp.example.com {
    reverse_proxy 127.0.0.1:8788
}
```

Set `OPENMGMT_MCP_ALLOWED_HOSTS=mcp.example.com` so rmcp's `Host`
validation (DNS-rebinding protection) accepts the public name.

## Client configuration

Any MCP client with streamable-HTTP support can point at
`https://mcp.example.com/mcp` with an `Authorization: Bearer <token>`
header. The token is a Black Candle OAuth access token (1h lifetime —
refresh it via your normal authd flow); it is validated on each call
(5-minute cache) but never stored.

## API key authentication (issue #38)

The OAuth flow (DCR + PKCE + loopback redirect + rotating refresh tokens)
is built for interactive clients like the Android app. Agent and machine
clients — and agent-connector flows, which accept exactly one credential
shape, a single static key — cannot use it. For those, the MCP server
accepts long-lived API keys as an alternative credential type. The OAuth
path is unchanged.

### Creating a key

Key management is a local operator action on the machine running
`openmgmt-mcp` — it is deliberately not exposed over the network:

```sh
# Read-only key (the default)
openmgmt-mcp apikey create --name "ci-runner"

# Read/write key
openmgmt-mcp apikey create --name "agent" --scopes tasks:read,tasks:write

openmgmt-mcp apikey list
openmgmt-mcp apikey revoke <id>
```

`create` prints the secret once — store it immediately:

```
API key created. The secret is shown once — store it now:

  omg_live_5Y-oCoJ7iF3sfqx_bA09Qefy-X_Ba1dphsU0sry_-TI

  id:     8a8a09f1-78be-4d38-af20-0eba7bb44770
  name:   agent
  prefix: omg_live_5Y-oCoJ
  scopes: tasks:read, tasks:write
```

### Using a key

Send it as the Bearer <redacted>, exactly like an OAuth token:

```
Authorization: Bearer omg_live_5Y-oCoJ7iF3sfqx_bA09Qefy-X_Ba1dphsU0sry_-TI
```

The server routes any Bearer <redacted> starting with `omg_live_` to API-key
validation; everything else keeps going through `AccountAuth` unchanged.

### Scopes

- `tasks:read` — read tools only (`list_tasks`, `query_tasks`, …).
- `tasks:write` — non-destructive write tools (`create_task`, `update_task`,
  …); implies read.

A key without `tasks:write` that calls a write tool gets `403 Forbidden`
and the attempt is audited. Scope classification reuses the #15 AI tool
registry, so scopes cannot drift from the registry. Destructive tools stay
unavailable remotely regardless of scope.

### Storage, revocation, audit

- Only the SHA-256 hash of a key is stored (`mcp_api_keys`), same pattern
  authd uses for tokens. The plaintext is never written to disk.
- Keys do not expire; revoke them with `openmgmt-mcp apikey revoke <id>`.
  Revoked keys are rejected with `401` naming the key id.
- Every tool call and auth decision is appended to `mcp_audit_log` with the
  caller recorded as `api-key:<id>` — the key itself never appears there:

```sql
SELECT called_at, caller, tool_name, success
FROM mcp_audit_log WHERE caller LIKE 'api-key:%' ORDER BY id DESC LIMIT 20;
```

### Custom-connector usage

Paste the key into the connector's hosted connect page as the single API
key. No request signing, no key/secret pair, no session handling — the
server needs only `Authorization: Bearer <key>` on each request.
