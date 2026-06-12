# OpenMgmt

OpenMgmt is a lightweight, local-first desktop project and task manager. It
organizes work by organization and project, scores task urgency, tracks active
timers, and presents the current plan as a fullscreen ER-style operations
board.

## Why Rust and Tauri

The application uses one Rust domain layer for the desktop UI, SQLite
persistence, urgency scoring, board grouping, and MCP tools. Tauri v2 provides
a small native desktop shell while Leptos provides a Rust/WASM interface. There
is no Node.js, npm, pnpm, Corepack, React, Vite, Electron, or hosted service.

## Features

- Create, edit, and archive organizations and projects
- Create and edit tasks, then start, complete, block, unblock, or cancel them
- Per-project task priority, scheduling, pinning, tags, estimates, and time limits
- Active task timers based on `started_at`
- Tuneable urgency scoring and seven-column auto-shifting board
- Dedicated fullscreen TV board with manual refresh and 10-second auto-refresh
- Local SQLite database at `data/openmgmt.sqlite`
- Idempotent database seed that repairs partially seeded databases
- Claude and MCP-compatible AI access through a separate rmcp server
- Read-only MCP by default, with explicit opt-in writes

## Prerequisites

Install:

1. [Rust with the MSVC toolchain](https://rustup.rs/)
2. [Tauri v2 Windows prerequisites](https://v2.tauri.app/start/prerequisites/)
3. The WASM target and Cargo-only development tools:

```powershell
rustup target add wasm32-unknown-unknown
cargo install tauri-cli --version "^2.11" --locked
cargo install trunk --version "0.21.14" --locked
```

These install into the current user's Cargo directory and do not require
administrator permissions.

## Run the desktop app

From the repository root:

```powershell
Set-Location apps/desktop/src-tauri
cargo tauri dev
```

The app migrates and seeds the database on startup. Select **Open TV Board** to
open a separate, frameless fullscreen Tauri window. The main app and TV board
use the same repository-local `data/openmgmt.sqlite` file.

## Seed the database

Startup seeding creates the default organizations, the OpenMgmt project, and
tasks in several statuses. It is safe to run repeatedly. To repair or reload
seed data while the app is open, select **Seed database** in the sidebar.

The seed includes active, overdue, blocked, scheduled, inbox, and in-progress
tasks so the TV board has useful data on a new database.

## Supported MVP workflow

1. Create an organization, then edit its name, description, color, or icon.
2. Create a project under an organization, edit its metadata, or archive it.
3. Create and edit tasks from the Today or project views.
4. Start, complete, block, unblock, or cancel a task from its task card.
5. Use **Refresh data** for an explicit reload. Successful mutations also
   refresh the main snapshot immediately.
6. Open the TV board to view NOW, NEXT UP, DUE SOON, WAITING / BLOCKED, LATER
   TODAY, OVERDUE, and DONE TODAY. The board displays an empty state when no
   tasks qualify.

## Build and test

```powershell
cargo build
cargo test
```

Build the production desktop app:

```powershell
Set-Location apps/desktop/src-tauri
cargo tauri build
```

## MCP server

The MCP server uses the same `data/openmgmt.sqlite` database:

```powershell
cargo run -p openmgmt-mcp
```

Read tools are always enabled. To expose non-destructive write tools in the
current PowerShell session:

```powershell
$env:OPENMGMT_MCP_WRITE_ENABLED = "true"
cargo run -p openmgmt-mcp
```

See [AI integration](docs/AI_INTEGRATION.md) for Claude configuration and
ChatGPT-compatible deployment notes.

## Manual verification

After `cargo tauri dev` opens the app:

1. Create and edit an organization; restart the app and confirm the changes
   remain.
2. Create and edit a project in that organization.
3. Create a task, edit all needed fields, then select **Start**. Confirm it
   becomes in progress immediately.
4. Select **Done** and confirm it appears in Done Today.
5. Block another task with a reason, unblock it, and cancel a disposable task.
6. Select **Open TV Board** and confirm task cards and all seven columns render.
7. Select **Seed database** twice and confirm it succeeds without duplicates.

## Known MVP limitations

- Archive and cancel are soft-delete operations; there is no restore UI yet.
- There is no task deletion, drag-and-drop board editing, recurring work, or
  multi-user synchronization.
- Date/time fields use browser-local `datetime-local` inputs and are stored in
  SQLite as UTC timestamps.
- The TV board is display-focused; task editing remains in the main window.

## Workspace

```text
crates/openmgmt-core       shared models, SQLite, scoring, board, commands
crates/openmgmt-mcp        rmcp stdio server
apps/desktop/src-tauri     Tauri v2 native shell and commands
apps/desktop/ui            Leptos Rust/WASM frontend
docs                       product and integration documentation
data                       local SQLite database
```
