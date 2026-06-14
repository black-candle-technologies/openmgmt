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

## Multi-User Direction

Sync events retain `actor_user_id`, `target_user_id`, and `workspace_id`.
Future task requests between users should preserve requester and target
semantics: one user may submit a request for another user's review, but must not
directly insert or mutate tasks in that user's schedule without acceptance.
