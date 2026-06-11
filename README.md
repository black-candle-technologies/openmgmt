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

- Organizations and categorized projects
- Per-project tasks with priority, scheduling, blocking, and time limits
- Active task timers based on `started_at`
- Tuneable urgency scoring and seven-column auto-shifting board
- Dedicated fullscreen TV board window refreshed every 10 seconds
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

The app migrates and seeds the database on startup. Select **Open TV Board** to
open a separate, frameless fullscreen Tauri window.

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

## Workspace

```text
crates/openmgmt-core       shared models, SQLite, scoring, board, commands
crates/openmgmt-mcp        rmcp stdio server
apps/desktop/src-tauri     Tauri v2 native shell and commands
apps/desktop/ui            Leptos Rust/WASM frontend
docs                       product and integration documentation
data                       local SQLite database
```
