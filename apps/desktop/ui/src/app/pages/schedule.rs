//! Scheduling / calendar workspace.
//!
//! A usable planner over the scheduling core Codex added: a Today timeline, a
//! seven-day week agenda, an unscheduled-task queue, an overdue list, and a live
//! conflict report. Tasks can be planned by drag-and-drop (onto an hour slot or a
//! day) or via a lightweight schedule/reschedule modal as a reliable fallback.
//!
//! Scheduled datetimes are stored in UTC; everything here renders and accepts
//! local time via the shared helpers in [`crate::app::state`], matching how the
//! rest of the app bridges the local `datetime-local`/`date`/`time` inputs.

use chrono::{DateTime, Duration, Utc};
use leptos::prelude::*;
use openmgmt_core::{
    CalendarBlock, RecurrenceRule, ScheduleConflict, ScheduleTaskInput, ScheduledBlockCompletion,
    Task, TaskWithContext,
};
use serde_json::json;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::app::components::*;
use crate::app::state::*;
use crate::app::tags::TagChip;

/// Recurrence choices offered in the schedule modal.
const RECURRENCE_OPTIONS: [(RecurrenceRule, &str); 5] = [
    (RecurrenceRule::None, "Does not repeat"),
    (RecurrenceRule::Daily, "Daily"),
    (RecurrenceRule::Weekdays, "Weekdays (Mon–Fri)"),
    (RecurrenceRule::Weekly, "Weekly"),
    (RecurrenceRule::Monthly, "Monthly"),
];

/// All schedule data the page renders, loaded together so the views stay in sync.
#[derive(Clone, Default)]
struct ScheduleData {
    today: Vec<TaskWithContext>,
    week: Vec<TaskWithContext>,
    unscheduled: Vec<TaskWithContext>,
    overdue: Vec<TaskWithContext>,
    conflicts: Vec<ScheduleConflict>,
}

/// The payload carried while a task is being dragged. Carrying the existing
/// reminder/deadline/recurrence means a drag-reschedule preserves that metadata
/// (the backend overwrites those fields from the schedule input).
#[derive(Clone)]
struct DragData {
    task_id: String,
    /// Present when the task is already on the calendar (so a drop reschedules).
    block_id: Option<String>,
    duration_minutes: i64,
    reminder_at: Option<DateTime<Utc>>,
    deadline_at: Option<DateTime<Utc>>,
    recurrence_rule: Option<RecurrenceRule>,
    recurrence_timezone: Option<String>,
}

/// The task targeted by the schedule/reschedule modal.
#[derive(Clone)]
struct ScheduleTarget {
    task: Task,
    reschedule: bool,
}

impl ScheduleTarget {
    fn new(task: Task) -> Self {
        let reschedule = task.calendar_block_id.is_some();
        Self { task, reschedule }
    }
}

/// One column of the week agenda.
struct DayColumn {
    y: i32,
    mo: u32,
    d: u32,
    weekday: &'static str,
    is_today: bool,
}

/// Shared, `Copy` handle bundling every signal the schedule surfaces need, so
/// sub-components and event handlers take a single `ctx` instead of a long prop
/// list. All fields are `Copy` signal handles, so `Sched` itself is `Copy`.
#[derive(Clone, Copy)]
struct Sched {
    state: AppState,
    data: RwSignal<ScheduleData>,
    loading: RwSignal<bool>,
    generation: StoredValue<u32>,
    drag: RwSignal<Option<DragData>>,
    active_zone: RwSignal<Option<String>>,
    modal: RwSignal<Option<ScheduleTarget>>,
    ics: RwSignal<Option<String>>,
    now: Signal<DateTime<Utc>>,
}

impl Sched {
    /// Reload every schedule slice, guarding against overlapping refreshes with a
    /// generation token (latest request wins) the same way the other pages do.
    fn reload(self) {
        let token = self.generation.get_value().wrapping_add(1);
        self.generation.set_value(token);
        self.loading.set(true);
        spawn_local(async move {
            let result = load_schedule_data().await;
            if self.generation.get_value() != token {
                return;
            }
            match result {
                Ok(next) => {
                    self.data.set(next);
                    self.state.error.set(None);
                }
                Err(error) => self.state.fail("Schedule load failed", error),
            }
            self.loading.set(false);
        });
    }

