# OpenMgmt Roadmap

## MVP

- Rust domain and SQLite persistence
- Organization, project, and task commands
- Shared scoring and auto-shifting ER board
- Leptos desktop views and fullscreen TV window
- Active timers and time-limit display
- rmcp stdio server with guarded writes
- Local-first startup with an empty user workspace

## Current Status

- Desktop app, sync support, Daily Operations, saved views, scoring settings, timers, TV board, and MCP foundations are in active integration.
- This branch implements the first ChatGPT / GPT Action bridge: authenticated HTTP API, read-only default, write-gated task actions, action logging, OpenAPI schema, and custom GPT setup docs.

## Phase 2A: ChatGPT / GPT App Integration

- Ship `openmgmt-gpt-bridge` as a separate optional binary.
- Keep all `/api/openmgmt/*` endpoints bearer-token authenticated.
- Keep GPT writes disabled by default through `OPENMGMT_GPT_WRITE_ENABLED=false`.
- Expose read endpoints for summary, organizations, projects, tasks, board, and today planning.
- Expose non-destructive task write endpoints for create, update, complete, start, and block.
- Maintain a durable GPT action log for write attempts.
- Publish `docs/gpt-action/openapi.yaml` for custom GPT Actions.

## Phase 2B: Claude / MCP Hardening

- Continue hardening MCP read tools and write gating.
- Keep destructive tools absent by default.
- Align MCP tool permissions with the AI safety model.

## Phase 2C: Local Model Support

- Expand provider configuration for Ollama, LM Studio, llama.cpp-compatible servers, and custom OpenAI-compatible endpoints.
- Keep local-only mode explicit and testable.
- Add deterministic assistant workflows before enabling model calls by default.

## Phase 3: Sync Hardening

- Improve sync conflict reporting and recovery.
- Add more integration tests around remote event ordering and replay safety.
- Preserve local-first ownership and offline behavior.

## Next Desktop/Core Work

- Full edit/archive forms in the desktop UI
- Drag-and-drop scheduling and board movement
- Pause/resume timer sessions with historical time entries
- Configurable workday boundaries and scoring weights
- Recurring tasks, dependencies, and saved filters
- SQLite backup, restore, and JSON/CSV export
- Tauri end-to-end tests and accessibility audit

## Later / Production Readiness

- Optional encrypted profiles
- Calendar and notification integrations
- Signed installers and automatic updates
- Multi-user collaboration only if it can preserve the lightweight model
