# OpenMgmt Roadmap

OpenMgmt is a lightweight, local-first operations desk for projects, tasks, timers, ER-board scheduling, and AI-assisted planning. The app should stay fast, private, and expandable without becoming a bloated enterprise project-management clone.

## Current Status

OpenMgmt has moved beyond the initial scaffold and is now in the v0.1 alpha / daily-operations phase.

Implemented or substantially started:

- Rust-first Cargo workspace
- Tauri v2 desktop shell
- Leptos/WASM desktop UI
- SQLite persistence through `rusqlite`
- Organizations, projects, and tasks
- Project categorization by organization and type
- Task scoring and ER-style operations board
- Windowed pop-out board
- Basic timers and time-limit display
- MCP server with guarded writes
- Early sync server/client/protocol work
- Daily Operations groundwork

## Product Principles

- Local-first by default
- Lightweight desktop experience
- User owns their data
- AI integrations are optional and permissioned
- Read-only AI mode by default
- No destructive AI actions without explicit user control
- Sync should preserve local-first ownership
- UI should feel like a serious operations tool, not a generic Kanban clone

## Phase 1: Daily Operations

Goal: make OpenMgmt genuinely useful as a daily command center.

Planned or in progress:

- Saved task views
  - All Tasks
  - Today
  - MVP
  - Launch
  - Bugs
  - Blocked
  - Due Soon
  - In Progress
  - Pinned
- Strong task filtering and sorting
  - urgency
  - priority
  - due date
  - status
  - project
  - organization
  - tag
- Prominent colored task tags
- Timer sessions
  - start
  - pause
  - resume
  - stop
  - complete
  - historical time entries
- Configurable scoring settings
  - priority weight
  - pinned boost
  - overdue boost
  - due-soon window
  - blocked/waiting penalties
  - paused-project penalties
- Backup/export
  - JSON export
  - CSV export
  - SQLite backup
- Recurring tasks
- Task dependencies
- Drag-and-drop scheduling and board movement
- Keyboard/accessibility improvements
- Tauri end-to-end tests

## Phase 2: AI Integration Layer

Goal: allow ChatGPT, Claude, and local models to read OpenMgmt context and safely assist with planning, triage, and task management.

### Shared AI Architecture

- Keep AI integration provider-agnostic
- Build around OpenMgmt commands/tools instead of model-specific hacks
- Support read-only mode by default
- Gate writes behind explicit settings
- Never expose destructive delete/archive tools by default
- Log or surface AI-written changes where possible
- Keep all provider configuration local

### Core AI Tools

Expose tools/capabilities for:

- `list_organizations`
- `list_projects`
- `get_project`
- `query_tasks`
- `get_task`
- `get_board_state`
- `get_today_plan`
- `list_saved_task_views`
- `list_timer_sessions`
- `get_scoring_settings`
- `create_task`
- `update_task`
- `complete_task`
- `start_task_timer`
- `pause_task_timer`
- `resume_task_timer`
- `stop_task_timer`
- `create_project`
- `summarize_project`
- `triage_backlog`
- `plan_today`

### ChatGPT / GPT Integration

Goal: allow ChatGPT to understand and operate OpenMgmt when the user explicitly connects it.

Planned:

- MCP/custom connector compatible architecture where available
- Read OpenMgmt data from ChatGPT
  - projects
  - tasks
  - saved views
  - board state
  - timers
  - scoring settings
- Write actions only when explicitly enabled
  - create task
  - update task
  - complete task
  - create project
- Safe fallback integrations
  - JSON export for ChatGPT upload/reference
  - Markdown summaries
  - local API or connector bridge if needed
- Documentation for ChatGPT setup and plan limitations

### Claude Integration

Goal: make Claude Desktop and Claude Code able to use OpenMgmt through MCP.

Planned:

- Harden existing MCP server
- Add all Daily Operations tools to MCP
- Claude setup docs
- Claude Code workflow docs
- Read-only default mode
- Write-enabled mode through environment flag
- Clear examples:
  - “Plan my day from OpenMgmt.”
  - “Triage my backlog.”
  - “Create launch tasks for this project.”
  - “Summarize blocked work.”
  - “What should I work on next?”

### Local Model Integration

Goal: support private/offline AI assistance through local or self-hosted models.

Planned providers:

- Ollama
- LM Studio
- llama.cpp-compatible servers
- Custom OpenAI-compatible endpoint
- Optional hosted OpenAI-compatible endpoint

Planned features:

- Local model settings page
- Provider selection
  - ChatGPT/OpenAI
  - Claude/Anthropic
  - Local model
  - Custom OpenAI-compatible endpoint
- Store provider configuration locally
- Test connection button
- Model picker
- Prompt/template configuration
- Local-only privacy mode
- Per-feature provider selection if useful

## Phase 3: AI Assistant Features

Goal: turn the integration layer into useful workflows.

Planned assistant actions:

- Plan my day
- What should I work on next?
- Summarize this project
- Break this project into tasks
- Triage my backlog
- Find blocked work
- Generate a launch checklist
- Rewrite task descriptions
- Create tasks from notes
- Review overdue work
- Suggest priority changes
- Summarize completed work
- Prepare a weekly project report
- Identify stale projects/tasks

Potential UI surfaces:

- AI panel in Daily Operations
- AI tab on project pages
- AI action menu on task views
- “Ask OpenMgmt” command palette
- Board assistant for operations triage

## Phase 4: Sync

Goal: finish the sync work without undermining the local-first model.

Planned:

- Sync settings UI
- Device registration UI
- Manual sync action
- Auto-sync option
- Sync status indicators
- Conflict handling strategy
- Pull/push failure recovery
- Server deployment docs
- Local network/self-hosted sync docs
- Security review
- Multi-device testing

## Phase 5: Production Readiness

Goal: prepare OpenMgmt for regular use and eventual public release.

Planned:

- Settings/profile system
- Optional encrypted local profile/data
- Calendar integration
- Desktop notifications/reminders
- Signed Windows/macOS/Linux installers
- Auto-update flow
- Better onboarding
- Data import/export polish
- Accessibility audit
- Tauri end-to-end tests
- Error reporting/log viewer
- Public documentation
- Release checklist

## Later / Optional

These should only be added if they preserve the lightweight local-first model:

- Multi-user collaboration
- Shared team workspaces
- Hosted sync service
- Plugin system
- Calendar two-way sync
- GitHub issue sync
- Obsidian/Markdown folder sync
- Command palette
- Mobile companion app