    /// Schedule or reschedule a task, then refresh and surface a conflict warning
    /// if the resulting block overlaps another planned block.
    fn run(self, task_id: String, input: ScheduleTaskInput, reschedule: bool) {
        spawn_local(async move {
            let command = if reschedule {
                "reschedule_task"
            } else {
                "schedule_task"
            };
            match invoke::<CalendarBlock>(command, json!({ "task_id": task_id, "input": input }))
                .await
            {
                Ok(block) => {
                    self.state.notice.set(Some(
                        if reschedule {
                            "Task rescheduled."
                        } else {
                            "Task scheduled."
                        }
                        .to_string(),
                    ));
                    self.reload();
                    self.state.refresh();
                    if let Ok(conflicts) =
                        invoke::<Vec<ScheduleConflict>>("list_schedule_conflicts", json!({})).await
                        && conflicts
                            .iter()
                            .any(|c| c.first.id == block.id || c.second.id == block.id)
                    {
                        self.state.notice.set(Some(
                            "Scheduled — heads up: this overlaps another block. See Conflicts below."
                                .to_string(),
                        ));
                    }
                }
                Err(error) => self.state.fail(
                    if reschedule {
                        "Reschedule failed"
                    } else {
                        "Schedule failed"
                    },
                    error,
                ),
            }
        });
    }

