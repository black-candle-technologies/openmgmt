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

Every message is **classified** first, and only write requests ever touch the
tool layer:

1. **Classify the turn** (deterministic router): pure chat, a workspace read, a
   write request, or something too vague to act on.
2. **Pure chat** → a plain conversational reply. The model never sees the write
   tool manifest and cannot call tools. "Say the words something" replies
   "something"; "what can you do?" answers in plain language.
3. **Read** → OpenMgmt injects compact workspace context and the model answers
   conversationally. No mutations, no proposals.
4. **Write** → OpenMgmt builds a **complete, ordered plan** up front (every step
   the request needs), validates it, and then applies the access gate:
   - **Read only** — nothing changes; it explains how to switch modes.
   - **Ask first** — one grouped *plan card* with every step; **Confirm plan**
     runs them in order, **Cancel** discards the whole plan.
   - **Full access** — the whole plan executes in order automatically, then the
     assistant reports what it did in one message.

Because the plan is built before anything runs, multi-step requests finish in a
single turn — no need to keep prompting for the next step.

Example — in **Full access**, "create an organization called localtest, and then
a project under that called localproject and then a task under that called
localtask" creates all three (org → project → task, in dependency order) and
replies "Done — …". In **Ask first**, the same message shows one three-step plan
card; **Confirm plan** runs all three in order. In **Read only**, it explains it
needs write access.

### Native tools vs. JSON plan fallback

Borrowing from Zed's Ollama integration, OpenMgmt detects each model's
capabilities (see [OLLAMA.md](OLLAMA.md)). Native Ollama tool calling is used for
tool-capable models on read interactions where it helps; models without tool
support fall back to a strict JSON `action_plan` protocol. **Writes always go
through the plan-first path regardless** — it's the safest product behavior, and
it's validated and access-gated before anything mutates.

## Safety

- Only known OpenMgmt operations are exposed — there is no shell, filesystem, or
  raw-SQL tool. Even Full access is "full access to OpenMgmt operations", not
  arbitrary code execution.
- The router keeps pure chat away from tools entirely: "say the words clear
  schedule" replies "clear schedule" — it never runs `clear_task_schedule`.
- Write tools only run when the turn is classified as a write request, so an
  unrelated message can't trigger a mutation even in Full access.
- Unknown tools, malformed plans, and read tools in a write plan are rejected;
  an invalid plan is repaired once, then turns into a clarifying question rather
  than a random action.
- The assistant never claims to have changed data unless a tool actually ran.
- Destructive operations (archiving, resetting settings) are flagged. There is no
  destructive *delete* tool in v1.

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
