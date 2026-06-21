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
- Base URL: `http://127.0.0.1:11434` (normalized; trailing slash trimmed)
- Default model: `qwen3:1.7b`
- Keep alive: `5m`
- Temperature / context window: blank (use the model's defaults)
- Chat mode: non-streaming `/api/chat` with `"stream": false`

OpenMgmt **does not auto-load a model** when Local AI opens — selecting a model
can pull it into memory, so you choose one explicitly.

## What It Powers

OpenMgmt uses Ollama to:

- Test the local connection with `/api/version`.
- Discover installed models with `/api/tags` and detect each model's
  capabilities with `/api/show` (see below).
- Run the Local AI assistant over non-streaming `/api/chat`: classify the turn,
  answer chat/read turns conversationally, and apply validated, access-gated
  write plans.
- Persist chat sessions, messages, and tool calls, with per-session model
  selection.

Access modes (Read only / Ask first / Full access) and the turn router are
documented in [LOCAL_AI_CHAT.md](LOCAL_AI_CHAT.md). The earlier planning
workflows (plan day, suggest next task, triage, summarize project) still exist.

## Model capability detection (Zed-style)

On a model refresh OpenMgmt calls `/api/tags`, then `/api/show` per model, and
records a capability profile:

- **context length** — from `model_info`'s `<architecture>.context_length`, or a
  `num_ctx` line in the model's `parameters` (fallback: 4096).
- **`supports_tools` / `supports_vision` / `supports_thinking`** — from the
  `/api/show` `capabilities` array.
- **parameter size / quantization / family** — from model details.
- **embedding models** — flagged (by `capabilities`, or `embed`/`bge`/`bert`
  naming) and demoted/hidden from the chat model picker.

`/api/show` calls run sequentially to stay gentle on the server. The model
dropdown shows a compact capability suffix (`tools`, `thinking`, `vision`, ctx)
and a `tools` / `json fallback` badge for the active model.

### Native tools vs. JSON plan protocol

- Tool-capable models can receive a native Ollama `tools` array on read turns.
- Models without tool support (and all **write** turns) use a strict JSON
  `action_plan` protocol: the model returns one
  `{"type":"action_plan","summary":"…","steps":[…]}` object, which OpenMgmt
  parses defensively (fenced, loose, or in prose), validates, and repairs once.
- A thinking model's `<think>…</think>` reasoning is stripped from the visible
  answer, and `num_ctx`/`keep_alive`/`temperature` are sent per request.
- Token usage (`prompt_eval_count`, `eval_count`) and `done_reason` are parsed
  when present; an empty assistant message with tool calls is handled cleanly.

## Privacy

This integration is local-only. OpenMgmt rejects cloud-looking base URLs and does not send API tokens to Ollama.

## Troubleshooting

- Ollama not running: start Ollama, then test the connection again.
- No models installed: run `ollama pull qwen3:1.7b`.
- Model too slow: use a smaller model such as `qwen3:1.7b`.
- GPU or VRAM pressure: close other GPU-heavy apps or use a smaller quantized model.
- Wrong URL: use `http://127.0.0.1:11434` unless you explicitly enabled local-network access.
- Model "says" it did something but nothing changed: write turns only mutate via
  a validated plan, so prose is never treated as a mutation. Rephrase the
  request, or pick a slightly larger model. `qwen3:1.7b` works via the JSON plan
  fallback even though its native tool support is limited.
- Embedding model missing from the picker: that's intentional — embedding models
  can't chat and are demoted.