    fn clear(self, task_id: String) {
        spawn_local(async move {
            match invoke::<Task>("clear_task_schedule", json!({ "task_id": task_id })).await {
                Ok(_) => {
                    self.state.notice.set(Some("Schedule cleared.".to_string()));
                    self.reload();
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Clear schedule failed", error),
            }
        });
    }

    fn complete_block(self, block_id: String) {
        spawn_local(async move {
            match invoke::<ScheduledBlockCompletion>(
                "complete_scheduled_block",
                json!({ "block_id": block_id }),
            )
            .await
            {
                Ok(result) => {
                    self.state.notice.set(Some(
                        if result.next_occurrence_task.is_some() {
                            "Block completed — next occurrence scheduled."
                        } else {
                            "Block completed."
                        }
                        .to_string(),
                    ));
                    self.reload();
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Complete block failed", error),
            }
        });
    }

    fn skip_block(self, block_id: String) {
        spawn_local(async move {
            match invoke::<CalendarBlock>("skip_scheduled_block", json!({ "block_id": block_id }))
                .await
            {
                Ok(_) => {
                    self.state.notice.set(Some("Block skipped.".to_string()));
                    self.reload();
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Skip block failed", error),
            }
        });
    }

    fn complete_task(self, task_id: String) {
        spawn_local(async move {
            match invoke::<Task>("complete_task", json!({ "id": task_id })).await {
                Ok(_) => {
                    self.state.notice.set(Some("Task completed.".to_string()));
                    self.reload();
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Complete task failed", error),
            }
        });
    }

    /// Drop the currently-dragged task onto a local date + whole-hour slot.
    fn drop_at(self, y: i32, mo: u32, d: u32, hour: u32) {
        self.active_zone.set(None);
        let Some(drag) = self.drag.get_untracked() else {
            return;
        };
        self.drag.set(None);
        match local_to_utc(y, mo, d, hour) {
            Ok(start) => {
                let input = ScheduleTaskInput {
                    start_at: start,
                    end_at: start + Duration::minutes(drag.duration_minutes.max(5)),
                    timezone: None,
                    reminder_at: drag.reminder_at,
                    deadline_at: drag.deadline_at,
                    recurrence_rule: drag.recurrence_rule,
                    recurrence_anchor_at: None,
                    recurrence_timezone: drag.recurrence_timezone.clone(),
                };
                self.run(drag.task_id.clone(), input, drag.block_id.is_some());
            }
            Err(error) => self.state.fail("Schedule failed", error),
        }
    }

    /// Drop onto the unschedule zone: only meaningful for an already-scheduled task.
    fn drop_unschedule(self) {
        self.active_zone.set(None);
        let Some(drag) = self.drag.get_untracked() else {
            return;
        };
        self.drag.set(None);
        if drag.block_id.is_some() {
            self.clear(drag.task_id.clone());
        }
    }
}

async fn load_schedule_data() -> Result<ScheduleData, String> {
    Ok(ScheduleData {
        today: invoke("get_schedule_today", json!({})).await?,
        week: invoke("get_schedule_week", json!({})).await?,
        unscheduled: invoke("get_unscheduled_tasks", json!({})).await?,
        overdue: invoke("get_overdue_tasks", json!({})).await?,
        conflicts: invoke("list_schedule_conflicts", json!({})).await?,
    })
}

// --- small helpers ---------------------------------------------------------

/// Build a UTC instant from a local date + whole hour by reusing the tested
/// `datetime-local` → UTC bridge.
fn local_to_utc(y: i32, mo: u32, d: u32, hour: u32) -> Result<DateTime<Utc>, String> {
    combine_local(&format!("{y:04}-{mo:02}-{d:02}"), &format!("{hour:02}:00"))
}

/// Combine a `YYYY-MM-DD` date and `HH:MM` time (both local) into a UTC instant.
fn combine_local(date: &str, time: &str) -> Result<DateTime<Utc>, String> {
    parse_datetime_local(format!("{date}T{time}"))?
        .ok_or_else(|| "Invalid date or time.".to_string())
}

fn drag_from_task(task: &Task) -> DragData {
    let duration = match (task.scheduled_start_at, task.scheduled_end_at) {
        (Some(start), Some(end)) => (end - start).num_minutes().max(5),
        _ => task
            .estimated_minutes
            .or(task.time_limit_minutes)
            .map(i64::from)
            .unwrap_or(60)
            .clamp(5, 24 * 60),
    };
    DragData {
        task_id: task.id.clone(),
        block_id: task.calendar_block_id.clone(),
        duration_minutes: duration,
        reminder_at: task.reminder_at,
        deadline_at: task.deadline_at,
        recurrence_rule: task.recurrence_rule,
        recurrence_timezone: task.recurrence_timezone.clone(),
    }
}

/// Visual state of a scheduled block relative to the live clock.
fn block_state(
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> &'static str {
    match (start, end) {
        (Some(s), Some(e)) if s <= now && now < e => "active",
        (_, Some(e)) if e <= now => "overdue",
        (Some(s), None) if s <= now => "active",
        _ => "upcoming",
    }
}

/// Seven local day columns for the current Monday-based week.
fn week_columns(now: DateTime<Utc>) -> Vec<DayColumn> {
    let weekday = local_weekday(now); // 0 = Sunday … 6 = Saturday
    let offset = if weekday == 0 { 6 } else { weekday - 1 } as i64;
    let day_ms = 86_400_000i64;
    let base_ms = now.timestamp_millis();
    let today = local_ymd(now);
    (0..7)
        .map(|i| {
            let ms = base_ms - offset * day_ms + (i as i64) * day_ms;
            let dt = DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or(now);
            let (y, mo, d) = local_ymd(dt);
            DayColumn {
                y,
                mo,
                d,
                weekday: weekday_short(local_weekday(dt)),
                is_today: (y, mo, d) == today,
            }
        })
        .collect()
}

fn allow_drop(ev: &web_sys::DragEvent) {
    ev.prevent_default();
    if let Some(transfer) = ev.data_transfer() {
        transfer.set_drop_effect("move");
    }
}

fn begin_drag(ev: &web_sys::DragEvent, label: &str) {
    if let Some(transfer) = ev.data_transfer() {
        let _ = transfer.set_data("text/plain", label);
        transfer.set_effect_allowed("move");
    }
}

fn copy_to_clipboard(state: AppState, text: String) {
    let Some(clipboard) = web_sys::window().map(|window| window.navigator().clipboard()) else {
        state.fail("Copy failed", "clipboard is unavailable".into());
        return;
    };
    let promise = clipboard.write_text(&text);
    spawn_local(async move {
        match JsFuture::from(promise).await {
            Ok(_) => state.notice.set(Some("ICS copied to clipboard.".into())),
            Err(_) => state.fail("Copy failed", "clipboard write was blocked".into()),
        }
    });
}

// --- page ------------------------------------------------------------------

#[component]
pub fn SchedulePage(state: AppState, now: RwSignal<DateTime<Utc>>) -> impl IntoView {
    let ctx = Sched {
        state,
        data: RwSignal::new(ScheduleData::default()),
        loading: RwSignal::new(true),
        generation: StoredValue::new(0),
        drag: RwSignal::new(None),
        active_zone: RwSignal::new(None),
        modal: RwSignal::new(None),
        ics: RwSignal::new(None),
        now: now.into(),
    };

    // Reload on mount and whenever the global snapshot refreshes (every 10s, and
    // after our own mutations call `state.refresh()`), so the schedule never
    // shows stale data after a drag/drop or an edit elsewhere.
    Effect::new(move |_| {
        let _ = ctx.state.synced_at.get();
        ctx.reload();
    });

    let reload = Callback::new(move |_| ctx.reload());
    let export = Callback::new(move |_| {
        spawn_local(async move {
            match invoke::<String>("generate_schedule_ics", json!({})).await {
                Ok(text) => ctx.ics.set(Some(text)),
                Err(error) => ctx.state.fail("Export ICS failed", error),
            }
        });
    });

    view! {
        <PageHeader
            eyebrow="SCHEDULING"
            title="Schedule"
            description="Plan work across today, this week, and unscheduled tasks."
        >
            <Button variant="ghost" on_click=reload>"Refresh"</Button>
            <button
                class="btn btn-ghost sched-conflict-chip"
                class:has-conflicts=move || !ctx.data.get().conflicts.is_empty()
                type="button"
                title="Re-check schedule conflicts"
                on:click=move |_| ctx.reload()
            >
                "Conflicts"
                <span class="sched-conflict-count">{move || ctx.data.get().conflicts.len()}</span>
            </button>
            <Button variant="primary" on_click=export>"Export ICS"</Button>
        </PageHeader>

        {move || (!ctx.data.get().conflicts.is_empty()).then(|| {
            let n = ctx.data.get().conflicts.len();
            view! {
                <div class="banner sched-conflict-banner">
                    <span>{format!("⚠ {n} scheduling conflict{} — overlapping blocks are listed below.", if n == 1 { "" } else { "s" })}</span>
                </div>
            }
        })}

        <div class="sched-layout">
            <div class="sched-main"><TodayTimeline ctx /></div>
            <div class="sched-side">
                <UnscheduledPanel ctx />
                <OverduePanel ctx />
            </div>
        </div>

        <WeekView ctx />
        <ConflictsPanel ctx />

        <ScheduleModal ctx />
        <IcsModal ctx />
    }
}

// --- Today timeline --------------------------------------------------------

#[component]
fn TodayTimeline(ctx: Sched) -> impl IntoView {
    view! {
        <section class="panel sched-timeline-panel">
            <div class="section-head">
                <div class="section-head-title">
                    <h2>"Today"</h2>
                    <span class="count-chip">{move || ctx.data.get().today.len()}</span>
                </div>
                <span class="sched-today-date">
                    {move || { let (_, mo, d) = local_ymd(Utc::now()); format!("{} {}", month_short(mo), d) }}
                </span>
            </div>
            {move || {
                let today = ctx.data.get().today;
                if today.is_empty() && ctx.loading.get() {
                    return view! { <LoadingState label="Loading schedule…" /> }.into_any();
                }
                let (ty, tmo, td) = local_ymd(Utc::now());

                // Core working window 8 AM–8 PM, widened to include any block that
                // falls outside it so nothing is ever hidden.
                let mut min_h = 8u32;
                let mut max_h = 20u32;
                for row in &today {
                    if let Some(start) = row.task.scheduled_start_at {
                        let sh = local_hour(start);
                        min_h = min_h.min(sh);
                        max_h = max_h.max((sh + 1).min(24));
                    }
                    if let Some(end) = row.task.scheduled_end_at {
                        max_h = max_h.max((local_hour(end) + 1).min(24));
                    }
                }
                let hours: Vec<u32> = (min_h..max_h.min(24)).collect();
                let empty = today.is_empty();

                view! {
                    {empty.then(|| view! {
                        <p class="sched-empty-hint">"Nothing scheduled today. Drag an unscheduled task here to plan your day."</p>
                    })}
                    <div class="sched-timeline">
                        {hours.into_iter().map(|h| {
                            let blocks: Vec<TaskWithContext> = today
                                .iter()
                                .filter(|row| row.task.scheduled_start_at.map(local_hour) == Some(h))
                                .cloned()
                                .collect();
                            view! {
                                <div
                                    class="sched-slot"
                                    class:drop-active=move || ctx.active_zone.get().as_deref() == Some(format!("today-{h}").as_str())
                                    on:dragover=move |ev| {
                                        allow_drop(&ev);
                                        let id = format!("today-{h}");
                                        if ctx.active_zone.get_untracked().as_deref() != Some(id.as_str()) {
                                            ctx.active_zone.set(Some(id));
                                        }
                                    }
                                    on:drop=move |ev| { ev.prevent_default(); ctx.drop_at(ty, tmo, td, h); }
                                >
                                    <span class="sched-slot-label">{hour_label(h)}</span>
                                    <div class="sched-slot-body">
                                        {if blocks.is_empty() {
                                            view! { <span class="sched-slot-empty"></span> }.into_any()
                                        } else {
                                            blocks.into_iter().map(|row| view! { <ScheduledCard ctx row /> }).collect_view().into_any()
                                        }}
                                    </div>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn ScheduledCard(ctx: Sched, row: TaskWithContext) -> impl IntoView {
    let task = row.task.clone();
    let start = task.scheduled_start_at;
    let end = task.scheduled_end_at;
    let title = task.title.clone();
    let priority = task.priority;
    let status_str = task.status.to_string();
    let project = row.project_name.clone();
    let org = row.organization_name.clone();
    let org_color = row
        .organization_color
        .clone()
        .unwrap_or_else(|| "#7c867c".into());
    let tags = task.tags.clone();
    let recurrence = task
        .recurrence_rule
        .filter(|rule| *rule != RecurrenceRule::None);
    let time_label = match (start, end) {
        (Some(s), Some(e)) => fmt_time_range(s, e),
        (Some(s), None) => fmt_time(s),
        _ => "—".to_string(),
    };
    let block_id = task.calendar_block_id.clone();

    let drag_task = task.clone();
    let drag_label = title.clone();
    let title_task = task.clone();
    let resched_task = task.clone();
    let clear_id = task.id.clone();

    let card_class = move || {
        format!(
            "sched-card sched-card-{}",
            block_state(start, end, ctx.now.get())
        )
    };

    view! {
        <article
            class=card_class
            draggable="true"
            on:dragstart=move |ev| { ctx.drag.set(Some(drag_from_task(&drag_task))); begin_drag(&ev, &drag_label); }
            on:dragend=move |_| ctx.active_zone.set(None)
        >
            <div class="sched-card-head">
                <PriorityBadge value=priority />
                <span class="sched-card-time">{time_label}</span>
                {recurrence.map(|rule| view! { <span class="sched-recur" title="Repeats">{"↻ "}{recurrence_label(rule)}</span> })}
            </div>
            <button class="sched-card-title" on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(title_task.clone())))>{title}</button>
            <div class="sched-card-meta">
                <StatusBadge status=status_str />
                <span class="er-org-dot" style=format!("background:{org_color}")></span>
                <span class="sched-card-project">{project}" · "{org}</span>
            </div>
            {(!tags.is_empty()).then(|| view! {
                <div class="sched-card-tags">{tags.into_iter().take(4).map(|tag| view! { <TagChip tag /> }).collect_view()}</div>
            })}
            <div class="sched-card-actions">
                <button class="btn btn-subtle sched-mini" type="button" on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(resched_task.clone())))>"Reschedule"</button>
                {block_id.clone().map(|id| {
                    let complete_id = id.clone();
                    let skip_id = id;
                    view! {
                        <button class="btn btn-primary sched-mini" type="button" on:click=move |_| ctx.complete_block(complete_id.clone())>"Complete"</button>
                        <button class="btn btn-subtle sched-mini" type="button" on:click=move |_| ctx.skip_block(skip_id.clone())>"Skip"</button>
                    }
                })}
                {block_id.is_some().then(|| view! {
                    <button class="btn btn-danger-soft sched-mini" type="button" on:click=move |_| ctx.clear(clear_id.clone())>"Clear"</button>
                })}
            </div>
        </article>
    }
}

// --- Unscheduled -----------------------------------------------------------

#[component]
fn UnscheduledPanel(ctx: Sched) -> impl IntoView {
    view! {
        <section
            class="panel sched-side-panel"
            class:drop-active=move || ctx.active_zone.get().as_deref() == Some("unschedule")
            on:dragover=move |ev| {
                allow_drop(&ev);
                if ctx.active_zone.get_untracked().as_deref() != Some("unschedule") {
                    ctx.active_zone.set(Some("unschedule".to_string()));
                }
            }
            on:drop=move |ev| { ev.prevent_default(); ctx.drop_unschedule(); }
        >
            <div class="section-head">
                <div class="section-head-title">
                    <h2>"Unscheduled"</h2>
                    <span class="count-chip">{move || ctx.data.get().unscheduled.len()}</span>
                </div>
            </div>
            <p class="sched-side-hint">"Drag a task onto a time slot or day — or drop a scheduled block here to unschedule it."</p>
            {move || {
                let mut rows = ctx.data.get().unscheduled;
                if rows.is_empty() {
                    if ctx.loading.get() {
                        return view! { <LoadingState label="Loading tasks…" /> }.into_any();
                    }
                    return view! { <EmptyState title="No unscheduled tasks" hint="Everything open is already on the calendar." /> }.into_any();
                }
                // Highest priority first (P1 above P5), urgency as the tiebreak.
                rows.sort_by(|a, b| a.task.priority.cmp(&b.task.priority).then(b.urgency_score.cmp(&a.urgency_score)));
                view! {
                    <div class="sched-card-list">
                        {rows.into_iter().map(|row| view! { <UnscheduledCard ctx row /> }).collect_view()}
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn UnscheduledCard(ctx: Sched, row: TaskWithContext) -> impl IntoView {
    let task = row.task.clone();
    let title = task.title.clone();
    let priority = task.priority;
    let project = row.project_name.clone();
    let org = row.organization_name.clone();
    let org_color = row
        .organization_color
        .clone()
        .unwrap_or_else(|| "#7c867c".into());
    let tags = task.tags.clone();
    let estimate = task.estimated_minutes.or(task.time_limit_minutes);

    let drag_task = task.clone();
    let drag_label = title.clone();
    let title_task = task.clone();
    let schedule_task = task;

    view! {
        <article
            class="sched-card sched-card-queued"
            draggable="true"
            on:dragstart=move |ev| { ctx.drag.set(Some(drag_from_task(&drag_task))); begin_drag(&ev, &drag_label); }
            on:dragend=move |_| ctx.active_zone.set(None)
        >
            <div class="sched-card-head">
                <PriorityBadge value=priority />
                {estimate.map(|minutes| view! { <span class="sched-card-est">{format!("~{minutes}m")}</span> })}
            </div>
            <button class="sched-card-title" on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(title_task.clone())))>{title}</button>
            <div class="sched-card-meta">
                <span class="er-org-dot" style=format!("background:{org_color}")></span>
                <span class="sched-card-project">{project}" · "{org}</span>
            </div>
            {(!tags.is_empty()).then(|| view! {
                <div class="sched-card-tags">{tags.into_iter().take(4).map(|tag| view! { <TagChip tag /> }).collect_view()}</div>
            })}
            <div class="sched-card-actions">
                <button class="btn btn-primary sched-mini" type="button" on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(schedule_task.clone())))>"Schedule"</button>
            </div>
        </article>
    }
}

// --- Overdue ---------------------------------------------------------------

#[component]
fn OverduePanel(ctx: Sched) -> impl IntoView {
    view! {
        <section class="panel sched-side-panel sched-overdue-panel">
            <div class="section-head">
                <div class="section-head-title">
                    <h2 class="sched-overdue-title">"Overdue"</h2>
                    <span class="count-chip">{move || ctx.data.get().overdue.len()}</span>
                </div>
            </div>
            {move || {
                let rows = ctx.data.get().overdue;
                if rows.is_empty() {
                    if ctx.loading.get() {
                        return view! { <LoadingState label="Checking…" /> }.into_any();
                    }
                    return view! { <p class="sched-clear">"Nothing overdue. Nice."</p> }.into_any();
                }
                view! {
                    <div class="sched-card-list">
                        {rows.into_iter().map(|row| view! { <OverdueRow ctx row /> }).collect_view()}
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn OverdueRow(ctx: Sched, row: TaskWithContext) -> impl IntoView {
    let task = row.task.clone();
    let title = task.title.clone();
    let priority = task.priority;
    let project = row.project_name.clone();
    let block_id = task.calendar_block_id.clone();
    let when = task
        .scheduled_end_at
        .or(task.scheduled_start_at)
        .or(task.due_at);
    let when_label = when
        .map(|value| format!("was {}", fmt_datetime(value)))
        .unwrap_or_else(|| "overdue".to_string());

    let title_task = task.clone();
    let resched_task = task.clone();
    let clear_id = task.id.clone();
    let complete_id = task.id.clone();
    let complete_block = block_id.clone();

    view! {
        <article class="sched-overdue-row">
            <div class="sched-overdue-main">
                <div class="sched-card-head">
                    <PriorityBadge value=priority />
                    <span class="sched-overdue-when">{when_label}</span>
                </div>
                <button class="sched-card-title" on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(title_task.clone())))>{title}</button>
                <span class="sched-card-project">{project}</span>
            </div>
            <div class="sched-card-actions">
                <button class="btn btn-subtle sched-mini" type="button" on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(resched_task.clone())))>"Reschedule"</button>
                <button class="btn btn-primary sched-mini" type="button" on:click=move |_| {
                    match &complete_block {
                        Some(id) => ctx.complete_block(id.clone()),
                        None => ctx.complete_task(complete_id.clone()),
                    }
                }>"Complete"</button>
                {block_id.is_some().then(|| view! {
                    <button class="btn btn-danger-soft sched-mini" type="button" on:click=move |_| ctx.clear(clear_id.clone())>"Clear"</button>
                })}
            </div>
        </article>
    }
}

// --- Week view -------------------------------------------------------------

#[component]
fn WeekView(ctx: Sched) -> impl IntoView {
    view! {
        <section class="panel sched-week-panel">
            <div class="section-head">
                <div class="section-head-title"><h2>"This week"</h2></div>
            </div>
            {move || {
                let week = ctx.data.get().week;
                let columns = week_columns(Utc::now());
                view! {
                    <div class="sched-week">
                        {columns.into_iter().map(|col| {
                            let (y, mo, d) = (col.y, col.mo, col.d);
                            let weekday = col.weekday;
                            let is_today = col.is_today;
                            let zone = format!("week-{y}-{mo}-{d}");
                            let zone_class = zone.clone();
                            let zone_over = zone;

                            let mut day_rows: Vec<TaskWithContext> = week
                                .iter()
                                .filter(|row| row.task.scheduled_start_at.map(local_ymd) == Some((y, mo, d)))
                                .cloned()
                                .collect();
                            day_rows.sort_by_key(|row| row.task.scheduled_start_at);

                            view! {
                                <div
                                    class=if is_today { "sched-week-day sched-week-today" } else { "sched-week-day" }
                                    class:drop-active=move || ctx.active_zone.get().as_deref() == Some(zone_class.as_str())
                                    on:dragover=move |ev| {
                                        allow_drop(&ev);
                                        if ctx.active_zone.get_untracked().as_deref() != Some(zone_over.as_str()) {
                                            ctx.active_zone.set(Some(zone_over.clone()));
                                        }
                                    }
                                    on:drop=move |ev| { ev.prevent_default(); ctx.drop_at(y, mo, d, 9); }
                                >
                                    <div class="sched-week-head">
                                        <span class="sched-week-dow">{weekday}</span>
                                        <span class="sched-week-date">{d}</span>
                                    </div>
                                    {if day_rows.is_empty() {
                                        view! { <p class="sched-week-empty">"—"</p> }.into_any()
                                    } else {
                                        view! {
                                            <div class="sched-week-list">
                                                {day_rows.into_iter().map(|row| view! { <WeekLine ctx row /> }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }
            }}
        </section>
    }
}

#[component]
fn WeekLine(ctx: Sched, row: TaskWithContext) -> impl IntoView {
    let task = row.task.clone();
    let title = task.title.clone();
    let priority = task.priority;
    let start_label = task.scheduled_start_at.map(fmt_time).unwrap_or_default();

    let drag_task = task.clone();
    let drag_label = title.clone();
    let modal_task = task;

    view! {
        <div
            class="sched-week-line"
            draggable="true"
            title="Drag to move · click to reschedule"
            on:dragstart=move |ev| { ctx.drag.set(Some(drag_from_task(&drag_task))); begin_drag(&ev, &drag_label); }
            on:dragend=move |_| ctx.active_zone.set(None)
            on:click=move |_| ctx.modal.set(Some(ScheduleTarget::new(modal_task.clone())))
        >
            <span class=format!("sched-dot priority-p{priority}")></span>
            <span class="sched-week-time">{start_label}</span>
            <span class="sched-week-line-title">{title}</span>
        </div>
    }
}

// --- Conflicts -------------------------------------------------------------

#[component]
fn ConflictsPanel(ctx: Sched) -> impl IntoView {
    view! {
        <section class="panel sched-conflicts-panel">
            <div class="section-head">
                <div class="section-head-title">
                    <h2>"Conflicts"</h2>
                    <span class="count-chip">{move || ctx.data.get().conflicts.len()}</span>
                </div>
                <div class="section-head-actions">
                    <button class="btn btn-ghost sched-mini" type="button" on:click=move |_| ctx.reload()>"Refresh conflicts"</button>
                </div>
            </div>
            {move || {
                let conflicts = ctx.data.get().conflicts;
                if conflicts.is_empty() {
                    return view! { <p class="sched-clear">"No scheduling conflicts."</p> }.into_any();
                }
                view! {
                    <div class="sched-conflict-list">
                        {conflicts.into_iter().map(|conflict| view! { <ConflictRow conflict /> }).collect_view()}
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn ConflictRow(conflict: ScheduleConflict) -> impl IntoView {
    let first = conflict.first;
    let second = conflict.second;
    view! {
        <div class="sched-conflict-row">
            <span class="sched-conflict-mark">"⚠"</span>
            <div class="sched-conflict-pair">
                <span class="sched-conflict-item">
                    <strong>{first.title}</strong>
                    <span class="sched-conflict-time">{fmt_time_range(first.start_at, first.end_at)}</span>
                </span>
                <span class="sched-conflict-vs">"overlaps"</span>
                <span class="sched-conflict-item">
                    <strong>{second.title}</strong>
                    <span class="sched-conflict-time">{fmt_time_range(second.start_at, second.end_at)}</span>
                </span>
            </div>
        </div>
    }
}

// --- Schedule / reschedule modal -------------------------------------------

#[component]
fn ScheduleModal(ctx: Sched) -> impl IntoView {
    move || {
        ctx.modal.get().map(|target| {
            let task = target.task.clone();
            let reschedule = target.reschedule;
            let task_id = task.id.clone();
            let heading = if reschedule { "Reschedule task" } else { "Schedule task" };
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
                    <div class="sched-modal-backdrop" on:click=move |_| ctx.modal.set(None)></div>
                    <div class="sched-modal">
                        <header class="sched-modal-head">
                            <h2>{heading}</h2>
                            <button class="icon-btn" type="button" on:click=move |_| ctx.modal.set(None)>"✕"</button>
                        </header>
                        <p class="sched-modal-task">{task_title}</p>
                        <form class="sched-modal-form" on:submit=move |event| {
                            event.prevent_default();
                            let date = input_value(date_ref);
                            let start_t = input_value(start_ref);
                            let end_t = input_value(end_ref);
                            if date.is_empty() || start_t.is_empty() || end_t.is_empty() {
                                ctx.state.fail("Schedule failed", "Date, start time, and end time are required.".into());
                                return;
                            }
                            let start_at = match combine_local(&date, &start_t) {
                                Ok(value) => value,
                                Err(error) => { ctx.state.fail("Schedule failed", error); return; }
                            };
                            let end_at = match combine_local(&date, &end_t) {
                                Ok(value) => value,
                                Err(error) => { ctx.state.fail("Schedule failed", error); return; }
                            };
                            if end_at <= start_at {
                                ctx.state.fail("Schedule failed", "End time must be after start time.".into());
                                return;
                            }
                            let reminder_at = match parse_datetime_local(input_value(reminder_ref)) {
                                Ok(value) => value,
                                Err(error) => { ctx.state.fail("Schedule failed", error); return; }
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
                            ctx.run(task_id.clone(), input, reschedule);
                            ctx.modal.set(None);
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
                                <button class="btn btn-subtle" type="button" on:click=move |_| ctx.modal.set(None)>"Cancel"</button>
                            </div>
                        </form>
                    </div>
                </div>
            }
        })
    }
}

// --- ICS export modal ------------------------------------------------------

#[component]
fn IcsModal(ctx: Sched) -> impl IntoView {
    move || {
        ctx.ics.get().map(|text| {
            let copy_text = text.clone();
            let download_text_value = text.clone();
            view! {
                <div class="sched-modal-layer">
                    <div class="sched-modal-backdrop" on:click=move |_| ctx.ics.set(None)></div>
                    <div class="sched-modal sched-modal-ics">
                        <header class="sched-modal-head">
                            <h2>"Calendar export (ICS)"</h2>
                            <button class="icon-btn" type="button" on:click=move |_| ctx.ics.set(None)>"✕"</button>
                        </header>
                        <p class="sched-modal-hint">"Copy this into any calendar app, or download it as an .ics file. Canceled and skipped blocks are omitted."</p>
                        <textarea class="sched-ics-text" readonly>{text.clone()}</textarea>
                        <div class="sched-modal-actions">
                            <button class="btn btn-primary" type="button" on:click=move |_| copy_to_clipboard(ctx.state, copy_text.clone())>"Copy ICS"</button>
                            <button class="btn btn-ghost" type="button" on:click=move |_| {
                                match download_text("openmgmt-schedule.ics", &download_text_value) {
                                    Ok(_) => ctx.state.notice.set(Some("ICS downloaded.".into())),
                                    Err(error) => ctx.state.fail("Download failed", error),
                                }
                            }>"Download .ics"</button>
                            <button class="btn btn-subtle" type="button" on:click=move |_| ctx.ics.set(None)>"Close"</button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}
