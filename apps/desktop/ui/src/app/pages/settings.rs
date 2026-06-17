//! Settings: the scoring-weight panel (get / update / reset) and a data section
//! for JSON/CSV exports and a local SQLite backup.

use leptos::prelude::*;
use openmgmt_core::{ScoringSettings, ScoringSettingsPatch};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::app::components::*;
use crate::app::state::*;

/// String mirror of the editable scoring weights, so the inputs stay controlled
/// and a reset/load can repopulate every field at once.
#[derive(Clone, Default)]
struct ScoringForm {
    priority_weight: String,
    pinned_boost: String,
    overdue_boost: String,
    due_soon_boost: String,
    in_progress_boost: String,
    blocked_penalty: String,
    waiting_penalty: String,
    paused_project_penalty: String,
    due_soon_window_hours: String,
}

impl ScoringForm {
    fn from_settings(settings: &ScoringSettings) -> Self {
        Self {
            priority_weight: settings.priority_weight.to_string(),
            pinned_boost: settings.pinned_boost.to_string(),
            overdue_boost: settings.overdue_boost.to_string(),
            due_soon_boost: settings.due_soon_boost.to_string(),
            in_progress_boost: settings.in_progress_boost.to_string(),
            blocked_penalty: settings.blocked_penalty.to_string(),
            waiting_penalty: settings.waiting_penalty.to_string(),
            paused_project_penalty: settings.paused_project_penalty.to_string(),
            due_soon_window_hours: settings.due_soon_window_hours.to_string(),
        }
    }

    fn to_patch(&self) -> Result<ScoringSettingsPatch, String> {
        Ok(ScoringSettingsPatch {
            priority_weight: Some(parse_score("Priority weight", &self.priority_weight)?),
            pinned_boost: Some(parse_score("Pinned boost", &self.pinned_boost)?),
            overdue_boost: Some(parse_score("Overdue boost", &self.overdue_boost)?),
            due_soon_boost: Some(parse_score("Due soon boost", &self.due_soon_boost)?),
            in_progress_boost: Some(parse_score("In-progress boost", &self.in_progress_boost)?),
            blocked_penalty: Some(parse_score("Blocked penalty", &self.blocked_penalty)?),
            waiting_penalty: Some(parse_score("Waiting penalty", &self.waiting_penalty)?),
            paused_project_penalty: Some(parse_score(
                "Paused-project penalty",
                &self.paused_project_penalty,
            )?),
            due_soon_window_hours: Some(parse_score(
                "Due-soon window (hours)",
                &self.due_soon_window_hours,
            )?),
        })
    }
}

fn parse_score(label: &str, value: &str) -> Result<i32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{label} must be a valid whole number."))
}

