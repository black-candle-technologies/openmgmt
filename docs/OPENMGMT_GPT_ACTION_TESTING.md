# OpenMgmt GPT Action Testing Guide

This guide walks through testing the OpenMgmt GPT Action bridge using either **ngrok** or **Cloudflare Tunnel**.

The goal is to let a custom GPT in ChatGPT call your local OpenMgmt bridge through a temporary public HTTPS URL.

## Current testing architecture

```text
ChatGPT Custom GPT
        |
        | HTTPS + Bearer token
        v
ngrok or Cloudflare Tunnel public URL
        |
        v
http://127.0.0.1:8790
        |
        v
openmgmt-gpt-bridge
        |
        v
data/openmgmt.sqlite
```

## Security notes

Do not commit real tokens or temporary tunnel URLs.

Use placeholders in committed docs:

```text
<OPENMGMT_GPT_API_TOKEN>
<YOUR_NGROK_URL>
<YOUR_CLOUDFLARE_TUNNEL_URL>
```

For personal testing, use a long random API token. Keep write mode disabled until read tests work.

```powershell
$env:OPENMGMT_GPT_WRITE_ENABLED = "false"
```

Only enable writes for a deliberate write test:

```powershell
$env:OPENMGMT_GPT_WRITE_ENABLED = "true"
```

After testing, stop the bridge and tunnel, rotate any exposed tokens, and return write mode to disabled.

## Prerequisites

From the OpenMgmt repo root:

```powershell
cd C:\Users\laneb\openmgmt
cargo test
cargo build
```

Make sure the GPT bridge runs:

```powershell
$env:OPENMGMT_GPT_API_TOKEN = "<OPENMGMT_GPT_API_TOKEN>"
$env:OPENMGMT_GPT_WRITE_ENABLED = "false"
cargo run -p openmgmt-gpt-bridge
```

Expected output:

```text
OpenMgmt GPT Action bridge listening on http://127.0.0.1:8790
```

Leave this terminal running.

## Option A: Test with ngrok

### 1. Configure ngrok

After installing ngrok and signing in, connect your local ngrok agent to your account:

```powershell
ngrok config add-authtoken "<YOUR_NGROK_AUTHTOKEN>"
```

Check the install:

```powershell
ngrok version
```

### 2. Start the tunnel

Open a second PowerShell window:

```powershell
ngrok http 8790
```

Copy the HTTPS forwarding URL, for example:

```text
https://example-subdomain.ngrok-free.dev
```

Leave the ngrok terminal running.

### 3. Test the tunnel

Open a third PowerShell window:

```powershell
curl.exe https://example-subdomain.ngrok-free.dev/health
```

Then test a protected endpoint:

```powershell
curl.exe -H "Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>" https://example-subdomain.ngrok-free.dev/api/openmgmt/summary
```

If you get JSON, ngrok is working.

## Option B: Test with Cloudflare Quick Tunnel

Cloudflare Quick Tunnels are good for temporary development testing. They generate a random public `trycloudflare.com` URL that proxies to localhost.

### 1. Install `cloudflared`

Install `cloudflared` using the current instructions from Cloudflare.

Check the install:

```powershell
cloudflared --version
```

### 2. Start the tunnel

Open a second PowerShell window:

```powershell
cloudflared tunnel --url http://localhost:8790
```

Copy the generated HTTPS URL, usually something like:

```text
https://example-random-subdomain.trycloudflare.com
```

Leave the `cloudflared` terminal running.

### 3. Test the tunnel

Open a third PowerShell window:

```powershell
curl.exe https://example-random-subdomain.trycloudflare.com/health
```

Then test a protected endpoint:

```powershell
curl.exe -H "Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>" https://example-random-subdomain.trycloudflare.com/api/openmgmt/summary
```

If you get JSON, Cloudflare Tunnel is working.

## Update the OpenAPI server URL

Open:

```text
docs/gpt-action/openapi.yaml
```

Find:

```yaml
servers:
  - url: https://YOUR-BRIDGE-HOST
```

Replace it with your active tunnel URL.

For ngrok:

```yaml
servers:
  - url: https://example-subdomain.ngrok-free.dev
```

For Cloudflare:

```yaml
servers:
  - url: https://example-random-subdomain.trycloudflare.com
```

Do not commit a temporary personal tunnel URL unless you intentionally want it in the repo.

## Create or update the custom GPT

In ChatGPT web:

1. Open **Explore GPTs**.
2. Click **Create**.
3. Go to **Configure**.
4. Name it:

```text
OpenMgmt Assistant
```

5. Copy the contents of:

```text
docs/gpt-action/GPT_INSTRUCTIONS.md
```

into the GPT **Instructions** field.

6. Under **Actions**, click **Create new action**.
7. Paste/import the contents of:

```text
docs/gpt-action/openapi.yaml
```

8. Set authentication to API key / Bearer token.
9. Paste the OpenMgmt bridge token as the API key:

```text
<OPENMGMT_GPT_API_TOKEN>
```

Do not include the word `Bearer` inside the key field if the GPT editor already has a Bearer option selected.

## Read-only test pass

Keep the bridge running with writes disabled:

```powershell
$env:OPENMGMT_GPT_WRITE_ENABLED = "false"
```

In the GPT preview, test:

```text
Show me my OpenMgmt summary.
```

Expected: it calls `getOpenMgmtSummary`.

```text
List my OpenMgmt organizations.
```

Expected: it calls `listOrganizations`.

