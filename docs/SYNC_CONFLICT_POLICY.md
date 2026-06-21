# Sync Conflict Policy

OpenMGMT remains local-first. Sync conflict handling is deterministic, optional, and defined in Rust first so the policy is type-safe, testable, and easy to expose through TOML or UI later.

## Current Default Policy

Organizations/projects:
- Normal edits use deterministic last-write-wins by server event order.
- Archive wins over normal update.
- Restore requires an explicit restore event.

Tasks:
- Normal edits use deterministic last-write-wins by server event order.
- Explicit status transitions follow server event order.
- Done/canceled/archived are protected from stale normal updates.
- Archive wins over normal update.
- Restore requires an explicit restore event.

Tags/labels:
- Task tags exist today, but conflict-specific tag merging is not implemented yet.
- Future policy should merge tag sets instead of overwriting the whole field.

## Code-Defined First

The policy lives in `openmgmt-core` as Rust structs and enums. That keeps replay behavior centralized, makes unsupported combinations visible in tests, and avoids adding a user-editable config format before the rules are stable.

The policy types derive `Serialize` and `Deserialize`, so they can later be loaded from config without changing the replay API.

## Future TOML Shape

```toml
[organization]
normal_update = "last_write_wins_whole_entity"
archive_vs_update = "archive_wins"
restore_behavior = "explicit_restore_only"

[project]
normal_update = "last_write_wins_whole_entity"
archive_vs_update = "archive_wins"
restore_behavior = "explicit_restore_only"

[task]
normal_update = "last_write_wins_whole_entity"
status_update = "server_order_wins"
terminal_status_behavior = "protect_done_canceled_archived"
archive_vs_update = "archive_wins"
restore_behavior = "explicit_restore_only"
```

No TOML loading is implemented yet.

## Conflict Records

Conflict records are stored in `sync_conflicts`. Each record includes the remote event ID, local device ID, entity type and ID, conflict kind, policy action, optional local and remote JSON snapshots, resolution status, creation time, and optional resolution time.

Open conflicts can be queried from core and Tauri. They can also be marked ignored. There is no full conflict-resolution UI yet.

## Open vs Auto-Resolved

Current default policy records human-reviewable conflicts as `open`:
- local unsynced change vs remote update
- archive vs update
- terminal task status protected from stale normal update

The schema and result counters support `auto_resolved`, but the default policy currently prefers open records for review when a risky condition is detected.

## Limitations

- Event payloads are currently whole-entity snapshots, so `LastWriteWinsPerField` is defined for future use but falls back to deterministic whole-entity behavior where per-field tracking is unavailable.
- Restore is not implemented as a sync operation yet. Normal updates do not restore archived entities under the default policy.
- Task archived state is represented by terminal status behavior because tasks do not currently have an `archived_at` column.
- Conflict records are queryable, but there is no dedicated UI for resolving them yet.
