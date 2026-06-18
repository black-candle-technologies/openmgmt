//! Reusable schedule/reschedule modal.
//!
//! A standalone scheduling drawer that needs only [`AppState`] and a [`Task`], so
//! it can be opened from anywhere (the Tasks page, record rows, …) without the
//! Schedule page's drag-and-drop context. It mirrors the Schedule page modal's
//! fields — date, start, end, reminder, recurrence — and writes through the same
//! `schedule_task` / `reschedule_task` commands.
//!
//! Times are entered in the viewer's local timezone and bridged to UTC via the
//! shared helpers in [`crate::app::state`], matching the rest of the app.

use chrono::{Duration, Utc};
use leptos::prelude::*;
use openmgmt_core::{CalendarBlock, RecurrenceRule, ScheduleTaskInput, Task};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::FormField;
use super::state::*;

/// Recurrence choices offered in the modal (kept in sync with the Schedule page).
const RECURRENCE_OPTIONS: [(RecurrenceRule, &str); 5] = [
    (RecurrenceRule::None, "Does not repeat"),
    (RecurrenceRule::Daily, "Daily"),
    (RecurrenceRule::Weekdays, "Weekdays (Mon–Fri)"),
    (RecurrenceRule::Weekly, "Weekly"),
    (RecurrenceRule::Monthly, "Monthly"),
];

/// Schedule or reschedule a single task. `on_close` is invoked when the modal
/// should be dismissed (cancel, backdrop, or a successful submit).
#[component]
pub fn ScheduleTaskModal(state: AppState, task: Task, on_close: Callback<()>) -> impl IntoView {
    let reschedule = task.calendar_block_id.is_some();
    let task_id = task.id.clone();
    let heading = if reschedule {
        "Reschedule task"
    } else {
        "Schedule task"
    };
    let task_title = task.title.clone();

    let init_date = task
        .scheduled_start_at
        .map(local_date_str)
        .unwrap_or_else(|| local_date_str(Utc::now()));
    let init_start = task
        .scheduled_start_at
        .map(local_time_str)
        .unwrap_or_else(|| "09:00".to_string());
    let default_minutes = task
        .estimated_minutes
        .or(task.time_limit_minutes)
        .map(i64::from)
        .unwrap_or(60)
        .clamp(5, 24 * 60);
    let init_end = match (task.scheduled_start_at, task.scheduled_end_at) {
        (_, Some(end)) => local_time_str(end),
        (Some(start), None) => local_time_str(start + Duration::minutes(default_minutes)),
        _ => "10:00".to_string(),
    };
    let init_reminder = datetime_local_value(task.reminder_at);
    let init_rule = task.recurrence_rule.unwrap_or(RecurrenceRule::None);
    let deadline = task.deadline_at;

    let date_ref = NodeRef::<leptos::html::Input>::new();
    let start_ref = NodeRef::<leptos::html::Input>::new();
    let end_ref = NodeRef::<leptos::html::Input>::new();
    let reminder_ref = NodeRef::<leptos::html::Input>::new();
    let recur_ref = NodeRef::<leptos::html::Select>::new();

    view! {
        <div class="sched-modal-layer">
            <div class="sched-modal-backdrop" on:click=move |_| on_close.run(())></div>
            <div class="sched-modal">
                <header class="sched-modal-head">
                    <h2>{heading}</h2>
                    <button class="icon-btn" type="button" on:click=move |_| on_close.run(())>"✕"</button>
                </header>
                <p class="sched-modal-task">{task_title}</p>
                <form class="sched-modal-form" on:submit=move |event| {
                    event.prevent_default();
                    let date = input_value(date_ref);
                    let start_t = input_value(start_ref);
                    let end_t = input_value(end_ref);
                    if date.is_empty() || start_t.is_empty() || end_t.is_empty() {
                        state.fail("Schedule failed", "Date, start time, and end time are required.".into());
                        return;
                    }
                    let start_at = match combine_local(&date, &start_t) {
                        Ok(value) => value,
                        Err(error) => { state.fail("Schedule failed", error); return; }
                    };
                    let end_at = match combine_local(&date, &end_t) {
                        Ok(value) => value,
                        Err(error) => { state.fail("Schedule failed", error); return; }
                    };
                    if end_at <= start_at {
                        state.fail("Schedule failed", "End time must be after start time.".into());
                        return;
                    }
                    let reminder_at = match parse_datetime_local(input_value(reminder_ref)) {
                        Ok(value) => value,
                        Err(error) => { state.fail("Schedule failed", error); return; }
                    };
                    let rule: RecurrenceRule = select_value(recur_ref).parse().unwrap_or(RecurrenceRule::None);
                    let input = ScheduleTaskInput {
                        start_at,
                        end_at,
                        timezone: None,
                        reminder_at,
                        deadline_at: deadline,
                        recurrence_rule: Some(rule),
                        recurrence_anchor_at: None,
                        recurrence_timezone: None,
                    };
                    let command = if reschedule { "reschedule_task" } else { "schedule_task" };
                    let id = task_id.clone();
                    spawn_local(async move {
                        let result = invoke::<CalendarBlock>(command, json!({ "taskId": id, "input": input })).await;
                        match result {
                            Ok(_) => {
                                state.notice.set(Some(
                                    if reschedule { "Task rescheduled." } else { "Task scheduled." }.to_string(),
                                ));
                                state.reload().await;
                            }
                            Err(error) => state.fail(
                                if reschedule { "Reschedule failed" } else { "Schedule failed" },
                                error,
                            ),
                        }
                    });
                    on_close.run(());
                }>
                    <FormField label="Date">
                        <input node_ref=date_ref type="date" value=init_date required />
                    </FormField>
                    <div class="form-row">
                        <FormField label="Start">
                            <input node_ref=start_ref type="time" value=init_start required />
                        </FormField>
                        <FormField label="End">
                            <input node_ref=end_ref type="time" value=init_end required />
                        </FormField>
                    </div>
                    <FormField label="Reminder" hint="Optional">
                        <input node_ref=reminder_ref type="datetime-local" value=init_reminder />
                    </FormField>
                    <FormField label="Repeat">
                        <select node_ref=recur_ref>
                            {RECURRENCE_OPTIONS.into_iter().map(|(value, label)| {
                                let selected = value == init_rule;
                                view! { <option value=value.to_string() selected=selected>{label}</option> }
                            }).collect_view()}
                        </select>
                    </FormField>
                    <div class="sched-modal-actions">
                        <button class="btn btn-primary" type="submit">{if reschedule { "Reschedule" } else { "Schedule" }}</button>
                        <button class="btn btn-subtle" type="button" on:click=move |_| on_close.run(())>"Cancel"</button>
                    </div>
                </form>
            </div>
        </div>
    }
}