```text
What is on my OpenMgmt board?
```

Expected: it calls `getBoardState`.

```text
Plan my day from OpenMgmt.
```

Expected: it calls `getTodayPlan`.

If these work, the read side is connected.

## Write-disabled safety test

Still with writes disabled, ask:

```text
Create an organization called GPT Test.
```

Expected: it should fail politely because write mode is disabled.

This confirms `OPENMGMT_GPT_WRITE_ENABLED=false` is protecting write endpoints.

## Enable writes for the write test

Stop the bridge terminal with `Ctrl+C`.

Restart it:

```powershell
cd C:\Users\laneb\openmgmt
$env:OPENMGMT_GPT_API_TOKEN = "<OPENMGMT_GPT_API_TOKEN>"
$env:OPENMGMT_GPT_WRITE_ENABLED = "true"
cargo run -p openmgmt-gpt-bridge
```

Leave your ngrok or Cloudflare tunnel running.

## Full write test flow

In the GPT preview, run these in order:

```text
Create an organization called GPT Test.
```

Expected action: `createOrganization`.

```text
Create a project called GPT Bridge Test inside GPT Test.
```

Expected action: `createProject`.

```text
Create a P3 task in GPT Bridge Test called Test GPT write path.
```

Expected action: `createTask`.

```text
Show me the task you just created.
```

Expected action: `listTasks` or `getTask`.

```text
Start the task called Test GPT write path.
```

Expected action: `startTask`.

```text
Block that task with the reason "Testing GPT block action."
```

Expected action: `blockTask`.

```text
Complete that task.
```

Expected action: `completeTask`.

## Verify in OpenMgmt desktop

Start the desktop app:

```powershell
cd C:\Users\laneb\openmgmt\apps\desktop\src-tauri
cargo tauri dev
```

Confirm you can see:

```text
Organization: GPT Test
Project: GPT Bridge Test
Task: Test GPT write path
Status: Done
```

This confirms the GPT bridge and the desktop app are using the same SQLite database.

## Verify with curl

You can also verify via the API:

```powershell
curl.exe -H "Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>" https://example-subdomain.ngrok-free.dev/api/openmgmt/organizations
```

```powershell
curl.exe -H "Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>" https://example-subdomain.ngrok-free.dev/api/openmgmt/projects
```

```powershell
curl.exe -H "Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>" https://example-subdomain.ngrok-free.dev/api/openmgmt/tasks
```

Replace the URL with your ngrok or Cloudflare tunnel URL.

## Common failure cases

### GPT says authentication failed

Check the token in three places:

1. Bridge terminal env var:

```powershell
$env:OPENMGMT_GPT_API_TOKEN
```

2. GPT Action authentication key.
3. Curl test:

```powershell
curl.exe -H "Authorization: Bearer <OPENMGMT_GPT_API_TOKEN>" <TUNNEL_URL>/api/openmgmt/summary
```

Make sure the token is identical. A single missing character will cause `401`.

### Read endpoints work but writes fail

Check whether the bridge is running with:

```powershell
$env:OPENMGMT_GPT_WRITE_ENABLED = "true"
```

If it is false, writes should return `403`.

### GPT cannot reach the server

Check:

```powershell
curl.exe <TUNNEL_URL>/health
```

If that fails:

- the bridge may not be running
- ngrok/cloudflared may not be running
- the tunnel URL may have changed
- the OpenAPI `servers.url` may still point to the old URL

### GPT Action importer skips functions

The GPT Action importer may reject some otherwise valid OpenAPI patterns. For OpenMgmt, operation-level parameters should be inlined instead of using `$ref` parameter objects.

Search the schema:

```powershell
Select-String -Path docs/gpt-action/openapi.yaml -Pattern "\$ref"
```

Schema `$ref`s under `components.schemas` are fine. Avoid `$ref` inside operation-level `parameters`.

### Ngrok shows requests but OpenMgmt returns 401

The tunnel is working, but the Bearer token is wrong.

### Ngrok/Cloudflare URL changed

Temporary tunnel URLs can change when you restart the tunnel. Update `servers.url` in `docs/gpt-action/openapi.yaml` and re-import/paste the schema in the GPT Action editor.

## After testing

Turn writes off:

```powershell
cd C:\Users\laneb\openmgmt
$env:OPENMGMT_GPT_API_TOKEN = "<OPENMGMT_GPT_API_TOKEN>"
$env:OPENMGMT_GPT_WRITE_ENABLED = "false"
cargo run -p openmgmt-gpt-bridge
```

Stop the tunnel when done:

```text
Ctrl+C
```

If any real token or tunnel secret was pasted into chat, logs, screenshots, or docs, rotate it before longer-term use.

## Recommended commit checklist

Before pushing:

```powershell
git status
cargo fmt --check
cargo test
cargo build
```

Make sure these are not committed:

- real GPT API token
- ngrok authtoken
- temporary ngrok URL, unless intentionally documented as an example
- temporary Cloudflare tunnel URL, unless intentionally documented as an example

Suggested file path if adding this guide to the repo:

```text
docs/gpt-action/TESTING.md
```

## References

- OpenAI GPT Actions documentation: https://platform.openai.com/docs/actions
- OpenAI Help: Creating and editing GPTs: https://help.openai.com/en/articles/8554397-creating-a-gpt
- ngrok getting started: https://ngrok.com/docs/getting-started/
- Cloudflare Quick Tunnels: https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/
