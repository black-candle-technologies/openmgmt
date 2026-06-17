//! Tasks page: a saved-view strip plus filter/sort controls, all driving the
//! backend `query_tasks` command. Results carry organization, urgency score, and
//! live timer state, rendered by `QueryTaskTable`.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{SavedTaskView, TaskWithContext};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::app::components::*;
use crate::app::records::QueryTaskTable;
use crate::app::state::*;
use crate::app::views::*;

#[component]
pub fn TasksPage(state: AppState, now: RwSignal<DateTime<Utc>>) -> impl IntoView {
    let now_sig: Signal<DateTime<Utc>> = now.into();

    // One struct signal so a saved-view preset can set every control at once and
    // the query effect tracks a single dependency.
    let filter = RwSignal::new(TaskFilterState::all_active());
    let active_view = RwSignal::new(Some("all-tasks".to_string()));

    // Saved views from the backend; falls back to the known system presets when
    // the database has not been seeded yet.
    let saved_views = RwSignal::new(Vec::<SavedTaskView>::new());
    spawn_local(async move {
        if let Ok(list) = invoke::<Vec<SavedTaskView>>("list_saved_task_views", json!({})).await {
            saved_views.set(list);
        }
    });

    // Query results plus the instant they were fetched (so timers tick forward).
    let results = RwSignal::new(Vec::<TaskWithContext>::new());
    let queried_at = RwSignal::new(Utc::now());
    let querying = RwSignal::new(true);
    let generation = StoredValue::new(0u32);

    Effect::new(move |_| {
        let current = filter.get();
        // Re-run after any global snapshot reload (e.g. a mutation elsewhere).
        let _ = state.synced_at.get();
        let (query_filter, sort) = build_query(&current, Utc::now());
        let token = generation.get_value() + 1;
        generation.set_value(token);
        querying.set(true);
        spawn_local(async move {
            let fetched_at = Utc::now();
            let response = invoke::<Vec<TaskWithContext>>(
                "query_tasks",
                json!({"filter": query_filter, "sort": sort}),
            )
            .await;
            // Drop stale responses so the latest filter always wins.
            if generation.get_value() != token {
                return;
            }
            match response {
                Ok(rows) => {
                    results.set(rows);
                    queried_at.set(fetched_at);
                }
                Err(error) => state.fail("Query tasks failed", error),
            }
            querying.set(false);
        });
    });

    let results_sig = Signal::derive(move || results.get());
    let queried_sig = Signal::derive(move || queried_at.get());
    let loading_sig = Signal::derive(move || querying.get());
    let result_count = Signal::derive(move || results.get().len());

    // Distinct tags in use, for the tag filter.
    let all_tags = Signal::derive(move || {
        state
            .snapshot
            .get()
            .tasks
            .iter()
            .flat_map(|task| task.tags.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    });

    // The view-strip chips: backend saved views when present, else presets.
    let view_chips = Signal::derive(move || {
        let backend = saved_views.get();
        if backend.is_empty() {
            default_view_presets()
                .into_iter()
                .map(|(slug, label)| (slug.to_string(), label.to_string()))
                .collect::<Vec<_>>()
        } else {
            backend
                .into_iter()
                .filter(|view| view.archived_at.is_none())
                .map(|view| (view.slug, view.name))
                .collect::<Vec<_>>()
        }
    });

    view! {
        <PageHeader
            eyebrow="EXECUTION"
            title="Tasks"
            description="Saved views and filters, scored and sorted by the operations engine. Click a view, refine with filters, and run a timer inline."
        >
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <div class="view-strip">
            {move || view_chips.get().into_iter().map(|(slug, label)| {
                let slug_click = slug.clone();
                let slug_active = slug.clone();
                let is_active = move || active_view.get().as_deref() == Some(slug_active.as_str());
                view! {
                    <button class="view-chip" class:active=is_active on:click=move |_| {
                        active_view.set(Some(slug_click.clone()));
                        filter.set(preset_for_slug(&slug_click));
                    }>{label}</button>
                }
            }).collect_view()}
        </div>

        <div class="filter-bar">
            <label class="filter-control filter-grow">
                <span>"Search"</span>
                <input
                    type="search"
                    placeholder="Title, project, tag…"
                    prop:value=move || filter.get().text
                    on:input=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.text = value);
                        active_view.set(None);
                    }
                />
            </label>
            <label class="filter-control">
                <span>"Sort by"</span>
                <select
                    prop:value=move || filter.get().sort_field
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.sort_field = value);
                        active_view.set(None);
                    }
                >
                    <option value="urgency">"Urgency"</option>
                    <option value="priority">"Priority"</option>
                    <option value="due_at">"Due date"</option>
                    <option value="status">"Status"</option>
                    <option value="project">"Project"</option>
                    <option value="organization">"Organization"</option>
                </select>
            </label>
            <label class="filter-control">
                <span>"Order"</span>
                <select
                    prop:value=move || if filter.get().sort_desc { "desc" } else { "asc" }
                    on:change=move |event| {
                        let desc = event_target_value(&event) == "desc";
                        filter.update(|f| f.sort_desc = desc);
                        active_view.set(None);
                    }
                >
                    <option value="desc">"Descending"</option>
                    <option value="asc">"Ascending"</option>
                </select>
            </label>
        </div>

        <div class="filter-bar">
            <label class="filter-control">
                <span>"Organization"</span>
                <select
                    prop:value=move || filter.get().organization_id
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.organization_id = value);
                        active_view.set(None);
                    }
                >
                    <option value="">"All organizations"</option>
                    {move || state.snapshot.get().organizations.into_iter().map(|item| view! {
                        <option value=item.id>{item.name}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Project"</span>
                <select
                    prop:value=move || filter.get().project_id
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.project_id = value);
                        active_view.set(None);
                    }
                >
                    <option value="">"All projects"</option>
                    {move || state.snapshot.get().projects.into_iter().map(|item| view! {
                        <option value=item.id>{item.name}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Status"</span>
                <select
                    prop:value=move || filter.get().status
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.status = value);
                        active_view.set(None);
                    }
                >
                    <option value="">"All statuses"</option>
                    {task_status_options().into_iter().map(|(value, label)| view! {
                        <option value=value.to_string()>{label}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Priority"</span>
                <select
                    prop:value=move || filter.get().priority
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.priority = value);
                        active_view.set(None);
                    }
                >
                    <option value="">"All priorities"</option>
                    {(1..=5).map(|value| view! { <option value=value.to_string()>{format!("P{value}")}</option> }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Due"</span>
                <select
                    prop:value=move || filter.get().due_window
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.due_window = value);
                        active_view.set(None);
                    }
                >
                    <option value="">"Any time"</option>
                    <option value="overdue">"Overdue"</option>
                    <option value="today">"Due today"</option>
                    <option value="soon">"Due soon (24h)"</option>
                    <option value="week">"This week"</option>
                </select>
            </label>
            <label class="filter-control">
                <span>"Tag"</span>
                <select
                    prop:value=move || filter.get().tag
                    on:change=move |event| {
                        let value = event_target_value(&event);
                        filter.update(|f| f.tag = value);
                        active_view.set(None);
                    }
                >
                    <option value="">"All tags"</option>
                    {move || all_tags.get().into_iter().map(|tag| {
                        let value = tag.clone();
                        view! { <option value=value>{tag}</option> }
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-check">
                <input
                    type="checkbox"
                    prop:checked=move || filter.get().pinned_only
                    on:change=move |event| {
                        let checked = event_target_checked(&event);
                        filter.update(|f| f.pinned_only = checked);
                        active_view.set(None);
                    }
                />
                <span>"Pinned"</span>
            </label>
            <label class="filter-check">
                <input
                    type="checkbox"
                    prop:checked=move || filter.get().include_done
                    on:change=move |event| {
                        let checked = event_target_checked(&event);
                        filter.update(|f| f.include_done = checked);
                        active_view.set(None);
                    }
                />
                <span>"Include done"</span>
            </label>
        </div>

        <Panel>
            <div class="qtask-summary">
                <span class="count-chip">{move || result_count.get()}</span>
                <span class="qtask-summary-label">"matching tasks"</span>
                {move || querying.get().then(|| view! { <span class="qtask-summary-busy"><span class="spinner"></span>"querying"</span> })}
            </div>
            <QueryTaskTable state rows=results_sig now=now_sig queried_at=queried_sig loading=loading_sig />
        </Panel>
    }
}
