use leptos::prelude::*;

use crate::app::components::*;
use crate::app::records::TaskTable;
use crate::app::state::*;

#[component]
pub fn TasksPage(state: AppState) -> impl IntoView {
    let project_filter = RwSignal::new(String::new());
    let status_filter = RwSignal::new(String::new());

    let filtered = Signal::derive(move || {
        let project = project_filter.get();
        let status = status_filter.get();
        state
            .snapshot
            .get()
            .tasks
            .into_iter()
            .filter(|task| project.is_empty() || task.project_id == project)
            .filter(|task| status.is_empty() || task.status.to_string() == status)
            .collect::<Vec<_>>()
    });

    view! {
        <PageHeader
            eyebrow="EXECUTION"
            title="Tasks"
            description="Every task in a lightweight, scannable table. Click a title to edit."
        >
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: None }))>"New task"</Button>
        </PageHeader>

        <div class="filter-bar">
            <label class="filter-control">
                <span>"Project"</span>
                <select on:change=move |event| project_filter.set(event_target_value(&event))>
                    <option value="">"All projects"</option>
                    {move || state.snapshot.get().projects.into_iter().map(|item| view! {
                        <option value=item.id>{item.name}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Status"</span>
                <select on:change=move |event| status_filter.set(event_target_value(&event))>
                    <option value="">"All statuses"</option>
                    {task_status_options().into_iter().map(|(value, label)| view! {
                        <option value=value.to_string()>{label}</option>
                    }).collect_view()}
                </select>
            </label>
        </div>

        <Panel>
            <TaskTable state tasks=filtered show_project=true empty_hint="create one or adjust filters" />
        </Panel>
    }
}
