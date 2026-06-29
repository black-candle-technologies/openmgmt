//! Scheduling / calendar workspace.
//!
//! A usable planner over the scheduling core Codex added: a Today timeline, a
//! seven-day week agenda, an unscheduled-task queue, an overdue list, and a live
//! conflict report. Tasks can be planned by drag-and-drop (onto an hour slot or a
//! day) or via a lightweight schedule/reschedule modal as a reliable fallback.
//!
//! Drag-and-drop reliability notes:
//! * The native webview file-drop handler is disabled for the main window
//!   (`dragDropEnabled: false` in `tauri.conf.json`) so HTML5 drag events fire.
//! * The drag payload is serialized into `DataTransfer` (custom MIME) so it
//!   survives the native drag round-trip; a Leptos signal is the fallback.
//! * A pointer-based "Move mode" is offered as an explicit, always-available
//!   fallback (handle → click a slot/day/panel) for environments where native
//!   HTML5 drag-and-drop still misbehaves.
//!
//! Scheduled datetimes are stored in UTC; everything here renders and accepts
//! local time via the shared helpers in [`crate::app::state`], matching how the
//! rest of the app bridges the local `datetime-local`/`date`/`time` inputs.

use chrono::{DateTime, Duration, Utc};
use leptos::prelude::*;
use openmgmt_core::{
    BlockEdge, CalendarBlock, RecurrenceRule, ScheduleConflict, ScheduleTaskInput,
    ScheduledBlockCompletion, ScheduledBlockHold, Task, TaskWithContext, block_pixel_layout,
    layout_columns, move_block, pixels_to_minutes, resize_block,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::app::components::*;
use crate::app::state::*;
use crate::app::tags::TagChip;

/// Custom MIME used to carry the full drag payload through `DataTransfer`.
const DRAG_MIME: &str = "application/x-openmgmt-schedule-drag";

/// Pixels per hour on the day timeline. The single source of truth for hour
/// height: it drives the block geometry math here AND is published to CSS as the
/// `--hour-px` custom property on `.sched-timeline`, so the hour rows and the
/// absolutely-positioned blocks always agree.
const HOUR_PX: f64 = 56.0;
/// Floor height for very short blocks so their time + title stay readable.
const MIN_CARD_PX: f64 = 46.0;

/// What a pointer drag on a scheduled block is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DragMode {
    /// Dragging the card body moves the whole block (duration preserved).
    Move,
    /// Dragging the top edge changes the start time only.
    ResizeTop,
    /// Dragging the bottom edge changes the end time only.
    ResizeBottom,
}

/// Live state of an in-progress move/resize drag on a scheduled block. One drag
/// is active at a time; the preview times drive the card geometry until commit.
#[derive(Clone)]
struct BlockDrag {
    task_id: String,
    /// `Some` when the task already has a calendar block (so a commit reschedules).
    block_id: Option<String>,
    mode: DragMode,
    orig_start: DateTime<Utc>,
    orig_end: DateTime<Utc>,
    /// Pointer Y at drag start, in client px, to measure the drag delta against.
    start_client_y: f64,
    /// Snapped preview times shown live while dragging; committed on pointer-up.
    preview_start: DateTime<Utc>,
    preview_end: DateTime<Utc>,
    // Metadata preserved across the reschedule the commit performs.
    reminder_at: Option<DateTime<Utc>>,
    deadline_at: Option<DateTime<Utc>>,
    recurrence_rule: Option<RecurrenceRule>,
    recurrence_timezone: Option<String>,
}

impl BlockDrag {
    /// Begin a drag for `task` in `mode` at pointer position `client_y`. Returns
    /// `None` for a task with no start time (nothing to drag).
    fn start(task: &Task, mode: DragMode, client_y: f64) -> Option<Self> {
        let orig_start = task.scheduled_start_at?;
        let orig_end = task
            .scheduled_end_at
            .unwrap_or(orig_start + Duration::hours(1));
        Some(Self {
            task_id: task.id.clone(),
            block_id: task.calendar_block_id.clone(),
            mode,
            orig_start,
            orig_end,
            start_client_y: client_y,
            preview_start: orig_start,
            preview_end: orig_end,
            reminder_at: task.reminder_at,
            deadline_at: task.deadline_at,
            recurrence_rule: task.recurrence_rule,
            recurrence_timezone: task.recurrence_timezone.clone(),
        })
    }

    fn changed(&self) -> bool {
        self.preview_start != self.orig_start || self.preview_end != self.orig_end
    }
}

/// The scheduled block targeted by the "On hold" dialog.
#[derive(Clone)]
struct HoldTarget {
    task: Task,
    block_id: String,
}

/// Capture the pointer on the dragged element so move/up events keep firing even
/// if the pointer leaves it (or the window) mid-drag.
fn capture_pointer(ev: &web_sys::PointerEvent) {
    if let Some(target) = ev.current_target()
        && let Ok(element) = target.dyn_into::<web_sys::Element>()
    {
        let _ = element.set_pointer_capture(ev.pointer_id());
    }
}

/// Whether a pointerdown landed on an interactive child (button/link/field or a
/// resize edge), so a card-body move drag must not start and steal the click.
fn target_is_interactive(ev: &web_sys::PointerEvent) -> bool {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|element| {
            element
                .closest("button, a, input, select, textarea, .sched-resize")
                .ok()
                .flatten()
        })
        .is_some()
}

/// Recurrence choices offered in the schedule modal.
const RECURRENCE_OPTIONS: [(RecurrenceRule, &str); 5] = [
    (RecurrenceRule::None, "Does not repeat"),
    (RecurrenceRule::Daily, "Daily"),
    (RecurrenceRule::Weekdays, "Weekdays (Mon–Fri)"),
    (RecurrenceRule::Weekly, "Weekly"),
    (RecurrenceRule::Monthly, "Monthly"),
];

/// Debug-only console trace (silent in release builds).
fn trace(message: impl AsRef<str>) {
    if cfg!(debug_assertions) {
        web_sys::console::log_1(&JsValue::from_str(message.as_ref()));
    }
}

