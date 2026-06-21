# OpenMgmt Ollama Integration

OpenMgmt uses a local Ollama server to power the Local AI chat — an agent-style
assistant that reads your workspace and, depending on access mode, manages
OpenMgmt through typed tools. Data is sent only to the configured local Ollama
base URL.

## Setup

1. Install Ollama from https://ollama.com.
2. Start Ollama.
3. Pull the default lightweight model:

```powershell
ollama pull qwen3:1.7b
```

Optional small models:

```powershell
ollama pull llama3.2:3b
ollama pull qwen2.5-coder:3b
```

Default OpenMgmt settings:

- Provider: Ollama
- Base URL: `http://127.0.0.1:11434`
- Default model: `qwen3:1.7b`
- Keep alive: `5m`
- Chat mode: non-streaming `/api/chat` with `"stream": false`

## What It Powers

OpenMgmt uses Ollama to:

- Test the local connection with `/api/version` and list installed models with `/api/tags`.
- Run the Local AI **agent loop** over non-streaming `/api/chat`: natural-language
  in, structured tool calls out, validated and gated by the chat's access mode.
- Read your workspace and manage tasks/projects/schedules through typed tools.
- Persist chat sessions and messages, with per-session model selection.

Access modes (Read only / Ask first / Full access) are documented in
`docs/LOCAL_AI_CHAT.md`. The earlier planning workflows (plan day, suggest next
task, triage, summarize project) still exist behind the scenes, but the chat is
now an agent rather than a command runner.

### Structured tool protocol

Local models rarely support native function calling, so the model is asked to
return one JSON object per turn — either `{"type":"final","message":"…"}` or
`{"type":"tool_calls","tool_calls":[…]}`. OpenMgmt parses this defensively
(fenced, loose, or surrounded by prose), retries once with a stricter nudge on
malformed output, and falls back to plain text without ever claiming a write
happened.

## Privacy

This integration is local-only. OpenMgmt rejects cloud-looking base URLs and does not send API tokens to Ollama.

## Troubleshooting

- Ollama not running: start Ollama, then test the connection again.
- No models installed: run `ollama pull qwen3:1.7b`.
- Model too slow: use a smaller model such as `qwen3:1.7b`.
- GPU or VRAM pressure: close other GPU-heavy apps or use a smaller quantized model.
- Wrong URL: use `http://127.0.0.1:11434` unless you explicitly enabled local-network access.
- Model "says" it did something but nothing changed: that means it didn't emit a
  valid tool call (common with very small models). Rephrase, or pick a slightly
  larger model. The tool runtime intentionally never fakes a mutation from prose.
