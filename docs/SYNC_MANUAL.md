# Manual Desktop Sync

OpenMGMT remains local-first. Sync is optional, and the desktop application
continues to work entirely from its local SQLite database when sync is disabled
or no server is configured.

The Tauri desktop shell exposes three manual sync commands:

- `sync_now` runs one OMGP/1 device registration, push, pull, and remote replay
  cycle. A second concurrent manual sync is rejected with
  `sync is already running`.
- `test_sync_connection` sends only the OMGP/1 hello request and verifies
  protocol compatibility. It does not register the device, push events, or
  pull events.
- `clear_sync_error` clears the locally stored sync error without changing the
  configured server or disabling sync.

The desktop UI exposes these actions from the **Sync** sidebar page. See
[`SYNC_UI.md`](SYNC_UI.md) for the manual verification checklist.

## Signing in

When the sync server enables account auth, device registration requires a
Black Candle account. The desktop app signs in with the native OAuth flow
(RFC 8252): it opens the system browser at the configured issuer, the user
logs in (or creates an account), and the issuer redirects back to a
loopback callback with an authorization code. The app exchanges the code
with PKCE S256 and stores the access token in the OS keychain — never in
the OpenMGMT SQLite database.

The access token is only presented at device-registration time; normal
push/pull sync keeps using the device token, so sync never depends on the
issuer being reachable. If the server ever stops recognizing the device
token (for example its database was recreated), the client re-registers
once per sync run, proving possession with the stored device token.

Self-hosters can point the app at their own issuer; the client identifies
itself with a CIMD client-metadata document (RFC 9728), so any OIDC
provider works.

There is still no background sync loop. Advanced conflict resolution and
multi-user task requests are also not implemented yet.
