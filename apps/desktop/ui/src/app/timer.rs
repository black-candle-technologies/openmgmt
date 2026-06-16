//! Timer session controls and live elapsed display.
//!
//! `TimerControls` is the single, reusable cluster of start / pause / resume /
//! stop / complete buttons plus the live "elapsed (limit)" readout. It is used
//! on the Tasks table, the Daily Operations hero, and the task edit drawer, so
//! the timer behaves identically everywhere.

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{ActiveTimerInfo, TaskStatus, TaskTimerSession};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::EmptyState;
use super::state::*;

/// Live elapsed seconds: the value the backend computed at `queried_at`, plus
/// wall-clock drift while the timer is still running.
pub fn live_elapsed_seconds(
    active: &ActiveTimerInfo,
    queried_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> i64 {
    if active.is_running {
        (active.elapsed_seconds + (now - queried_at).num_seconds()).max(0)
    } else {
        active.elapsed_seconds.max(0)
    }
}

/// `H:MM:SS` once an hour is reached, otherwise `M:SS`.
pub fn format_hms(total: i64) -> String {
    let total = total.max(0);
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Elapsed seconds for a raw timer session (mirrors the core's accounting), used
/// by the drawer panel which fetches sessions directly rather than via a query.
fn session_elapsed(session: &TaskTimerSession, now: DateTime<Utc>) -> i64 {
    let stored = session.duration_seconds.unwrap_or(0).max(0);
    if session.paused_at.is_some() || session.stopped_at.is_some() || session.completed_at.is_some()
    {
        return stored;
    }
    let run_started = session.resumed_at.unwrap_or(session.started_at);
    stored + (now - run_started).num_seconds().max(0)
}

fn session_state_label(session: &TaskTimerSession) -> &'static str {
    if session.completed_at.is_some() {
        "completed"
    } else if session.stopped_at.is_some() {
        "stopped"
    } else if session.paused_at.is_some() {
        "paused"
    } else {
        "running"
    }
}

fn active_info_from_session(session: &TaskTimerSession, now: DateTime<Utc>) -> ActiveTimerInfo {
    ActiveTimerInfo {
        session_id: session.id.clone(),
        started_at: session.started_at,
        paused_at: session.paused_at,
        resumed_at: session.resumed_at,
        elapsed_seconds: session_elapsed(session, now),
        is_running: session.paused_at.is_none(),
    }
}

fn run_timer_command(
    state: AppState,
    command: &'static str,
    task_id: String,
    success: &'static str,
    context: &'static str,
) {
    spawn_local(async move {
        // Timer commands return either a session or a task; we only need the
        // success/failure signal, so deserialize into the permissive Value.
        let result = invoke::<serde_json::Value>(command, json!({ "task_id": task_id })).await;
        finish_action(state, result, success, context).await;
    });
}

/// Start / pause / resume / stop / complete plus a live elapsed readout.
///
/// `variant` is "compact" (table rows) or "full" (Daily Ops hero). `active` is
/// the task's current timer (when any), and `queried_at` is when that data was
/// fetched, so the readout can tick forward between refreshes.
#[component]
pub fn TimerControls(
    state: AppState,
    now: Signal<DateTime<Utc>>,
    queried_at: Signal<DateTime<Utc>>,
    #[prop(into)] task_id: String,
    status: TaskStatus,
    time_limit_minutes: Option<i32>,
    active: Option<ActiveTimerInfo>,
    #[prop(default = "compact")] variant: &'static str,
) -> impl IntoView {
    let terminal = matches!(status, TaskStatus::Done | TaskStatus::Canceled);
    let is_active = active.is_some();
    let is_running = active
        .as_ref()
        .map(|timer| timer.is_running)
        .unwrap_or(false);
    let active_for_display = active.clone();
    let limit_seconds = time_limit_minutes.map(|minutes| minutes as i64 * 60);

    // Live elapsed + over-limit state, recomputed every clock tick.
    let elapsed = move || {
        active_for_display
            .as_ref()
            .map(|timer| live_elapsed_seconds(timer, queried_at.get(), now.get()))
    };
    let readout = move || match elapsed() {
        Some(seconds) => {
            let over = limit_seconds.is_some_and(|limit| seconds >= limit);
            let label = match time_limit_minutes {
                Some(minutes) => format!("{} / {}m", format_hms(seconds), minutes),
                None => format_hms(seconds),
            };
            let class = if over {
                "timer-elapsed timer-over"
            } else {
                "timer-elapsed"
            };
            Some(view! { <span class=class title="Elapsed (limit)">{label}</span> })
        }
        None => None,
    };

    // Button cluster, derived once from the snapshot's timer state.
    let start_id = task_id.clone();
    let pause_id = task_id.clone();
    let resume_id = task_id.clone();
    let stop_id = task_id.clone();
    let complete_id = task_id.clone();

    view! {
        <div class=format!("timer-controls timer-controls-{variant}")>
            {readout}
            {(!is_active && !terminal).then(|| view! {
                <button class="btn btn-subtle timer-btn" title="Start timer" on:click=move |_| {
                    run_timer_command(state, "start_task_timer", start_id.clone(), "Timer started.", "Start timer failed");
                }>"Start"</button>
            })}
            {(is_active && is_running).then(|| view! {
                <button class="btn btn-subtle timer-btn" title="Pause timer" on:click=move |_| {
                    run_timer_command(state, "pause_task_timer", pause_id.clone(), "Timer paused.", "Pause timer failed");
                }>"Pause"</button>
            })}
            {(is_active && !is_running).then(|| view! {
                <button class="btn btn-subtle timer-btn" title="Resume timer" on:click=move |_| {
                    run_timer_command(state, "resume_task_timer", resume_id.clone(), "Timer resumed.", "Resume timer failed");
                }>"Resume"</button>
            })}
            {(is_active).then(|| view! {
                <button class="btn btn-ghost timer-btn" title="Stop timer" on:click=move |_| {
                    run_timer_command(state, "stop_task_timer", stop_id.clone(), "Timer stopped.", "Stop timer failed");
                }>"Stop"</button>
            })}
            {(!terminal).then(|| view! {
                <button class="btn btn-primary timer-btn" title="Complete task (stops timer)" on:click=move |_| {
                    run_timer_command(state, "complete_task_with_timer", complete_id.clone(), "Task completed.", "Complete task failed");
                }>"Complete"</button>
            })}
        </div>
    }
}

/// Timer panel for the task edit drawer: live controls plus a recent-sessions
/// history. Self-refreshing — it re-fetches whenever the global snapshot reloads
/// (which timer actions trigger via `finish_action`).
#[component]
pub fn TaskTimerPanel(
    state: AppState,
    now: Signal<DateTime<Utc>>,
    #[prop(into)] task_id: String,
    status: TaskStatus,
    time_limit_minutes: Option<i32>,
) -> impl IntoView {
    let active = RwSignal::new(None::<ActiveTimerInfo>);
    let sessions = RwSignal::new(Vec::<TaskTimerSession>::new());
    let queried_at = RwSignal::new(Utc::now());

    let fetch_id = task_id.clone();
    Effect::new(move |_| {
        // Re-fetch after every snapshot reload (timer actions trigger one).
        let _ = state.synced_at.get();
        let id = fetch_id.clone();
        let fetched_at = Utc::now();
        spawn_local(async move {
            if let Ok(session) = invoke::<Option<TaskTimerSession>>(
                "get_active_timer_session",
                json!({ "task_id": id.clone() }),
            )
            .await
            {
                active.set(
                    session
                        .as_ref()
                        .map(|s| active_info_from_session(s, fetched_at)),
                );
                queried_at.set(fetched_at);
            }
            if let Ok(list) = invoke::<Vec<TaskTimerSession>>(
                "list_task_timer_sessions",
                json!({ "task_id": id }),
            )
            .await
            {
                sessions.set(list);
            }
        });
    });

    let queried_sig = Signal::derive(move || queried_at.get());
    let controls_task_id = task_id.clone();

    view! {
        <div class="timer-panel">
            <div class="timer-panel-head">
                <span class="timer-panel-label">"TIMER"</span>
            </div>
            {move || view! {
                <TimerControls
                    state now queried_at=queried_sig
                    task_id=controls_task_id.clone()
                    status=status
                    time_limit_minutes=time_limit_minutes
                    active=active.get()
                    variant="full"
                />
            }}
            <div class="timer-sessions">
                {move || {
                    let mut list = sessions.get();
                    if list.is_empty() {
                        view! { <EmptyState title="No timer sessions yet" hint="Start the timer to track focused time on this task." /> }.into_any()
                    } else {
                        // Most recent first, capped to keep the drawer calm.
                        list.truncate(6);
                        view! {
                            <ul class="timer-session-list">
                                {list.into_iter().map(|session| {
                                    let when = session.started_at.format("%b %-d, %-I:%M %p").to_string();
                                    let label = session_state_label(&session);
                                    let seconds = session_elapsed(&session, now.get_untracked());
                                    view! {
                                        <li class="timer-session">
                                            <span class=format!("timer-session-state timer-session-{label}")>{label}</span>
                                            <span class="timer-session-when">{when}</span>
                                            <span class="timer-session-dur">{format_hms(seconds)}</span>
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
