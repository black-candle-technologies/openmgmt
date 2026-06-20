# OpenMgmt Roadmap

## MVP

- Rust domain and SQLite persistence
- Organization, project, and task commands
- Shared scoring and auto-shifting ER board
- Leptos desktop views, schedule UI, and board workflows
- Active timers and time-limit display
- rmcp stdio server with guarded writes
- Local-first startup with an empty user workspace

## Current Status

- Desktop app, scheduling core, schedule UI support, board scheduling behavior, saved views, scoring settings, timers, sync support, and MCP foundations are current beta work.
- Scheduling is merged into the core: task schedule fields, `calendar_blocks`, conflict detection, time block suggestions, auto-start for due scheduled tasks, scheduled block completion/skip, and ICS export.
- This branch keeps GPT integration active: authenticated HTTP API, read-only default, write-gated task actions, action logging, OpenAPI schema, and custom GPT setup docs.

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

## Upcoming Beta Work

- Finish GPT integration and beta API QA.
- Add Claude MCP integration hardening.
- Add local model integration for Ollama, LM Studio, llama.cpp-compatible servers, and OpenAI-compatible endpoints.
- Harden sync conflict handling and recovery.
- Add SQLite backup and restore.
- Ship signed installers.
- Add automatic update support.
- Complete beta QA, Tauri end-to-end checks, and accessibility pass.