/// All schedule data the page renders, loaded together so the views stay in sync.
#[derive(Clone, Default)]
struct ScheduleData {
    /// Scheduled tasks for the *selected* day (not necessarily real today).
    day: Vec<TaskWithContext>,
    /// Scheduled tasks for the week containing the selected day.
    week: Vec<TaskWithContext>,
    unscheduled: Vec<TaskWithContext>,
    overdue: Vec<TaskWithContext>,
    conflicts: Vec<ScheduleConflict>,
}

/// The payload moved while a task is being dragged (or held in "Move mode").
///
/// It is `Serialize`/`Deserialize` so it can ride through the native drag via
/// `DataTransfer` JSON rather than relying solely on an in-memory signal. It
/// carries the existing reminder/deadline/recurrence so a drag-reschedule
/// preserves that metadata (the backend overwrites those fields from the input).
#[derive(Clone, Serialize, Deserialize)]
struct DragData {
    task_id: String,
    title: String,
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
    /// True when this column is the real wall-clock today.
    is_today: bool,
    /// True when this column is the user's currently selected schedule day.
    is_selected: bool,
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
    /// Native-drag payload fallback (when `DataTransfer` JSON is unavailable).
    drag: RwSignal<Option<DragData>>,
    /// Pointer-based "Move mode" payload: set on handle click, placed on target click.
    moving: RwSignal<Option<DragData>>,
    active_zone: RwSignal<Option<String>>,
    modal: RwSignal<Option<ScheduleTarget>>,
    /// In-progress move/resize of a timeline block (drives the live preview).
    block_drag: RwSignal<Option<BlockDrag>>,
    /// The block whose "On hold" dialog is open, if any.
    hold: RwSignal<Option<HoldTarget>>,
    ics: RwSignal<Option<String>>,
    now: Signal<DateTime<Utc>>,
    /// The local day the timeline + week view are focused on, as `(year, month, day)`.
    selected_day: RwSignal<(i32, u32, u32)>,
}

