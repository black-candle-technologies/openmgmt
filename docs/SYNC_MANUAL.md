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

This step does not add a background sync loop or a full settings interface.
Real authentication, advanced conflict resolution, and multi-user task
requests are also not implemented yet.
