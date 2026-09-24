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

## Token scopes

Both interactive OAuth tokens and personal access tokens (minted in the
website's **Dashboard → Tokens** section) authenticate through the same
Black Candle userinfo path. The token's granted scope is enforced per
request, before tool dispatch:

| Scope | MCP access |
|---|---|
| `identity` (or empty — pre-scope issuers) | Full access. Every existing OAuth client behaves exactly as before. |
| `openmgmt:tasks:read` | Read-only tools (`list_tasks`, `query_tasks`, `get_board_state`, …). |
| `openmgmt:tasks:write` | All tools (write implies read). |
| anything else (e.g. `courier:messages:read`) | No MCP tool access — every call is rejected with `403`. |

Multi-scope tokens (space-delimited per RFC 6749) grant the union of
their scopes. Read/write classification reuses the #15 AI tool registry,
so scopes can never drift from it. Scope denials are recorded in
`mcp_audit_log` with caller, scope, and tool (never token material).

Known caveat: successful userinfo validations are cached for five
minutes, so revoking a personal token (or narrowing its scope) takes up
to five minutes to reach the MCP server.

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
