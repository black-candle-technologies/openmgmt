# Scheduling Core

OpenMgmt scheduling is local-first. The scheduling core stores planned work in
the same SQLite database as organizations, projects, tasks, timers, and saved
views. It does not call Google, Outlook, Apple, or any other calendar provider.

## Task Scheduling Fields

Tasks retain the existing `scheduled_at` field for compatibility. New scheduling
operations also maintain:

- `scheduled_start_at`
- `scheduled_end_at`
- `deadline_at`
- `reminder_at`
- `recurrence_rule`
- `recurrence_anchor_at`
- `recurrence_timezone`
- `calendar_block_id`

New scheduling APIs treat `scheduled_start_at` and `scheduled_end_at` as the
authoritative planned range. `scheduled_at` mirrors the start time so existing
Daily Operations and UI code remains compatible.

## Calendar Blocks

The `calendar_blocks` table stores local time blocks. A block can reference a
task, project, and organization.

Sources:

- `openmgmt`
- `imported_ics`
- `google_calendar_future`
- `outlook_future`

Statuses:

- `planned`
- `completed`
- `skipped`
- `moved`
- `canceled`

Rescheduling preserves the previous block as `moved` and creates a new planned
block. Clearing a task schedule cancels its active block and removes its
schedule, reminder, and recurrence metadata.

## Conflict Detection

Two planned blocks conflict when their ranges overlap:

```text
first.start_at < second.end_at
second.start_at < first.end_at
```

Completed, skipped, moved, and canceled blocks are ignored. Conflicts are
allowed and reported rather than rejected.

## Recurrence v1

Supported recurrence rules:

- `none`
- `daily`
- `weekdays`
- `weekly`
- `monthly`

Completing a scheduled recurring block completes the current task and creates
the next task occurrence with the same duration and recurrence rule. Weekday
recurrence skips Saturday and Sunday. Monthly recurrence uses calendar-month
arithmetic.

Complex RRULE expressions, exception dates, locale-specific holidays, and
daylight-saving timezone conversion are not implemented yet. Times are stored
in UTC; the timezone field is retained as metadata for future calendar UI and
provider integrations.

## Core and Tauri Commands

- `get_schedule_today`
- `get_schedule_week`
- `get_schedule_for_day`
- `get_unscheduled_tasks`
- `get_overdue_tasks`
- `auto_start_due_scheduled_tasks`
- `schedule_task`
- `reschedule_task`
- `clear_task_schedule`
- `list_schedule_conflicts`
- `suggest_next_time_block`
- `suggest_tasks_for_time_window`
- `complete_scheduled_block`
- `skip_scheduled_block`
- `generate_schedule_ics`

`auto_start_due_scheduled_tasks` is the backend polling hook for starting
scheduled blocks when the current time enters their planned range.

No desktop calendar UI is included in this phase.

## Board Integration

Backend board classification understands scheduled ranges:

- a currently active scheduled range appears in NOW
- future work scheduled today appears in LATER TODAY
- an elapsed unfinished scheduled range appears in OVERDUE
- completed scheduled tasks appear in DONE TODAY
- unscheduled work continues through the existing scoring path

P1 remains the highest priority.

## ICS Export

`generate_schedule_ics` returns an RFC 5545-style `VCALENDAR` string containing
local scheduled blocks. Canceled, skipped, and moved blocks are omitted. The command
returns text and does not write to the filesystem.

## Future Integrations

Planned follow-up work:

- agenda and calendar desktop views
- reminder delivery
- ICS import and duplicate matching
- Google Calendar OAuth and two-way mapping
- Outlook/Microsoft Graph integration
- Apple Calendar workflows through ICS or platform APIs
- timezone-aware recurrence expansion and conflict reconciliation
