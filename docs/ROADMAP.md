# OpenMgmt Roadmap

## MVP

- Rust domain and SQLite persistence
- Organization, project, and task commands
- Shared scoring and auto-shifting ER board
- Leptos desktop views and fullscreen TV window
- Active timers and time-limit display
- rmcp stdio server with guarded writes
- Local AI agent chat over Ollama: a conversational assistant with a deterministic
  turn router (chat / read / write / clarify), plan-first multi-step writes, and
  per-chat access modes (read only / ask first / full access)
- Zed-style Ollama model capability detection (`/api/show`): context length,
  tools/vision/thinking, embedding-model filtering, no auto-load of a default model

## Next

- Native Ollama tool-call loop for read turns on tool-capable models (the JSON
  plan path remains the default for writes)
- Streaming `/api/chat` responses in the chat panel
- Optional API key / custom headers for remote, self-hosted Ollama-compatible
  endpoints (local-only today)
- Full edit/archive forms in the desktop UI
- Drag-and-drop scheduling and board movement
- Pause/resume timer sessions with historical time entries
- Configurable workday boundaries and scoring weights
- Recurring tasks, dependencies, and saved filters
- SQLite backup, restore, and JSON/CSV export
- Tauri end-to-end tests and accessibility audit

## Scheduling and Calendar Integration

- Internal task scheduling ranges and local calendar blocks
- Calendar and agenda desktop UI
- Recurring tasks and recurrence exceptions
- Local reminder delivery
- ICS export and import
- Google Calendar integration after local scheduling stabilizes
- Outlook and Apple Calendar integration later

## Later

- Optional encrypted profiles
- Calendar and notification integrations
- Sync experiments that preserve local-first ownership
- Signed installers and automatic updates
- Multi-user collaboration only if it can preserve the lightweight model
