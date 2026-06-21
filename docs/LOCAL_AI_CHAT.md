# Local AI Chat

OpenMgmt's Local AI chat is an agent-style assistant backed by a local Ollama
model. You talk to it in plain language — like a chatbot in a terminal, embedded
in OpenMgmt — and it can read your workspace and, depending on the chat's access
mode, manage your tasks, projects, and schedules through safe, typed tools. Chat
sessions, messages, and tool calls are stored locally in SQLite.

## What It Does

- Natural-language conversation — no commands to memorize.
- Reads your workspace (organizations, projects, tasks, board, schedule, daily ops).
- Manages OpenMgmt through typed, validated tools — never shell, filesystem, or raw SQL.
- Resolves names to records for you ("the project Website", "any organization") — you don't supply UUIDs.
- Per-chat **access mode** controls how much it may change.
- Per-session model selection.

## Access Modes

Access mode is set inside each chat (the dropdown in the chat header). The
default is **Ask first**.

| Mode | Read tools | Write tools |
| --- | --- | --- |
| **Read only** | run automatically | blocked — it explains what it *would* do and asks you to switch modes |
| **Ask first** (default) | run automatically | proposed as confirmation cards; nothing changes until you confirm |
| **Full access** | run automatically | run automatically, no confirmation |

Switching a chat **into Full access** asks for a one-time confirmation
("Allow Local AI to create and update OpenMgmt data without confirmation in this
chat?"). It does not ask again per message. Full access is visually flagged with
a warning badge.

## How a Turn Works

1. You send a natural-language message.
2. OpenMgmt builds compact workspace context and a tool manifest and sends them,
   with the access mode, to Ollama.
3. The model replies with either a final answer or structured tool calls.
4. OpenMgmt validates and resolves the tool calls, then applies the access gate:
   execute (reads, or writes in full access), propose (writes in ask-first), or
   block (writes in read-only).
5. Tool results are appended to the transcript.
6. If tools ran, OpenMgmt feeds the results back so the model can summarize what
   happened. The loop is capped at a few steps so it can never run away.

Example — in Full access, "Create a project called localtest under any
organization and make a task called do things" creates the project, then the
task inside it, and replies with what was created. In Ask first, the same
message shows proposed action cards; clicking Confirm runs them in order. In
Read only, it explains it needs write access.

## Safety

- Only known OpenMgmt operations are exposed — there is no shell, filesystem, or
  raw-SQL tool. Even Full access is "full access to OpenMgmt operations", not
  arbitrary code execution.
- Unknown tools, malformed tool calls, and calls with missing required arguments
  are rejected.
- The assistant never claims to have changed data unless a tool actually ran.
- Destructive operations (archiving, resetting settings) are flagged.

## Privacy

OpenMgmt sends context only to the configured local Ollama base URL. The default
is `http://127.0.0.1:11434` — your own machine.

## Recommended Model

```powershell
ollama pull qwen3:1.7b
```

Small local models sometimes need clearer instructions and may miss details, but
the tool runtime prevents fake mutations: if a model only *says* it did
something without emitting a valid tool call, nothing is changed.

## Slash Commands (hidden)

Slash commands (`/plan`, `/board`, `/tasks`, `/models`, `/use <model>`, …) still
work as a hidden power-user/debug path, but they are no longer part of the normal
UX. You never need them — just type what you want.
