use leptos::prelude::*;

use crate::app::components::*;
use crate::app::records::TaskTable;
use crate::app::state::*;

#[component]
pub fn ProjectDetailPage(state: AppState, page: RwSignal<Page>, id: String) -> impl IntoView {
    let meta_id = id.clone();
    let tasks_id = id.clone();
    let new_task_id = id.clone();

    let project_tasks = Signal::derive(move || {
        state
            .snapshot
            .get()
            .tasks
            .into_iter()
            .filter(|task| task.project_id == tasks_id)
            .collect::<Vec<_>>()
    });

    view! {
        <button class="back-link" on:click=move |_| page.set(Page::Projects)>"← All projects"</button>
        {move || {
            let snapshot = state.snapshot.get();
            match snapshot.projects.iter().find(|project| project.id == meta_id).cloned() {
                Some(project) => {
                    let org_name = snapshot
                        .organizations
                        .iter()
                        .find(|org| org.id == project.organization_id)
                        .map(|org| org.name.clone())
                        .unwrap_or_else(|| "—".into());
                    let edit_project = project.clone();
                    let deadline = project
                        .deadline
                        .map(|at| at.format("%b %-d, %Y %-I:%M %p").to_string())
                        .unwrap_or_else(|| "—".into());
                    let repo = project.repo_url.clone().unwrap_or_else(|| "—".into());
                    let notes = project.notes.clone().unwrap_or_else(|| "No notes.".into());
                    let kind = humanize(&project.project_type.to_string());
                    let status = project.status.to_string();
                    let priority = project.priority;
                    let description = project
                        .description
                        .clone()
                        .unwrap_or_else(|| "No description.".into());
                    let title = project.name.clone();

                    view! {
                        <PageHeader eyebrow=org_name.clone() title=title description=description>
                            <Button variant="ghost" on_click=Callback::new(move |_| state.open_drawer(Drawer::EditProject(edit_project.clone())))>"Edit project"</Button>
                        </PageHeader>
                        <section class="meta-grid">
                            <div class="meta-item"><span class="meta-label">"Organization"</span><span class="meta-value">{org_name}</span></div>
                            <div class="meta-item"><span class="meta-label">"Type"</span><span class="meta-value">{kind}</span></div>
                            <div class="meta-item"><span class="meta-label">"Status"</span><StatusBadge status /></div>
                            <div class="meta-item"><span class="meta-label">"Priority"</span><PriorityBadge value=priority /></div>
                            <div class="meta-item"><span class="meta-label">"Deadline"</span><span class="meta-value">{deadline}</span></div>
                            <div class="meta-item"><span class="meta-label">"Repository"</span><span class="meta-value meta-mono">{repo}</span></div>
                            <div class="meta-item meta-item-wide"><span class="meta-label">"Notes"</span><span class="meta-value">{notes}</span></div>
                        </section>
                    }.into_any()
                }
                None => view! { <Panel><EmptyState title="Project not found" hint="It may have been archived." /></Panel> }.into_any(),
            }
        }}

        <Section title="Project tasks">
            <div class="section-inline-action">
                <Button variant="subtle" on_click=Callback::new(move |_| state.open_drawer(Drawer::CreateTask { project_id: Some(new_task_id.clone()) }))>"New task"</Button>
            </div>
            <TaskTable state tasks=project_tasks show_project=false empty_hint="add one with New task" />
        </Section>
    }
}
