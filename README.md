# OpenMgmt

OpenMgmt is a lightweight, local-first desktop project and task manager. It
organizes work by organization and project, scores task urgency, tracks active
timers, and presents the current plan as a live operations board.

The interface is built from a small, reusable Leptos component library
(app shell, sidebar, top bar, panels, badges, record tables/cards, a side
drawer for create/edit, and a shared board) so new views are quick to add.
Records are created and edited in a focused side drawer instead of always-open
forms, keeping the workspace calm and scannable.

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
- Dedicated TV board that opens in a normal, movable, resizable window with
  manual refresh and 10-second auto-refresh (kiosk/fullscreen reserved for a
  later option)
- Local SQLite database at `data/openmgmt.sqlite`
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

On first launch, the app creates and migrates an empty local SQLite database at
`data/openmgmt.sqlite`. It does not create sample organizations, projects, or
tasks; users create their own workspace records. Select **Open TV Board** in the
top bar to open the board in a separate, normal Tauri window (decorated,
movable, and resizable; centered at 1440x900). The board window renders a dark
operations layout and never shows a blank white screen. In development it loads
the Trunk dev server with the board query string (`http://127.0.0.1:1420/?board=1`);
in a packaged build it loads the bundled asset (`tauri://localhost/index.html?board=1`).
The main app and TV board use the same repository-local `data/openmgmt.sqlite` file.

The left sidebar navigates Dashboard, Organizations, Projects, Tasks, Today, and
an embedded Board. The top bar exposes the current page title, a status
indicator, and the Refresh and Open TV Board actions.

## Supported MVP workflow

1. Create an organization from the **New organization** drawer, then reopen the
   drawer to edit its name, description, color, or icon, or to archive it.
2. Create a project from the **New project** drawer, edit its metadata, or
   archive it. Open a project to see its workspace page and task table.
3. Create and edit tasks from the **New task** drawer (reachable from the top
   bar, Tasks, Today, and project pages). Click any task title to edit it.
4. Start, complete, block, unblock, or cancel a task from its row/card actions.
5. Use **Refresh** for an explicit reload. Successful mutations also refresh the
   main snapshot immediately.
6. Open the TV board (or the in-app **Board** section) to view NOW, NEXT UP, DUE
   SOON, WAITING / BLOCKED, LATER TODAY, OVERDUE, and DONE TODAY. The board
   displays an empty state when no tasks qualify.

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
6. Select **Open TV Board** and confirm it opens in a normal, movable,
   resizable window (not fullscreen) with all seven columns and readable cards,
   and no blank white screen.

## Troubleshooting (Windows)

If `cargo tauri dev` fails to start with:

```text
Only one usage of each socket address (protocol/network address/port) is
normally permitted. (os error 10048)
The "beforeDevCommand" terminated with a non-zero status code.
```

port `1420` is already in use — almost always by a stale `trunk` (or a previous
`openmgmt`/`cargo tauri dev`) process that survived a crash or a hard stop,
because the dev server is started with `wait: false` and is not always cleaned
up on exit. Find and stop whatever is holding the port, then re-run:

```powershell
# Show the process (PID) listening on 1420
Get-NetTCPConnection -LocalPort 1420 -State Listen |
  Select-Object LocalAddress, LocalPort, OwningProcess
# (fallback) netstat -ano | Select-String ":1420"

# Stop that specific process by PID
Stop-Process -Id <PID> -Force

# Or stop any stale dev processes by name
Get-Process trunk, openmgmt, openmgmt-ui -ErrorAction SilentlyContinue |
  Stop-Process -Force
```

The dev server now binds a single loopback interface (`127.0.0.1:1420`, set in
`apps/desktop/ui/Trunk.toml`) and `devUrl` matches it, which avoids the
dual-stack IPv4/IPv6 self-conflict that can also produce `os error 10048` on
Windows even when no other process holds the port.

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
apps/desktop/ui/src/app    UI modules: state, components, records, forms,
                           board, and pages/ (one module per workspace section)
docs                       product and integration documentation
data                       local SQLite database
```
