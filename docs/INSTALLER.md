# Windows installer

OpenMgmt ships as a Tauri desktop app. A packaged install does not require Git,
Rust, Cargo, Trunk, PowerShell, or the repository folder at runtime.

## Developer prerequisites

Install these only on the machine that builds the installer:

1. Rust with the MSVC toolchain
2. Tauri v2 Windows prerequisites, including the WebView2 build requirements
3. The WASM target and Cargo-installed build tools:

```powershell
rustup target add wasm32-unknown-unknown
cargo install tauri-cli --version "^2.11" --locked
cargo install trunk --version "0.21.14" --locked
```

OpenMgmt requires Windows 10 or 11 and the Microsoft WebView2 runtime. Windows 11
ships it by default; on some Windows 10 machines it must be installed (the Tauri
NSIS installer downloads it on first run if missing). Unsigned installers may show
Microsoft SmartScreen warnings until OpenMgmt has code signing.

## Build

From the repository root:

```powershell
Set-Location apps\desktop\src-tauri
cargo tauri build
```

The packaged app uses the bundled frontend from `apps\desktop\ui\dist`; it does
not use `http://127.0.0.1:1420` or require a dev server.

Expected artifacts from this workspace (paths relative to the repository root):

- NSIS installer: `target\release\bundle\nsis\OpenMgmt_<version>_x64-setup.exe` (~3.6 MB)
- Desktop executable: `target\release\openmgmt-desktop.exe` (~11 MB)

The installer filename tracks the `version` in `tauri.conf.json`. MSI packaging
is not enabled by default (only the `nsis` target is configured).

## Install and run

Run the NSIS installer, then launch OpenMgmt from the Start Menu. The app opens
as a normal desktop application and keeps user data across restarts.

The installer is configured for per-user installation, so administrator rights
should not be required for a normal install.

### Manual smoke test (run before each beta)

Not automatable in CI; run by hand on a clean Windows 10/11 machine:

1. Run `OpenMgmt_<version>_x64-setup.exe`; confirm install completes without admin.
2. Launch from the Start Menu; confirm a normal window opens with no console window
   and no blank white screen.
3. Create org `Installer Test Org`, project `Installer Test Project`, task
   `Installer Test Task`.
4. Close and reopen the app; confirm the data persists.
5. Confirm the DB exists at `%LOCALAPPDATA%\OpenMgmt\openmgmt.sqlite` or
   `%APPDATA%\OpenMgmt\openmgmt.sqlite`, depending on the machine.
6. Open the TV Board; confirm it renders the board (not a white screen).
7. Uninstall via Settings → Apps; confirm the app data folder remains.

## Data locations

Development runs use the repository-local database:

```text
data\openmgmt.sqlite
```

Installed release builds use a per-user writable database:

```text
%LOCALAPPDATA%\OpenMgmt\openmgmt.sqlite
```

If `%LOCALAPPDATA%` is unavailable, OpenMgmt falls back to `%APPDATA%` as the
base directory. The app creates the directory and runs migrations on first
launch. It does not create sample organizations, projects, or tasks.

`OPENMGMT_DATABASE_PATH` can still override the database path for explicit test
or advanced local workflows.

OpenMgmt does not automatically copy a development database from
`data\openmgmt.sqlite` into the installed app data directory. This avoids
silently overwriting installed user data.

## Uninstall