impl Sched {
    /// Reload every schedule slice, guarding against overlapping refreshes with a
    /// generation token (latest request wins) the same way the other pages do.
    fn reload(self) {
        let token = self.generation.get_value().wrapping_add(1);
        self.generation.set_value(token);
        self.loading.set(true);
        let day = self.selected_day.get_untracked();
        spawn_local(async move {
            let result = load_schedule_data(day).await;
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

    /// Focus the timeline + week view on a specific local day and reload its data.
    fn select_day(self, day: (i32, u32, u32)) {
        if self.selected_day.get_untracked() != day {
            self.selected_day.set(day);
            self.reload();
        }
    }

    /// Step the selected day by `delta` days (negative = earlier).
    fn shift_day(self, delta: i32) {
        let (y, mo, d) = self.selected_day.get_untracked();
        self.select_day(shift_local_day(y, mo, d, delta));
    }

    /// Jump the selection back to the real wall-clock today.
    fn select_today(self) {
        self.select_day(local_ymd(Utc::now()));
    }

    /// Schedule or reschedule a task, then refresh and surface a conflict warning
    /// if the resulting block overlaps another planned block.
    fn run(self, task_id: String, input: ScheduleTaskInput, reschedule: bool) {
        self.run_with_success(task_id, input, reschedule, None);
    }

    fn run_with_success(
        self,
        task_id: String,
        input: ScheduleTaskInput,
        reschedule: bool,
        on_success: Option<Callback<()>>,
    ) {
        spawn_local(async move {
            let command = if reschedule {
                "reschedule_task"
            } else {
                "schedule_task"
            };
            match invoke_schedule::<CalendarBlock>(
                command,
                json!({ "taskId": task_id, "input": input }),
            )
            .await
            {
                Ok(block) => {
                    trace(format!("[sched] {command} ok block={}", block.id));
                    self.state.notice.set(Some(
                        if reschedule {
                            "Task rescheduled."
                        } else {
                            "Task scheduled."
                        }
                        .to_string(),
                    ));
                    self.state.refresh();
                    if let Some(on_success) = on_success {
                        on_success.run(());
                    }
                    match invoke_schedule::<Vec<ScheduleConflict>>(
                        "list_schedule_conflicts",
                        json!({}),
                    )
                    .await
                    {
                        Ok(conflicts)
                            if conflicts
                                .iter()
                                .any(|c| c.first.id == block.id || c.second.id == block.id) =>
                        {
                            self.state.notice.set(Some(
                                "Scheduled — heads up: this overlaps another block. See Conflicts below."
                                    .to_string(),
                            ));
                        }
                        Ok(_) => {}
                        Err(error) => self.state.fail("Conflict refresh failed", error),
                    }
                }
                Err(error) => {
                    trace(format!("[sched] {command} failed: {error}"));
                    self.state.fail(
                        if reschedule {
                            "Reschedule failed"
                        } else {
                            "Schedule failed"
                        },
                        error,
                    )
                }
            }
        });
    }

    fn clear(self, task_id: String) {
        spawn_local(async move {
            match invoke_schedule::<Task>("clear_task_schedule", json!({ "taskId": task_id })).await
            {
                Ok(_) => {
                    self.state.notice.set(Some("Schedule cleared.".to_string()));
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Clear schedule failed", error),
            }
        });
    }

    fn complete_block(self, block_id: String) {
        spawn_local(async move {
            match invoke_schedule::<ScheduledBlockCompletion>(
                "complete_scheduled_block",
                json!({ "blockId": block_id }),
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
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Complete block failed", error),
            }
        });
    }

    /// Update the live drag preview from the current pointer Y. Shared by move and
    /// both resize edges so preview and commit always use identical math.
    fn block_drag_update(self, client_y: f64) {
        let Some(drag) = self.block_drag.get_untracked() else {
            return;
        };
        let delta = pixels_to_minutes(client_y - drag.start_client_y, HOUR_PX);
        let (preview_start, preview_end) = match drag.mode {
            DragMode::Move => move_block(drag.orig_start, drag.orig_end, delta),
            DragMode::ResizeTop => {
                resize_block(drag.orig_start, drag.orig_end, BlockEdge::Start, delta)
            }
            DragMode::ResizeBottom => {
                resize_block(drag.orig_start, drag.orig_end, BlockEdge::End, delta)
            }
        };
        self.block_drag.set(Some(BlockDrag {
            preview_start,
            preview_end,
            ..drag
        }));
    }

    /// Commit the drag once on pointer-up: reschedule only if the time changed,
    /// then clear the drag state. Idempotent — a second call is a no-op.
    fn block_drag_commit(self) {
        let Some(drag) = self.block_drag.get_untracked() else {
            return;
        };
        self.block_drag.set(None);
        if !drag.changed() {
            return;
        }
        let input = ScheduleTaskInput {
            start_at: drag.preview_start,
            end_at: drag.preview_end,
            timezone: None,
            reminder_at: drag.reminder_at,
            deadline_at: drag.deadline_at,
            recurrence_rule: drag.recurrence_rule,
            recurrence_anchor_at: None,
            recurrence_timezone: drag.recurrence_timezone.clone(),
        };
        self.run(drag.task_id.clone(), input, drag.block_id.is_some());
    }

    /// Abandon an in-progress drag without committing (pointer canceled/lost).
    fn block_drag_cancel(self) {
        self.block_drag.set(None);
    }

    fn skip_block(self, block_id: String) {
        spawn_local(async move {
            match invoke_schedule::<CalendarBlock>(
                "skip_scheduled_block",
                json!({ "blockId": block_id }),
            )
            .await
            {
                Ok(_) => {
                    self.state.notice.set(Some("Block skipped.".to_string()));
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Skip block failed", error),
            }
        });
    }

    /// Put a scheduled block on hold (keeping the task open), optionally with a
    /// continuation block. Args are camelCase to match the Tauri command.
    fn hold_block(self, block_id: String, continuation: Option<ScheduleTaskInput>) {
        spawn_local(async move {
            match invoke_schedule::<ScheduledBlockHold>(
                "hold_scheduled_block",
                json!({ "blockId": block_id, "continuation": continuation }),
            )
            .await
            {
                Ok(result) => {
                    self.state.notice.set(Some(
                        if result.continuation_block.is_some() {
                            "Task put on hold — continuation scheduled."
                        } else {
                            "Task put on hold."
                        }
                        .to_string(),
                    ));
                    self.hold.set(None);
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Hold failed", error),
            }
        });
    }

    fn complete_task(self, task_id: String) {
        spawn_local(async move {
            match invoke_schedule::<Task>("complete_task", json!({ "id": task_id })).await {
                Ok(_) => {
                    self.state.notice.set(Some("Task completed.".to_string()));
                    self.state.refresh();
                }
                Err(error) => self.state.fail("Complete task failed", error),
            }
        });
    }

    /// Shared scheduling core for both native drop and pointer placement: build a
    /// schedule input from a payload + local date/hour and invoke the backend.
    fn schedule_drag(self, drag: DragData, y: i32, mo: u32, d: u32, hour: u32) {
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

    fn unschedule_drag(self, drag: DragData) {
        if drag.block_id.is_some() {
            self.clear(drag.task_id.clone());
        } else {
            self.state
                .notice
                .set(Some("That task is already unscheduled.".to_string()));
        }
    }

    // --- native drag/drop entry points ---

    /// Handle a native drop onto a date + whole-hour slot.
    fn drop_at(self, ev: &web_sys::DragEvent, y: i32, mo: u32, d: u32, hour: u32) {
        self.active_zone.set(None);
        match read_drop_payload(ev, self) {
            Some(drag) => {
                trace(format!("[sched] drop slot {y:04}-{mo:02}-{d:02} {hour}:00"));
                self.drag.set(None);
                self.schedule_drag(drag, y, mo, d, hour);
            }
            None => {
                self.drag.set(None);
                self.state
                    .fail("Drop failed", "no task payload found.".to_string());
            }
        }
    }

    /// Handle a native drop onto the unschedule zone.
    fn drop_unschedule(self, ev: &web_sys::DragEvent) {
        self.active_zone.set(None);
        match read_drop_payload(ev, self) {
            Some(drag) => {
                trace("[sched] drop unschedule");
                self.drag.set(None);
                self.unschedule_drag(drag);
            }
            None => {
                self.drag.set(None);
                self.state
                    .fail("Drop failed", "no task payload found.".to_string());
            }
        }
    }

    // --- pointer "Move mode" entry points ---

    fn start_move(self, drag: DragData) {
        trace(format!("[sched] move start task={}", drag.task_id));
        self.moving.set(Some(drag));
    }

    fn cancel_move(self) {
        self.moving.set(None);
    }

    fn place_at(self, y: i32, mo: u32, d: u32, hour: u32) {
        let Some(drag) = self.moving.get_untracked() else {
            return;
        };
        self.moving.set(None);
        trace(format!(
            "[sched] place slot {y:04}-{mo:02}-{d:02} {hour}:00"
        ));
        self.schedule_drag(drag, y, mo, d, hour);
    }

    fn place_unschedule(self) {
        let Some(drag) = self.moving.get_untracked() else {
            return;
        };
        self.moving.set(None);
        self.unschedule_drag(drag);
    }
}

async fn load_schedule_data(day: (i32, u32, u32)) -> Result<ScheduleData, String> {
    let (day_start, day_end) = local_day_window(day)?;
    let (week_start, week_end) = local_week_window(day)?;
    Ok(ScheduleData {
        day: invoke_schedule(
            "get_schedule_for_day",
            json!({ "start": day_start, "end": day_end }),
        )
        .await?,
        week: invoke_schedule(
            "get_schedule_for_day",
            json!({ "start": week_start, "end": week_end }),
        )
        .await?,
        unscheduled: invoke_schedule("get_unscheduled_tasks", json!({})).await?,
        overdue: invoke_schedule("get_overdue_tasks", json!({})).await?,
        conflicts: invoke_schedule("list_schedule_conflicts", json!({})).await?,
    })
}

async fn invoke_schedule<T: DeserializeOwned>(
    command: &str,
    args: serde_json::Value,
) -> Result<T, String> {
    invoke(command, args)
        .await
        .map_err(|error| format!("{command}: {error}"))
}

// --- drag payload plumbing -------------------------------------------------

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
        title: task.title.clone(),
        block_id: task.calendar_block_id.clone(),
        duration_minutes: duration,
        reminder_at: task.reminder_at,
        deadline_at: task.deadline_at,
        recurrence_rule: task.recurrence_rule,
        recurrence_timezone: task.recurrence_timezone.clone(),
    }
}

/// Start a native drag: stash the payload both as `DataTransfer` JSON (primary)
/// and in the signal (fallback), and set the move drop-effect.
fn begin_drag(ev: &web_sys::DragEvent, ctx: Sched, data: DragData) {
    ctx.drag.set(Some(data.clone()));
    if let Some(transfer) = ev.data_transfer() {
        match serde_json::to_string(&data) {
            Ok(payload) => {
                let _ = transfer.set_data(DRAG_MIME, &payload);
            }
            Err(error) => trace(format!("[sched] serialize failed: {error}")),
        }
        let _ = transfer.set_data("text/plain", &data.title);
        transfer.set_effect_allowed("move");
    }
    trace(format!("[sched] dragstart task={}", data.task_id));
}

/// Read the drop payload: prefer the serialized `DataTransfer` JSON, fall back to
/// the in-memory signal. Returns `None` only when neither is present.
fn read_drop_payload(ev: &web_sys::DragEvent, ctx: Sched) -> Option<DragData> {
    if let Some(transfer) = ev.data_transfer()
        && let Ok(payload) = transfer.get_data(DRAG_MIME)
        && !payload.is_empty()
    {
        match serde_json::from_str::<DragData>(&payload) {
            Ok(data) => {
                trace("[sched] payload from DataTransfer");
                return Some(data);
            }
            Err(error) => trace(format!("[sched] payload parse failed: {error}")),
        }
    }
    let fallback = ctx.drag.get_untracked();
    if fallback.is_some() {
        trace("[sched] payload from signal fallback");
    }
    fallback
}

fn allow_drop(ev: &web_sys::DragEvent) {
    ev.prevent_default();
    if let Some(transfer) = ev.data_transfer() {
        transfer.set_drop_effect("move");
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

// --- small helpers ---------------------------------------------------------

/// Build a UTC instant from a local date + whole hour by reusing the tested
/// `datetime-local` → UTC bridge (`combine_local` lives in [`crate::app::state`]).
fn local_to_utc(y: i32, mo: u32, d: u32, hour: u32) -> Result<DateTime<Utc>, String> {
    combine_local(&format!("{y:04}-{mo:02}-{d:02}"), &format!("{hour:02}:00"))
}

/// Local `(year, month, day)` shifted by `delta` days, handling month/year
/// rollover via the browser `Date` (which normalises out-of-range day numbers).
fn shift_local_day(y: i32, mo: u32, d: u32, delta: i32) -> (i32, u32, u32) {
    let date = js_sys::Date::new_with_year_month_day(y as u32, mo as i32 - 1, d as i32 + delta);
    (
        date.get_full_year() as i32,
        date.get_month() + 1,
        date.get_date(),
    )
}

/// Local day-of-week (0 = Sunday … 6 = Saturday) for a local `(y, mo, d)`.
fn local_weekday_of(y: i32, mo: u32, d: u32) -> u32 {
    js_sys::Date::new_with_year_month_day(y as u32, mo as i32 - 1, d as i32).get_day()
}

/// Monday of the week containing the given local day.
fn week_monday(y: i32, mo: u32, d: u32) -> (i32, u32, u32) {
    let weekday = local_weekday_of(y, mo, d); // 0 = Sunday
    let offset = if weekday == 0 { 6 } else { weekday - 1 } as i32;
    shift_local_day(y, mo, d, -offset)
}

/// UTC `[start, end)` window covering the local day (local midnight → next midnight).
fn local_day_window(day: (i32, u32, u32)) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let (y, mo, d) = day;
    let start = local_to_utc(y, mo, d, 0)?;
    let (ny, nmo, nd) = shift_local_day(y, mo, d, 1);
    let end = local_to_utc(ny, nmo, nd, 0)?;
    Ok((start, end))
}

/// UTC `[start, end)` window covering the Monday-based week containing `day`.
fn local_week_window(day: (i32, u32, u32)) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let (my, mmo, md) = week_monday(day.0, day.1, day.2);
    let start = local_to_utc(my, mmo, md, 0)?;
    let (ey, emo, ed) = shift_local_day(my, mmo, md, 7);
    let end = local_to_utc(ey, emo, ed, 0)?;
    Ok((start, end))
}

/// Parse a `<input type="date">` value (`YYYY-MM-DD`) into a local `(y, mo, d)`.
fn parse_local_date(value: &str) -> Option<(i32, u32, u32)> {
    let mut parts = value.split('-');
    let y = parts.next()?.parse().ok()?;
    let mo = parts.next()?.parse().ok()?;
    let d = parts.next()?.parse().ok()?;
    Some((y, mo, d))
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

/// Seven local day columns for the Monday-based week containing `selected`, each
/// flagged for the real wall-clock today and for the current selection.
fn week_columns_for(selected: (i32, u32, u32), real_today: (i32, u32, u32)) -> Vec<DayColumn> {
    let (my, mmo, md) = week_monday(selected.0, selected.1, selected.2);
    (0..7)
        .map(|i| {
            let (y, mo, d) = shift_local_day(my, mmo, md, i);
            DayColumn {
                y,
                mo,
                d,
                weekday: weekday_short(local_weekday_of(y, mo, d)),
                is_today: (y, mo, d) == real_today,
                is_selected: (y, mo, d) == selected,
            }
        })
        .collect()
}

/// Friendly label for the selected day, e.g. `Thursday, June 18` (or `Today` when
/// the selection matches the real wall-clock day).
fn selected_day_label(selected: (i32, u32, u32), real_today: (i32, u32, u32)) -> String {
    let (y, mo, d) = selected;
    let weekday = weekday_full(local_weekday_of(y, mo, d));
    let date = format!("{}, {} {}", weekday, month_full(mo), d);
    if selected == real_today {
        format!("Today · {date}")
    } else {
        date
    }
}

/// Full weekday name for a 0-based (Sunday) day-of-week.
fn weekday_full(weekday: u32) -> &'static str {
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    DAYS.get(weekday as usize).copied().unwrap_or("")
}

/// Full month name for a 1-based month number.
fn month_full(month: u32) -> &'static str {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    MONTHS
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("")
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
        moving: RwSignal::new(None),
        active_zone: RwSignal::new(None),
        modal: RwSignal::new(None),
        block_drag: RwSignal::new(None),
        hold: RwSignal::new(None),
        ics: RwSignal::new(None),
        now: now.into(),
        selected_day: RwSignal::new(local_ymd(Utc::now())),
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
            match invoke_schedule::<String>("generate_schedule_ics", json!({})).await {
                Ok(text) => ctx.ics.set(Some(text)),
                Err(error) => ctx.state.fail("Export ICS failed", error),
            }
        });
    });

