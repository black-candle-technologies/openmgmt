# OpenMgmt MVP Product Requirements

## Purpose

OpenMgmt gives one operator a fast, private view of work spread across
organizations and project types. It should answer: what is active, what is
urgent, what is blocked, and what should happen next?

## Core workflows

1. Create and maintain organizations.
2. Create projects categorized by organization and work type.
3. Capture, schedule, start, block, unblock, complete, or cancel project tasks.
4. Track elapsed time for active tasks and display configured time limits.
5. Review a scored Daily Operations view (the daily planning page that replaced
   the former Today page).
6. Open a dedicated fullscreen board on a television or second monitor.
7. Let an MCP-capable assistant inspect work and optionally create or update it.

## Board

The board contains NOW, NEXT UP, DUE SOON, WAITING / BLOCKED, LATER TODAY,
OVERDUE, and DONE TODAY.

Classification uses task status and dates. Each column is then sorted by urgency
score. Priority, project priority, pinned state, overdue dates, near due dates,
and active work increase urgency. Blocked, waiting, and paused-project work
receive penalties. Done and canceled tasks are excluded except for work
completed today.

## Constraints

- Single local operator; no authentication
- SQLite at `data/openmgmt.sqlite`
- Rust and Cargo workspace
- Tauri v2 desktop shell
- Leptos Rust/WASM UI
- No npm, pnpm, Corepack, Node.js, React, Vite, Electron, or Docker
- No public network service required

## Acceptance criteria

- `cargo build` and `cargo test` pass from a fresh clone after Rust
  prerequisites are installed.
- Every specified Tauri command is exposed.
- Migrations are idempotent and first launch creates an empty workspace.
- The separate TV board is readable at a distance and refreshes every 10
  seconds.
- MCP delete/archive tools are absent and write tools are disabled by default.
