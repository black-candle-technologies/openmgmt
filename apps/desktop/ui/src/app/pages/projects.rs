use leptos::prelude::*;

use crate::app::components::*;
use crate::app::records::ProjectCard;
use crate::app::state::*;

#[component]
pub fn ProjectsPage(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    // Visual filters. Empty string means "all".
    let org_filter = RwSignal::new(String::new());
    let status_filter = RwSignal::new(String::new());
    let type_filter = RwSignal::new(String::new());

    let filtered = Signal::derive(move || {
        let org = org_filter.get();
        let status = status_filter.get();
        let kind = type_filter.get();
        state
            .snapshot
            .get()
            .projects
            .into_iter()
            .filter(|project| org.is_empty() || project.organization_id == org)
            .filter(|project| status.is_empty() || project.status.to_string() == status)
            .filter(|project| kind.is_empty() || project.project_type.to_string() == kind)
            .collect::<Vec<_>>()
    });

    view! {
        <PageHeader
            eyebrow="PORTFOLIO"
            title="Projects"
            description="Active bodies of work across your organizations."
        >
            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateProject { organization_id: None }))>"New project"</Button>
        </PageHeader>

        <div class="filter-bar">
            <label class="filter-control">
                <span>"Organization"</span>
                <select on:change=move |event| org_filter.set(event_target_value(&event))>
                    <option value="">"All"</option>
                    {move || state.snapshot.get().organizations.into_iter().map(|item| view! {
                        <option value=item.id>{item.name}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Status"</span>
                <select on:change=move |event| status_filter.set(event_target_value(&event))>
                    <option value="">"All"</option>
                    {project_status_options().into_iter().map(|(value, label)| view! {
                        <option value=value.to_string()>{label}</option>
                    }).collect_view()}
                </select>
            </label>
            <label class="filter-control">
                <span>"Type"</span>
                <select on:change=move |event| type_filter.set(event_target_value(&event))>
                    <option value="">"All"</option>
                    {project_type_options().into_iter().map(|(value, label)| view! {
                        <option value=value.to_string()>{label}</option>
                    }).collect_view()}
                </select>
            </label>
        </div>

        {move || {
            let projects = filtered.get();
            if projects.is_empty() {
                view! {
                    <Panel>
                        <EmptyState title="No projects match" hint="Adjust the filters or create a new project.">
                            <Button variant="primary" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateProject { organization_id: None }))>"New project"</Button>
                        </EmptyState>
                    </Panel>
                }.into_any()
            } else {
                view! {
                    <div class="record-grid">
                        {projects.into_iter().map(|project| view! {
                            <ProjectCard state project page />
                        }).collect_view()}
                    </div>
                }.into_any()
            }
        }}
    }
}