    view! {
        <div class="sched-root" class:sched-is-moving=move || ctx.moving.get().is_some()>
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

            <div class="sched-daynav">
                <div class="sched-daynav-controls">
                    <button class="btn btn-subtle" type="button" title="Previous day" on:click=move |_| ctx.shift_day(-1)>"‹ Prev"</button>
                    <button
                        class="btn btn-subtle"
                        class:active=move || ctx.selected_day.get() == local_ymd(Utc::now())
                        type="button"
                        title="Jump to today"
                        on:click=move |_| ctx.select_today()
                    >"Today"</button>
                    <button class="btn btn-subtle" type="button" title="Next day" on:click=move |_| ctx.shift_day(1)>"Next ›"</button>
                    <input
                        class="sched-daynav-date"
                        type="date"
                        prop:value=move || { let (y, mo, d) = ctx.selected_day.get(); format!("{y:04}-{mo:02}-{d:02}") }
                        on:change=move |ev| {
                            if let Some(day) = parse_local_date(&event_target_value(&ev)) {
                                ctx.select_day(day);
                            }
                        }
                    />
                </div>
                <span class="sched-daynav-label">
                    {move || selected_day_label(ctx.selected_day.get(), local_ymd(Utc::now()))}
                </span>
            </div>

            <p class="sched-explainer">
                "Scheduled tasks move to NOW during their time block, Later Today before their block, and Overdue if the block passes unfinished. Tasks auto-start when their scheduled time arrives while the app is open."
            </p>

            {move || ctx.moving.get().map(|drag| view! {
                <div class="banner sched-move-banner">
                    <span>{format!("Moving “{}” — click a time slot or day to place it, or the Unscheduled panel to clear.", drag.title)}</span>
                    <button class="banner-dismiss" type="button" on:click=move |_| ctx.cancel_move()>"Cancel"</button>
                </div>
            })}

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
            <HoldModal ctx />
            <IcsModal ctx />
        </div>
    }
}

