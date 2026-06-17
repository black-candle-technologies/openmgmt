# GPT Action Bridge Security

The GPT Action bridge is optional and local-first. It does not send OpenMgmt data anywhere unless you run the bridge and configure a custom GPT Action to call it.

## Authentication

Every `/api/openmgmt/*` endpoint requires:

```http
Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>
```

Generate a long random token with a password manager or a command such as:

```powershell
[Convert]::ToBase64String((1..48 | ForEach-Object { Get-Random -Maximum 256 }))
```

Do not commit tokens, paste them into docs, or share them in screenshots.

## Read-Only Default

Writes are disabled unless the bridge is started with:

```powershell
$env:OPENMGMT_GPT_WRITE_ENABLED = "true"
```

When write mode is disabled, write endpoints return `403`.

## No Destructive Endpoints

The bridge exposes create-only writes for organizations and projects, plus task creation, limited task updates, and task start/complete/block transitions. Every write requires `OPENMGMT_GPT_WRITE_ENABLED=true`.

It does not expose any archive, delete, rename-of-org/project, backup, restore, arbitrary SQL, or filesystem write endpoints. Organizations and projects can be created but never removed or renamed through the bridge.

## Action Logging

The bridge records GPT write attempts in `gpt_action_log`.

The log stores:

- action name
- resource type and optional resource ID
- method and path
- short request summary
- success flag
- error message when applicable

Bearer tokens are never logged. Full request bodies are not logged.

## Exposing the Bridge

ChatGPT cannot call a private `localhost` server directly. To use GPT Actions, the bridge must be reachable over HTTPS from ChatGPT, commonly through a tunnel during development.

Risks:

- Any reachable bridge endpoint can expose local OpenMgmt data to callers with the token.
- A weak or leaked token allows API access.
- A broad tunnel can expose the service outside your machine.

Recommendations:

- Bind locally by default with `OPENMGMT_GPT_BIND=127.0.0.1:8790`. Avoid `0.0.0.0`, which exposes the bridge to your whole network; the bridge logs a warning when bound to a non-loopback address.
- Use HTTPS for anything reachable by ChatGPT.
- Use a long random token.
- Restrict CORS with `OPENMGMT_GPT_CORS_ORIGIN` when browser access is needed.
- Keep write mode off until you are actively testing writes.
- Stop the bridge when not using the custom GPT.