#[component]
pub fn SettingsPage(state: AppState) -> impl IntoView {
    let form = RwSignal::new(ScoringForm::default());
    let loaded = RwSignal::new(false);

    spawn_local(async move {
        match invoke::<ScoringSettings>("get_scoring_settings", json!({})).await {
            Ok(settings) => {
                form.set(ScoringForm::from_settings(&settings));
                loaded.set(true);
            }
            Err(error) => {
                loaded.set(true);
                state.fail("Load scoring settings failed", error);
            }
        }
    });

    let save = move || {
        let patch = match form.get_untracked().to_patch() {
            Ok(patch) => patch,
            Err(error) => {
                state.fail("Save scoring settings failed", error);
                return;
            }
        };
        spawn_local(async move {
            match invoke::<ScoringSettings>("update_scoring_settings", json!({ "patch": patch }))
                .await
            {
                Ok(settings) => {
                    form.set(ScoringForm::from_settings(&settings));
                    state.notice.set(Some("Scoring settings saved.".into()));
                    state.refresh();
                }
                Err(error) => state.fail("Save scoring settings failed", error),
            }
        });
    };

    let reset = move || {
        if !confirmed("Reset scoring weights to defaults?") {
            return;
        }
        spawn_local(async move {
            match invoke::<ScoringSettings>("reset_scoring_settings", json!({})).await {
                Ok(settings) => {
                    form.set(ScoringForm::from_settings(&settings));
                    state
                        .notice
                        .set(Some("Scoring settings reset to defaults.".into()));
                    state.refresh();
                }
                Err(error) => state.fail("Reset scoring settings failed", error),
            }
        });
    };

    view! {
        <PageHeader
            eyebrow="CONFIGURATION"
            title="Settings"
            description="Tune how work is scored and prioritized, and export or back up your local data."
        />

        <Section title="Scoring weights">
            <p class="settings-note">
                "Urgency is a sum of these weights. Positive numbers push a task up the board; penalties (negative numbers) push it down. Changes apply across the Board, Tasks, and Daily Operations."
            </p>
            {move || if loaded.get() {
                view! {
                    <div class="settings-grid">
                        <ScoreField label="Priority weight" hint="Per priority point (P1–P5)"
                            value=Signal::derive(move || form.get().priority_weight)
                            on_input=Callback::new(move |value| form.update(|f| f.priority_weight = value)) />
                        <ScoreField label="Pinned boost" hint="Added when a task is pinned"
                            value=Signal::derive(move || form.get().pinned_boost)
                            on_input=Callback::new(move |value| form.update(|f| f.pinned_boost = value)) />
                        <ScoreField label="Overdue boost" hint="Base boost once a task is overdue"
                            value=Signal::derive(move || form.get().overdue_boost)
                            on_input=Callback::new(move |value| form.update(|f| f.overdue_boost = value)) />
                        <ScoreField label="Due soon boost" hint="Boost inside the due-soon window"
                            value=Signal::derive(move || form.get().due_soon_boost)
                            on_input=Callback::new(move |value| form.update(|f| f.due_soon_boost = value)) />
                        <ScoreField label="In-progress boost" hint="Added while a task is in progress"
                            value=Signal::derive(move || form.get().in_progress_boost)
                            on_input=Callback::new(move |value| form.update(|f| f.in_progress_boost = value)) />
                        <ScoreField label="Blocked penalty" hint="Usually negative"
                            value=Signal::derive(move || form.get().blocked_penalty)
                            on_input=Callback::new(move |value| form.update(|f| f.blocked_penalty = value)) />
                        <ScoreField label="Waiting penalty" hint="Usually negative"
                            value=Signal::derive(move || form.get().waiting_penalty)
                            on_input=Callback::new(move |value| form.update(|f| f.waiting_penalty = value)) />
                        <ScoreField label="Paused-project penalty" hint="Tasks in paused projects"
                            value=Signal::derive(move || form.get().paused_project_penalty)
                            on_input=Callback::new(move |value| form.update(|f| f.paused_project_penalty = value)) />
                        <ScoreField label="Due-soon window (hours)" hint="How early 'due soon' begins"
                            value=Signal::derive(move || form.get().due_soon_window_hours)
                            on_input=Callback::new(move |value| form.update(|f| f.due_soon_window_hours = value)) />
                    </div>
                    <div class="settings-actions">
                        <Button variant="primary" on_click=Callback::new(move |_| save())>"Save weights"</Button>
                        <Button variant="ghost" on_click=Callback::new(move |_| reset())>"Reset to defaults"</Button>
                    </div>
                }.into_any()
            } else {
                view! { <LoadingState label="Loading scoring settings…" /> }.into_any()
            }}
        </Section>

        <Section title="Data export & backup">
            <p class="settings-note">
                "Exports download to your browser/webview downloads folder. The database backup writes a consistent SQLite snapshot to a path you choose."
            </p>
            <div class="data-actions">
                <Button variant="subtle" on_click=Callback::new(move |_| export(state, "export_tasks_json", "openmgmt-tasks.json"))>"Export tasks (JSON)"</Button>
                <Button variant="subtle" on_click=Callback::new(move |_| export(state, "export_tasks_csv", "openmgmt-tasks.csv"))>"Export tasks (CSV)"</Button>
                <Button variant="subtle" on_click=Callback::new(move |_| export(state, "export_all_json", "openmgmt-all.json"))>"Export everything (JSON)"</Button>
                <Button variant="ghost" on_click=Callback::new(move |_| backup(state))>"Back up database…"</Button>
            </div>
        </Section>
    }
}

/// A single labelled numeric scoring input.
#[component]
fn ScoreField(
    #[prop(into)] label: String,
    #[prop(into)] hint: String,
    value: Signal<String>,
    on_input: Callback<String>,
) -> impl IntoView {
    view! {
        <FormField label=label hint=hint>
            <input
                type="number"
                prop:value=move || value.get()
                on:input=move |event| on_input.run(event_target_value(&event))
            />
        </FormField>
    }
}

/// Run a string-returning export command and trigger a download.
fn export(state: AppState, command: &'static str, filename: &'static str) {
    spawn_local(async move {
        match invoke::<String>(command, json!({})).await {
            Ok(content) => match download_text(filename, &content) {
                Ok(()) => state.notice.set(Some(format!("Exported {filename}."))),
                Err(error) => state.fail("Export failed", error),
            },
            Err(error) => state.fail("Export failed", error),
        }
    });
}

/// Prompt for a destination path and write a SQLite backup there.
fn backup(state: AppState) {
    let Some(path) = prompt_with_default(
        "Save database backup to (full path or filename):",
        "openmgmt-backup.sqlite",
    ) else {
        return;
    };
    let path = path.trim().to_owned();
    if path.is_empty() {
        state.fail("Backup failed", "Destination path is required.".into());
        return;
    }
    spawn_local(async move {
        match invoke::<()>("backup_sqlite_database", json!({ "target_path": path })).await {
            Ok(()) => state.notice.set(Some("Database backup written.".into())),
            Err(error) => state.fail("Backup failed", error),
        }
    });
}