/// Small grab handle that both seeds the native drag and starts pointer "Move
/// mode" on click, so dragging *and* click-to-place both work from one control.
#[component]
fn DragHandle(ctx: Sched, task: Task) -> impl IntoView {
    let move_data = drag_from_task(&task);
    view! {
        <button
            class="sched-drag-handle"
            type="button"
            title="Drag to move, or click then pick a slot"
            aria-label="Move task"
            on:click=move |ev| { ev.stop_propagation(); ctx.start_move(move_data.clone()); }
        >"⠿"</button>
    }
}

// --- Today timeline --------------------------------------------------------

#[component]
fn TodayTimeline(ctx: Sched) -> impl IntoView {
    // Heading flips between "Today" and the selected weekday so the main timeline
    // is never mislabelled when the user has navigated away from the real today.
    let heading = move || {
        let selected = ctx.selected_day.get();
        if selected == local_ymd(Utc::now()) {
            "Today".to_string()
        } else {
            weekday_full(local_weekday_of(selected.0, selected.1, selected.2)).to_string()
        }
    };
    view! {
        <section class="panel sched-timeline-panel">
            <div class="section-head">
                <div class="section-head-title">
                    <h2>{heading}</h2>
                    <span class="count-chip">{move || ctx.data.get().day.len()}</span>
                </div>
                <span class="sched-today-date">
                    {move || { let (_, mo, d) = ctx.selected_day.get(); format!("{} {}", month_short(mo), d) }}
                </span>
            </div>
            {move || {
                let day = ctx.data.get().day;
                if day.is_empty() && ctx.loading.get() {
                    return view! { <LoadingState label="Loading schedule…" /> }.into_any();
                }
                let (ty, tmo, td) = ctx.selected_day.get();

                // Core working window 8 AM–8 PM, widened to include any block that
                // falls outside it so nothing is ever hidden.
                let mut min_h = 8u32;
                let mut max_h = 20u32;
                for row in &day {
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
                let empty = day.is_empty();
                let day_start_hour = min_h;
                let timeline_px = hours.len() as f64 * HOUR_PX;

                // Lay every scheduled block out absolutely so it spans its real
                // duration, packing overlaps into side-by-side columns.
                let blocks: Vec<TaskWithContext> = day
                    .iter()
                    .filter(|row| row.task.scheduled_start_at.is_some())
                    .cloned()
                    .collect();
                let intervals: Vec<(i64, i64)> = blocks
                    .iter()
                    .map(|row| {
                        let s =
                            local_minutes_of_day(row.task.scheduled_start_at.expect("filtered above"));
                        let e = row
                            .task
                            .scheduled_end_at
                            .map(local_minutes_of_day)
                            .unwrap_or(s + 60)
                            .max(s + 15);
                        (s, e)
                    })
                    .collect();
                let positioned: Vec<(TaskWithContext, usize, usize)> = blocks
                    .into_iter()
                    .zip(layout_columns(&intervals))
                    .map(|(row, (col, cols))| (row, col, cols))
                    .collect();

                view! {
                    {empty.then(|| view! {
                        <p class="sched-empty-hint">"Nothing scheduled for this day. Drag an unscheduled task here to plan it, then drag a block's top or bottom edge to change its time."</p>
                    })}
                    <div
                        class="sched-timeline"
                        class:is-dragging=move || ctx.block_drag.get().is_some()
                        style=format!("height:{timeline_px}px;--hour-px:{HOUR_PX}px")
                    >
                        <div class="sched-hours">
                            {hours.into_iter().map(|h| view! {
                                <div
                                    class="sched-slot"
                                    class:drop-active=move || ctx.active_zone.get().as_deref() == Some(format!("today-{h}").as_str())
                                    on:dragenter=move |ev| { ev.prevent_default(); ctx.active_zone.set(Some(format!("today-{h}"))); }
                                    on:dragover=move |ev| {
                                        allow_drop(&ev);
                                        let id = format!("today-{h}");
                                        if ctx.active_zone.get_untracked().as_deref() != Some(id.as_str()) {
                                            ctx.active_zone.set(Some(id));
                                        }
                                    }
                                    on:dragleave=move |_| {
                                        let id = format!("today-{h}");
                                        if ctx.active_zone.get_untracked().as_deref() == Some(id.as_str()) {
                                            ctx.active_zone.set(None);
                                        }
                                    }
                                    on:drop=move |ev| { ev.prevent_default(); ev.stop_propagation(); ctx.drop_at(&ev, ty, tmo, td, h); }
                                >
                                    <span class="sched-slot-label">{hour_label(h)}</span>
                                    <span class="sched-slot-cell"></span>
                                    <button
                                        class="sched-move-target"
                                        type="button"
                                        aria-label=move || format!("Place task at {}", hour_label(h))
                                        on:click=move |_| ctx.place_at(ty, tmo, td, h)
                                    ></button>
                                </div>
                            }).collect_view()}
                        </div>
                        <div class="sched-blocks">
                            {positioned.into_iter().map(|(row, col, cols)| view! {
                                <ScheduledCard ctx row day_start_hour=day_start_hour column=col columns=cols />
                            }).collect_view()}
                        </div>
                    </div>
                }.into_any()
            }}
        </section>
    }
}

#[component]
fn ScheduledCard(
    ctx: Sched,
    row: TaskWithContext,
    /// First hour shown on the timeline, so the block can offset from the top.
    day_start_hour: u32,
    /// This block's column and the column count of its overlap cluster, for
    /// laying overlapping blocks out side by side.
    column: usize,
    columns: usize,
) -> impl IntoView {
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
    let block_id = task.calendar_block_id.clone();

    let title_task = task.clone();
    let resched_task = task.clone();
    let hold_task = task.clone();
    let top_task = task.clone();
    let bottom_task = task.clone();
    let move_task = task.clone();
    let clear_id = task.id.clone();
    let id_for_class = task.id.clone();
    let id_for_style = task.id.clone();
    let id_for_label = task.id.clone();

    // Effective times: the live move/resize preview while this block is being
    // dragged, otherwise its stored times. Shared by the geometry and the label.
    let effective_times = move |task_id: &str| -> (DateTime<Utc>, DateTime<Utc>) {
        if let Some(drag) = ctx.block_drag.get()
            && drag.task_id == task_id
        {
            return (drag.preview_start, drag.preview_end);
        }
        let s = start.unwrap_or_else(Utc::now);
        let e = end.unwrap_or(s + Duration::hours(1));
        (s, e)
    };

    let card_class = move || {
        let dragging = ctx
            .block_drag
            .get()
            .is_some_and(|drag| drag.task_id == id_for_class);
        format!(
            "sched-card sched-block sched-card-{}{}",
            block_state(start, end, ctx.now.get()),
            if dragging { " is-dragging" } else { "" }
        )
    };
    let card_style = move || {
        let (s, e) = effective_times(&id_for_style);
        let start_min = local_minutes_of_day(s);
        let end_min = local_minutes_of_day(e).max(start_min + 15);
        let (top, height) = block_pixel_layout(
            start_min,
            end_min,
            day_start_hour as i64,
            HOUR_PX,
            MIN_CARD_PX,
        );
        let count = columns.max(1) as f64;
        let left = column as f64 / count * 100.0;
        let width = 100.0 / count;
        format!("top:{top}px;height:{height}px;left:calc({left}% + 2px);width:calc({width}% - 6px)")
    };
    let time_label = move || {
        let (s, e) = effective_times(&id_for_label);
        fmt_time_range(s, e)
    };

    view! {
        // The card body is the move grip: pointerdown starts a Move drag unless it
        // lands on a button/title/resize edge (then that control handles it). The
        // edges are separate ResizeEdge targets. All share one captured pointer so
        // move and resize can never run at once.
        <article
            class=card_class
            style=card_style
            on:pointerdown=move |ev: web_sys::PointerEvent| {
                if ev.button() != 0 || target_is_interactive(&ev) { return; }
                ev.prevent_default();
                capture_pointer(&ev);
                if let Some(drag) = BlockDrag::start(&move_task, DragMode::Move, f64::from(ev.client_y())) {
                    ctx.block_drag.set(Some(drag));
                }
            }
            on:pointermove=move |ev: web_sys::PointerEvent| ctx.block_drag_update(f64::from(ev.client_y()))
            on:pointerup=move |_| ctx.block_drag_commit()
            on:pointercancel=move |_| ctx.block_drag_cancel()
        >
            <ResizeEdge ctx task=top_task mode=DragMode::ResizeTop class="sched-resize-top" />
            <div class="sched-card-head">
                <PriorityBadge value=priority />
                <span class="sched-card-time">{time_label}</span>
                {recurrence.map(|rule| view! { <span class="sched-recur" title="Repeats">{"↻ "}{recurrence_label(rule)}</span> })}
                <span class="sched-grip" aria-hidden="true" title="Drag the card to move; drag an edge to resize">"⠿"</span>
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
                    let hold_id = id.clone();
                    let skip_id = id;
                    let hold_task = hold_task.clone();
                    view! {
                        <button class="btn btn-primary sched-mini" type="button" on:click=move |_| ctx.complete_block(complete_id.clone())>"Complete"</button>
                        <button class="btn btn-subtle sched-mini" type="button" title="Pause this session and keep the task open" on:click=move |_| ctx.hold.set(Some(HoldTarget { task: hold_task.clone(), block_id: hold_id.clone() }))>"On hold"</button>
                        <button class="btn btn-subtle sched-mini" type="button" on:click=move |_| ctx.skip_block(skip_id.clone())>"Skip"</button>
                    }
                })}
                {block_id.is_some().then(|| view! {
                    <button class="btn btn-danger-soft sched-mini" type="button" on:click=move |_| ctx.clear(clear_id.clone())>"Clear"</button>
                })}
            </div>
            <ResizeEdge ctx task=bottom_task mode=DragMode::ResizeBottom class="sched-resize-bottom" />
        </article>
    }
}