Use Windows Settings (Apps) or the generated uninstaller to remove the installed
application. The uninstaller removes the program files but leaves user data in
`%LOCALAPPDATA%\OpenMgmt\` or `%APPDATA%\OpenMgmt\` intact, so reinstalling
preserves existing data. To wipe data, delete that folder manually after
uninstalling.

## Troubleshooting

- **`trunk` not found during build** — run `cargo install trunk --version "0.21.14" --locked`.
- **`cargo tauri` not found** — run `cargo install tauri-cli --version "^2.11" --locked`.
- **wasm build fails / missing target** — run `rustup target add wasm32-unknown-unknown`.
- **Tauri build schema error** — a `tauri.conf.json` key is invalid for Tauri v2;
  `cargo tauri build` reports the offending key. Do not guess; fix the named key.
- **App opens to a blank white window** — usually the bundled frontend is stale or
  missing. Rebuild with `trunk build --release` in `apps\desktop\ui`, then rerun
  `cargo tauri build`. For the TV Board specifically, see the board-URL notes in
  `apps/desktop/src-tauri/src/commands.rs`.
- **Stuck on "Loading OpenMgmt"** - the packaged frontend failed before the
  Leptos app mounted. Common causes are WebAssembly blocked by CSP, missing
  bundled JS/WASM/CSS assets, a frontend panic during mount, or a stale
  installed app. Uninstall the old build, rebuild with `cargo tauri build`,
  reinstall the new NSIS installer, and launch again. Confirm
  `%LOCALAPPDATA%\OpenMgmt\openmgmt.sqlite` or
  `%APPDATA%\OpenMgmt\openmgmt.sqlite` is created, try running
  `%LOCALAPPDATA%\OpenMgmt\openmgmt-desktop.exe` directly, and inspect the
  WebView console if the boot screen shows its failure diagnostics.
- **Installer blocked by SmartScreen** — expected for the unsigned beta. Click
  "More info" → "Run anyway", or sign the installer (see below).

## Future release prep

Not implemented yet; tracked here for when we go public:

- **Version bump** — update `version` in `apps/desktop/src-tauri/tauri.conf.json`
  (the installer filename follows it).
- **Code signing** — sign `openmgmt-desktop.exe` and the NSIS installer with an
  Authenticode certificate to avoid SmartScreen.
- **GitHub Releases** — attach the `*-setup.exe` artifact.
- **Release feed** — publish installer assets and `release.json` as described below;
  the app uses a consent-based installer downloader, not the Tauri updater plugin.
- **Release channels** — alpha / beta / stable, distinguished by version suffix.

## Bundle CI and update feed

`.github/workflows/release.yml` builds universal macOS DMGs and Windows x64 NSIS
installers on `v*` tags or manual dispatch. It uploads workflow artifacts only:
creating a GitHub release, signing/notarizing, and publishing assets remain
explicit release-engineering steps. Regular PR CI checks the WASM UI and library
crates, not the native desktop shell; run the bundle workflow on the candidate
branch before shipping.

The app checks the latest ten GitHub releases on startup (at most once per hour)
and via Settings → Updates → Check now. Drafts and prereleases are excluded.
A release must contain a `release.json` asset and the matching installer asset:

```json
{
  "schema": 1,
  "files": [
    {
      "platform": "windows",
      "arch": "x86_64",
      "name": "OpenMgmt_0.2.0_x64-setup.exe",
      "sha256": "REPLACE_WITH_64_HEX_DIGITS_FROM_THE_FINAL_INSTALLER",
      "version_code": 101
    }
  ]
}
```

Use `macos` / `universal` for the universal DMG. Names must be portable filenames,
not paths; hashes must be computed after signing. The example hash is deliberately
invalid and must be replaced. Selection uses a strictly increasing numeric
`version_code`, not tag or semver ordering. For each release, increase
`UPDATE_VERSION_CODE` in `apps/desktop/src-tauri/src/updater.rs` (currently 100)
alongside the app version and use that code in the manifest. Never reuse a code.

Downloads require explicit consent. The app verifies SHA-256 before opening the
OS installer; mismatches delete the temporary download. This validates integrity
against the repository's manifest, not an independent signature. Repository
release permissions and platform signing remain security boundaries.

Before publishing, smoke-test both platforms: appearance persistence, Sync
sign-in/out, repository binding, prompt generation/model selection, dispatch,
session polling/cancel/follow-up/PR link, and update acceptance/dismissal. Exercise
an invalid manifest and checksum mismatch and confirm no installer opens.
Dispatch requires the deployed service to implement the `/v1` contract in
`crates/openmgmt-sync-client/src/dispatch.rs`; automated local checks do not prove
live-service compatibility. Repository bindings and agent metadata are local-only
and are intentionally not synced between devices.
