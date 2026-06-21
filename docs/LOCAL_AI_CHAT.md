# Local AI Command Chat

OpenMgmt local AI chat is a local-first assistant surface backed by Ollama. It stores chat sessions, messages, and proposed tool calls in SQLite.

## What It Does

- Persistent chat sessions with per-session model selection
- Saved user, assistant, system, and tool messages
- Compact OpenMgmt context for planning
- Deterministic slash commands that do not require a model
- Known OpenMgmt tools only: no shell, no filesystem, no raw SQL
- Write actions are proposed first and require confirmation before execution

## Slash Commands

- `/help`
- `/plan`
- `/board`
- `/tasks`
- `/tasks blocked`
- `/tasks overdue`
- `/schedule today`
- `/schedule week`
- `/unscheduled`
- `/models`
- `/use <model>`
- `/create task <title>`
- `/complete task <id>`
- `/start task <id>`

Read commands run immediately. Write commands create a proposed tool call.

## Tool Confirmation Flow

1. User or AI proposes a known write tool.
2. OpenMgmt stores it as `proposed`.
3. User confirms it.
4. OpenMgmt executes only the known operation.
5. The result is saved as a tool message.

Canceled calls do not execute. Destructive tools are not registered in v1.

## Context

The assistant receives compact context, not a database dump. Context can include workspace counts, board state, daily operations, schedule, blocked tasks, overdue tasks, unscheduled tasks, and the rule that P1 is highest priority.

## Privacy

OpenMgmt sends data only to the configured local Ollama base URL. The default is `http://127.0.0.1:11434`.

## Recommended Model

```powershell
ollama pull qwen3:1.7b
```

Small local models can miss details. Use slash commands for deterministic answers and confirmed tools for writes.