/// A thin drag target on the top or bottom edge of a scheduled block. Dragging it
/// resizes the block (snapped to [`openmgmt_core::SCHEDULE_SNAP_MINUTES`]) with a
/// live preview, committing via `reschedule_task` on release. It stops propagation
/// so an edge drag never also starts a card-body move.
#[component]
fn ResizeEdge(ctx: Sched, task: Task, mode: DragMode, class: &'static str) -> impl IntoView {
    let down_task = task;
    view! {
        <span
            class=format!("sched-resize {class}")
            title="Drag to change the time"
            aria-label="Resize scheduled block"
            on:dragstart=move |ev| { ev.prevent_default(); ev.stop_propagation(); }
            on:pointerdown=move |ev: web_sys::PointerEvent| {
                if ev.button() != 0 { return; }
                ev.prevent_default();
                ev.stop_propagation();
                capture_pointer(&ev);
                if let Some(drag) = BlockDrag::start(&down_task, mode, f64::from(ev.client_y())) {
                    ctx.block_drag.set(Some(drag));
                }
            }
            on:pointermove=move |ev: web_sys::PointerEvent| ctx.block_drag_update(f64::from(ev.client_y()))
            on:pointerup=move |_| ctx.block_drag_commit()
            on:pointercancel=move |_| ctx.block_drag_cancel()
        ></span>
    }
}

