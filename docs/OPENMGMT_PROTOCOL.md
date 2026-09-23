# OpenMGMT Protocol

OpenMGMT Protocol version 1 is identified as `OMGP/1` and serialized as the
version string `omgp/1`.

## Principles

OpenMGMT remains local-first. The local database and application continue to
work without a server. Sync exchanges events rather than SQLite database files.

The protocol message types are transport-agnostic. Future implementations may
carry the same messages over HTTP, WebSocket, local IPC, or another transport
without changing their serialized shapes.

## Initial Messages

- hello request and response
- device registration request and response
- sync push request and response
- sync pull request and response
- protocol error

Auth context and device-token fields are modeled for future compatibility, but
authentication, token issuance, sessions, and permission enforcement are not
implemented.

Server and client networking are not implemented in this step.

## Local Sync Settings

Sync is optional. OpenMGMT continues to operate in local-first mode when sync is
disabled or no server is configured.

The stored server URL may point to a local server, such as
`http://127.0.0.1:8787`, or a cloud-hosted server. This step stores settings and
reports local status only; it does not perform network requests or enforce
authentication.

The local status model supports:

- `disabled`
- `not_configured`
- `ready`
- `syncing`
- `error`

`syncing` is reserved for a future sync runner and is not currently produced.

## Initial Sync Server

The optional `openmgmt-server` binary provides the first OMGP/1 event server.
It runs locally by default:

- bind address: `127.0.0.1:8787`
- database: `data/openmgmt-server.sqlite`

The server exposes:

- `GET /health`
- `POST /omgp/v1/hello`
- `POST /omgp/v1/devices/register`
- `POST /omgp/v1/sync/push`
- `POST /omgp/v1/sync/pull`

The server database stores registered devices and sync events only. It does not
reuse the desktop database or directly mutate organizations, projects, or
tasks. The desktop app remains fully local-first and does not require the
server.

## Account Authentication

Device registration is gated on a Black Candle account. The client sends its
OAuth access token as a `Bearer` credential on
`POST /omgp/v1/devices/register`; the server validates it against the
configured issuer's `/oauth/userinfo` endpoint (the same validation used by
the HTTPS MCP transport) and binds the device to the stable authd user id —
never the email address.

- New devices are registered to the authenticated account.
- Re-registering an existing device id requires proof of possession: either
  the previous device token (`previous_device_token`) or a bearer token for
  the owning account. A different account without the device token is denied.
- Devices registered before account auth was enabled can only be claimed
  by presenting their current device token; an arbitrary signed-in
  account cannot take them over.
- Push is restricted to events stamped with the authenticated device id;
  mismatched events are rejected.
- Pull is scoped to the authenticated account: a device only sees events
  pushed by devices registered to the same account.

The issuer is configurable for self-hosters via `OPENMGMT_AUTH_ISSUER`
(default `https://auth.blackcandletech.com`). Setting it to an empty value
disables account auth entirely (open registration — only sensible on
loopback). Local-only OpenMGMT remains account-free; auth applies when sync
is enabled.

Normal sync traffic keeps using device tokens after registration, so the
issuer is not contacted on every push/pull. Successful userinfo validations
are cached for five minutes.

Background sync and domain conflict resolution are not implemented yet.

## Manual Sync Client

The `openmgmt-sync-client` crate provides a manual, one-shot `sync_once`
operation. It reads local settings, negotiates OMGP/1, registers the device when
needed, pushes pending local events, and pulls server events.

The client does not run in the background. The desktop Tauri shell exposes a
`sync_now` command, but this step does not add a settings or sync UI.

Pulled organization, project, and task events are replayed into the local
SQLite database in server order. Applied remote events are deduplicated by
`event_id`. Events from the local device are treated as echoes and do not
mutate domain rows.

Replay writes never create new local sync events. Each domain write and its
`applied_remote_events` marker are committed in the same transaction. The
client advances its server checkpoint only after the entire pulled batch is
successfully replayed; a failed dependency or malformed payload leaves the
checkpoint unchanged for retry.

Conflict behavior is currently deterministic last-write-wins in server event
order. Project events require their organization to exist, and task events
require their project to exist. Advanced conflict resolution is not yet
implemented.

The desktop app remains fully local-first and continues to work when sync is
disabled or no server is available.

## Multi-User Direction

Sync events retain `actor_user_id`, `target_user_id`, and `workspace_id`.
Future task requests between users should preserve requester and target
semantics: one user may submit a request for another user's review, but must not
directly insert or mutate tasks in that user's schedule without acceptance.
