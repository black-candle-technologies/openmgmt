# OpenMGMT Dispatch API

Standalone, local-first app companion service: draft an editable task work order,
list Pi models, dispatch it against a project repository, and restore job state.
The desktop/Android clients retain responsibility for selecting the project's
repository/branch and letting the user edit the prompt before dispatch.

## Run and deploy

```sh
cargo run -p openmgmt-dispatch
cargo test -p openmgmt-dispatch
cargo clippy -p openmgmt-dispatch --all-targets -- -D warnings
cargo fmt --all -- --check
```

Provision `CP_SERVICE_CREDENTIAL` first. An admin mints an authd **service account**
credential (hsurr surrogate) with `controlplane:execute:pi`. Store it in the
root-managed `/etc/bct/dispatch.env` (mode 0600), never in source, client bundles,
or prompts. The service never mints or ships this credential. Rotate/revoke it
through authd. Repository access also depends on that service account's Control
Plane configuration.

Build with `cargo build --release -p openmgmt-dispatch`, install the binary into
`/opt/bct/openmgmt-dispatch/openmgmt-dispatch`, and install
`deploy/bct-dispatch.service`. Make the working directory writable by `ubuntu`
for SQLite; protect DB files/backups since they contain prompts and job output.
Use a TLS reverse proxy when exposing the service beyond loopback. Do not expose
it publicly without deployment-level rate limits and suitable account admission
policy: by default any valid authd subject can dispatch using the service account.
Run one service process per DB (lifecycle mutations are serialized in-process).

## Environment

| Variable | Default / purpose |
| --- | --- |
| `BIND` | `127.0.0.1` |
| `PORT` | `8790` |
| `DISPATCH_DB` | `./dispatch.sqlite` (bundled SQLite, WAL) |
| `AUTHD_BASE` | `https://auth.blackcandletech.com` |
| `REQUIRE_DISPATCH_SCOPE` | `false`; set `true` to require `openmgmt:dispatch` |
| `MINIMAX_API_KEY` | Unset: deterministic template only |
| `MINIMAX_BASE` | `https://api.minimax.io/v1` |
| `MINIMAX_MODEL` | `MiniMax-M3` |
| `MODELS_JSON` | Unset: embedded catalog; otherwise a JSON file path |
| `CP_EXEC_URL` | `https://mcp.internal.blackcandletech.com`; root URL gets `/v1/execute` appended, or supply a full execution endpoint |
| `CP_SERVICE_CREDENTIAL` | Required service credential, described above |
| `POLL_INTERVAL_SECS` | `15`, positive integer; delay between polling sweeps |
| `RUST_LOG` | `openmgmt_dispatch=info` |

Catalog file format is a JSON array of
`{provider, model, label, available, unavailable_reason?}`. Defaults are
`openai-codex / gpt-6-astra` and `opencode-go / gpt-6-luna`, both available.
Unavailable/unknown selections cannot be dispatched. Config is loaded at startup.

## Authentication and ownership

Every endpoint requires `Authorization: Bearer <authd access token>`. Middleware
calls `GET AUTHD_BASE/oauth/userinfo` with that token on every request. Missing,
invalid, expired, unavailable userinfo, or missing subject returns 401. The `sub`
is stored as `owner_sub`; reads, follow-ups, cancellation, latest-by-task, and
resolution are owner-isolated (other owners see 404). If userinfo supplies `scope`
or `scopes` (space-delimited string or array), scopes are logged, never tokens.
Scope enforcement is opt-in until the Scope Registry entry is deployed. authd
must include scopes in userinfo for opt-in scope enforcement to succeed.

The caller's token goes only to configured authd. The CP credential goes only to
the configured HTTPS execution endpoint. HTTP redirects are disabled on all
clients, preventing credential forwarding to redirect destinations. Admins must
configure trusted authd/MiniMax/CP URLs. User input is serialized as JSON into JS
arguments, never interpolated as executable code. Requests are limited to 256 KiB.

## JSON endpoints

