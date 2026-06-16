# Sync UI Manual Verification

OpenMgmt remains local-first. Sync is optional, manual, and does not prevent
normal task or project work when a server is unavailable.

1. Start the desktop app with no sync server.
2. Open **Sync** settings from the sidebar.
3. Confirm sync is disabled by default.
4. Enable sync with no server URL and save.
5. Confirm status says **Not configured**.
6. Set the server URL to `http://127.0.0.1:8787`.
7. Start `openmgmt-server` locally.
8. Click **Test connection**.
9. Confirm a compatible server response appears.
10. Create a task locally.
11. Click **Sync now**.
12. Confirm pushed and accepted counts update.
13. Stop the server.
14. Click **Test connection** or **Sync now**.
15. Confirm an error appears but the app still works locally.
16. Click **Clear error** and confirm the error disappears.

Additional checks:

- Save an HTTP URL and confirm it is accepted; HTTPS is not required.
- Enter a URL without `http://` or `https://` and confirm the UI shows a hint.
- Confirm the device ID is secondary/collapsed and no device token is shown.
- Confirm **Sync now** is disabled and labeled **Syncing...** during a run.
- Confirm the latest result shows pushed, accepted, rejected, pulled, applied,
  and checkpoint values when returned.