// --- Unscheduled -----------------------------------------------------------

#[component]
fn UnscheduledPanel(ctx: Sched) -> impl IntoView {
    view! {
        <section
            class="panel sched-side-panel"
            class:drop-active=move || ctx.active_zone.get().as_deref() == Some("unschedule")
            on:dragenter=move |ev| { ev.prevent_default(); ctx.active_zone.set(Some("unschedule".to_string())); }
            on:dragover=move |ev| {
                allow_drop(&ev);
                if ctx.active_zone.get_untracked().as_deref() != Some("unschedule") {
                    ctx.active_zone.set(Some("unschedule".to_string()));
                }
            }
            on:dragleave=move |_| {
                if ctx.active_zone.get_untracked().as_deref() == Some("unschedule") {
                    ctx.active_zone.set(None);
                }
            }
            on:drop=move |ev| { ev.prevent_default(); ev.stop_propagation(); ctx.drop_unschedule(&ev); }
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
            <button
                class="sched-move-target sched-move-unschedule"
                type="button"
                aria-label="Unschedule the task being moved"
                on:click=move |_| ctx.place_unschedule()
            ></button>
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
    let handle_task = task.clone();
    let title_task = task.clone();
    let schedule_task = task;

    view! {
        <article
            class="sched-card sched-card-queued"
            draggable="true"
            on:dragstart=move |ev| begin_drag(&ev, ctx, drag_from_task(&drag_task))
            on:dragend=move |_| ctx.active_zone.set(None)
        >
            <div class="sched-card-head">
                <PriorityBadge value=priority />
                {estimate.map(|minutes| view! { <span class="sched-card-est">{format!("~{minutes}m")}</span> })}
                <DragHandle ctx task=handle_task />
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
                <span class="sched-week-hint">"Click a day to plan it"</span>
            </div>
            {move || {
                let week = ctx.data.get().week;
                let columns = week_columns_for(ctx.selected_day.get(), local_ymd(Utc::now()));
                view! {
                    <div class="sched-week">
                        {columns.into_iter().map(|col| {
                            let (y, mo, d) = (col.y, col.mo, col.d);
                            let weekday = col.weekday;
                            let is_today = col.is_today;
                            let is_selected = col.is_selected;
                            let mut day_class = String::from("sched-week-day");
                            if is_today { day_class.push_str(" sched-week-today"); }
                            if is_selected { day_class.push_str(" sched-week-selected"); }
                            let zone = format!("week-{y}-{mo}-{d}");
                            let zone_class = zone.clone();
                            let zone_enter = zone.clone();
                            let zone_over = zone.clone();
                            let zone_leave = zone;

                            let mut day_rows: Vec<TaskWithContext> = week
                                .iter()
                                .filter(|row| row.task.scheduled_start_at.map(local_ymd) == Some((y, mo, d)))
                                .cloned()
                                .collect();
                            day_rows.sort_by_key(|row| row.task.scheduled_start_at);

                            view! {
                                <div
                                    class=day_class
                                    class:drop-active=move || ctx.active_zone.get().as_deref() == Some(zone_class.as_str())
                                    on:dragenter=move |ev| { ev.prevent_default(); ctx.active_zone.set(Some(zone_enter.clone())); }
                                    on:dragover=move |ev| {
                                        allow_drop(&ev);
                                        if ctx.active_zone.get_untracked().as_deref() != Some(zone_over.as_str()) {
                                            ctx.active_zone.set(Some(zone_over.clone()));
                                        }
                                    }
                                    on:dragleave=move |_| {
                                        if ctx.active_zone.get_untracked().as_deref() == Some(zone_leave.as_str()) {
                                            ctx.active_zone.set(None);
                                        }
                                    }
                                    on:drop=move |ev| { ev.prevent_default(); ev.stop_propagation(); ctx.drop_at(&ev, y, mo, d, 9); }
                                >
                                    <button
                                        class="sched-week-head"
                                        type="button"
                                        title="Show this day in the timeline"
                                        on:click=move |_| ctx.select_day((y, mo, d))
                                    >
                                        <span class="sched-week-dow">{weekday}</span>
                                        <span class="sched-week-date">{d}</span>
                                    </button>
                                    {if day_rows.is_empty() {
                                        view! { <p class="sched-week-empty">"—"</p> }.into_any()
                                    } else {
                                        view! {
                                            <div class="sched-week-list">
                                                {day_rows.into_iter().map(|row| view! { <WeekLine ctx row /> }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                    <button
                                        class="sched-move-target"
                                        type="button"
                                        aria-label="Place task on this day"
                                        on:click=move |_| ctx.place_at(y, mo, d, 9)
                                    ></button>
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
    let modal_task = task;

    view! {
        <div
            class="sched-week-line"
            draggable="true"
            title="Drag to move · click to reschedule"
            on:dragstart=move |ev| begin_drag(&ev, ctx, drag_from_task(&drag_task))
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
            let recurrence_timezone = task.recurrence_timezone.clone();

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
                                recurrence_timezone: recurrence_timezone.clone(),
                            };
                            let close_modal = Callback::new(move |_| ctx.modal.set(None));
                            ctx.run_with_success(task_id.clone(), input, reschedule, Some(close_modal));
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

// --- On-hold modal ---------------------------------------------------------

#[component]
fn HoldModal(ctx: Sched) -> impl IntoView {
    move || {
        ctx.hold.get().map(|target| {
            let task = target.task.clone();
            let task_title = task.title.clone();
            let block_id_just = target.block_id.clone();
            let block_id_cont = target.block_id.clone();
            let deadline = task.deadline_at;

            // Default the continuation to the same time of day, one day later.
            let default_minutes = task
                .estimated_minutes
                .or(task.time_limit_minutes)
                .map(i64::from)
                .unwrap_or(60)
                .clamp(5, 24 * 60);
            let base_start = task.scheduled_start_at;
            let next_start = base_start
                .map(|start| start + Duration::days(1))
                .unwrap_or_else(|| Utc::now() + Duration::days(1));
            let init_date = local_date_str(next_start);
            let init_start = base_start
                .map(local_time_str)
                .unwrap_or_else(|| "09:00".to_string());
            let init_end = match (base_start, task.scheduled_end_at) {
                (_, Some(end)) => local_time_str(end),
                (Some(start), None) => local_time_str(start + Duration::minutes(default_minutes)),
                _ => "10:00".to_string(),
            };

            let show_continuation = RwSignal::new(false);
            let date_ref = NodeRef::<leptos::html::Input>::new();
            let start_ref = NodeRef::<leptos::html::Input>::new();
            let end_ref = NodeRef::<leptos::html::Input>::new();

            view! {
                <div class="sched-modal-layer">
                    <div class="sched-modal-backdrop" on:click=move |_| ctx.hold.set(None)></div>
                    <div class="sched-modal sched-hold-modal">
                        <header class="sched-modal-head">
                            <h2>"Put this task on hold?"</h2>
                            <button class="icon-btn" type="button" on:click=move |_| ctx.hold.set(None)>"✕"</button>
                        </header>
                        <p class="sched-modal-task">{task_title}</p>
                        <p class="sched-modal-hint">"This keeps the task open and ends this scheduled work session. You can optionally schedule another session to continue it on another day."</p>
                        {move || {
                            // Clone the owned values per render so this reactive
                            // closure stays `FnMut` (the inner handlers move them).
                            let block_id_just = block_id_just.clone();
                            let block_id_cont = block_id_cont.clone();
                            let init_date = init_date.clone();
                            let init_start = init_start.clone();
                            let init_end = init_end.clone();
                            if show_continuation.get() {
                            view! {
                                <form class="sched-modal-form" on:submit=move |event| {
                                    event.prevent_default();
                                    let date = input_value(date_ref);
                                    let start_t = input_value(start_ref);
                                    let end_t = input_value(end_ref);
                                    if date.is_empty() || start_t.is_empty() || end_t.is_empty() {
                                        ctx.state.fail("Hold failed", "Date, start time, and end time are required.".into());
                                        return;
                                    }
                                    let start_at = match combine_local(&date, &start_t) {
                                        Ok(value) => value,
                                        Err(error) => { ctx.state.fail("Hold failed", error); return; }
                                    };
                                    let end_at = match combine_local(&date, &end_t) {
                                        Ok(value) => value,
                                        Err(error) => { ctx.state.fail("Hold failed", error); return; }
                                    };
                                    if end_at <= start_at {
                                        ctx.state.fail("Hold failed", "End time must be after start time.".into());
                                        return;
                                    }
                                    let input = ScheduleTaskInput {
                                        start_at,
                                        end_at,
                                        timezone: None,
                                        reminder_at: None,
                                        deadline_at: deadline,
                                        recurrence_rule: None,
                                        recurrence_anchor_at: None,
                                        recurrence_timezone: None,
                                    };
                                    ctx.hold_block(block_id_cont.clone(), Some(input));
                                }>
                                    <FormField label="Continuation date">
                                        <input node_ref=date_ref type="date" value=init_date.clone() required />
                                    </FormField>
                                    <div class="form-row">
                                        <FormField label="Start">
                                            <input node_ref=start_ref type="time" value=init_start.clone() required />
                                        </FormField>
                                        <FormField label="End">
                                            <input node_ref=end_ref type="time" value=init_end.clone() required />
                                        </FormField>
                                    </div>
                                    <div class="sched-modal-actions">
                                        <button class="btn btn-primary" type="submit">"Schedule continuation"</button>
                                        <button class="btn btn-subtle" type="button" on:click=move |_| show_continuation.set(false)>"Back"</button>
                                        <button class="btn btn-ghost" type="button" on:click=move |_| ctx.hold.set(None)>"Cancel"</button>
                                    </div>
                                </form>
                            }.into_any()
                        } else {
                            view! {
                                <div class="sched-modal-actions sched-hold-choices">
                                    <button class="btn btn-primary" type="button" on:click=move |_| ctx.hold_block(block_id_just.clone(), None)>"Just put on hold"</button>
                                    <button class="btn btn-subtle" type="button" on:click=move |_| show_continuation.set(true)>"Schedule continuation"</button>
                                    <button class="btn btn-ghost" type="button" on:click=move |_| ctx.hold.set(None)>"Cancel"</button>
                                </div>
                            }.into_any()
                            }
                        }}
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