- `POST /v1/prompts:generate`: snapshot `{title, description?, project_name?,
  organization?, priority?, due?, tags?, repo, default_branch}` → `{prompt, source}`.
  MiniMax OpenAI-compatible `/chat/completions`, with server-side work-order
  instructions. Missing key, any error, empty response, or 20-second timeout falls
  back to the deterministic snapshot template (`source: "template"` rather than
  `"minimax"`). Only snapshot data is sent to MiniMax, not auth credentials.
- `GET /v1/models`: catalog array.
- `POST /v1/dispatches`: `{task_id, project_id, repo, branch, prompt, provider,
  model}` → `{dispatch_id, pi_job_id, state}`. IDs are strings, repo is `owner/name`.
  A registry row/event is committed before the start call. A failed/unconfirmed
  start returns the row with `state: "failed"` and a null job ID for restoration.
- `GET /v1/dispatches/{id}`: dispatch fields, PR URL, activity, timestamps, events.
- `GET /v1/dispatches?task_id=...`: latest owner-scoped dispatch, or 404.
- `POST /v1/dispatches/{id}/messages`: `{text}`. Fetch current Pi status; live jobs
  receive `pi.resume`. Ended jobs get `pi.start` on the same repository/branch with
  original work order, prior summary (output fallback), and follow-up. Both job
  IDs are retained in `dispatch_jobs` as `initial` / `continuation`.
- `POST /v1/dispatches/{id}/cancel`: `pi.cancel`, then `failed` with activity/event
  reason `Stopped by you`. CP failure leaves the state unchanged for retry.
- `POST /v1/dispatches/{id}/resolve`: owner confirmation that a PR was closed or
  merged; transitions `pr_opened` to `done`. Other states return 409.

## Control Plane protocol and state machine

Matches `bct-exec`: HTTPS POST `/v1/execute`, bearer service credential, JSON
`{"code":"pi.start({...})","modules":["pi"]}`; consume the response's `result`.
The same wrapper calls `pi.status`, `pi.resume`, and `pi.cancel` with `job_id`.
Start includes only `prompt`, `repo`, `branch`, `provider`, `model`, `timeout: 1800`.
Every initial, continuation, or resumed prompt starts with exactly:

```text
[openmgmt-dispatch <dispatch_id> task <task_id> repo <owner/name>]
```

No unsupported title or metadata arguments are sent. `CpClient`, `Auth`, and
`PromptGenerator` are injectable traits; tests use a recording `FakeCpClient`.
No tests require live credentials, MiniMax, or CP jobs.

The watcher polls persisted `dispatched` / `running` rows after startup:

| Pi status | Dispatch state |
| --- | --- |
| queued / starting | dispatched |
| running | running |
| done with matching repository PR URL in output | pr_opened |
| done without PR | done |
| failed / cancelled / interrupted-past-deadline | failed |

It accepts `state` or `status` in Pi responses. Unknown status and transient CP
failures do not falsely complete jobs. Activity is the last nonempty output line,
trimmed to 200 Unicode characters. PR extraction accepts
`https://github.com/<bound-owner>/<bound-repo>/pull/<number>`. Transitions append
ordered timestamped events; repeated polls do not duplicate transition events.

### Deliberate v1 limits

- **No GitHub polling**: `pr_opened` stays visible until the owner calls `/resolve`.
  A Pi `done` response alone is not evidence that a PR was closed or merged.
- No transactional exactly-once guarantee across SQLite and CP. An HTTP timeout
  or crash may leave an unconfirmed start; it is marked failed and never
  automatically retried. Operators must reconcile using the prompt linkage in
  CP before starting again to avoid duplicate jobs.
- Pi status must expose textual `output` for activity/PR detection and `summary`
  or output for continuation context. A status response without those fields
  cannot provide that context; this service does not invent a log API.
- No distributed lifecycle locking, pagination/retention policy, per-repository
  authorization beyond CP service-account permissions, or client UI changes.
  The in-process lifecycle lock favors correctness over high dispatch throughput.
