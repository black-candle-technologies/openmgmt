# OpenMgmt Ollama Integration

OpenMgmt can use a local Ollama server for read-only planning workflows. Data is sent only to the configured local Ollama base URL.

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

## Workflows

OpenMgmt can:

- Test the local Ollama connection with `/api/version`
- List installed local models with `/api/tags`
- Run a simple prompt with `/api/chat`
- Plan the day from board and schedule context
- Suggest the next task
- Triage overdue, blocked, due-soon, and unscheduled tasks
- Summarize a project
- Suggest a rewritten task description without saving it

## Privacy

This integration is local-only. OpenMgmt rejects cloud-looking base URLs and does not send API tokens to Ollama.

## Troubleshooting

- Ollama not running: start Ollama, then test the connection again.
- No models installed: run `ollama pull qwen3:1.7b`.
- Model too slow: use a smaller model such as `qwen3:1.7b`.
- GPU or VRAM pressure: close other GPU-heavy apps or use a smaller quantized model.
- Wrong URL: use `http://127.0.0.1:11434` unless you explicitly enabled local-network access.
