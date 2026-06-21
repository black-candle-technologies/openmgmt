# Background Sync

OpenMGMT remains local-first. Background sync is optional, only runs when sync is enabled and a server URL is configured, and shares the same non-overlapping sync guard as manual Sync Now.

## Manual Verification Checklist

1. Start openmgmt-server locally.
2. Open desktop app.
3. Enable sync and set server URL.
4. Confirm startup sync runs without blocking app startup.
5. Create a task.
6. Wait for debounce interval.
7. Confirm task syncs without pressing Sync Now.
8. Create several tasks quickly.
9. Confirm only one debounced sync attempt occurs.
10. Open another local app/database and confirm changes arrive after periodic/manual sync.
11. Stop server.
12. Confirm background sync records an error but app remains usable.
13. Restart server.
14. Confirm later background sync recovers.
15. Click Sync Now while background sync is running.
16. Confirm overlap is handled cleanly.
17. Disable sync.
18. Confirm background sync stops.
