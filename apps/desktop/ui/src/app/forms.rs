//! Record create/edit forms, presented inside a focused side drawer.
//!
//! Keeping every mutation form in a drawer means the underlying pages never
//! show large always-open forms — they stay calm and scannable.

use leptos::prelude::*;
use openmgmt_core::{
    NewOrganization, NewProject, NewTask, Organization, OrganizationPatch, Project, ProjectPatch,
    ProjectStatus, ProjectType, Task, TaskPatch, TaskStatus,
};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::{FormField, IconButton};
use super::state::*;

/// Renders the active drawer (if any) as an overlay panel.
#[component]
pub fn DrawerHost(state: AppState) -> impl IntoView {
    move || {
        state.drawer.get().map(|drawer| {
            let (title, body) = match drawer {
                Drawer::CreateOrganization => (
                    "New organization".to_string(),
                    view! { <OrganizationForm state /> }.into_any(),
                ),
                Drawer::EditOrganization(organization) => (
                    "Edit organization".to_string(),
                    view! { <OrganizationForm state existing=organization /> }.into_any(),
                ),
                Drawer::CreateProject { organization_id } => (
                    "New project".to_string(),
                    view! { <ProjectForm state preset_org=organization_id.unwrap_or_default() /> }.into_any(),
                ),
                Drawer::EditProject(project) => (
                    "Edit project".to_string(),
                    view! { <ProjectForm state existing=project /> }.into_any(),
                ),
                Drawer::CreateTask { project_id } => (
                    "New task".to_string(),
                    view! { <TaskForm state preset_project=project_id.unwrap_or_default() /> }.into_any(),
                ),
                Drawer::EditTask(task) => (
                    "Edit task".to_string(),
                    view! { <TaskForm state existing=task /> }.into_any(),
                ),
            };
            view! {
                <div class="drawer-layer">
                    <div class="drawer-backdrop" on:click=move |_| state.close_drawer()></div>
                    <aside class="drawer">
                        <header class="drawer-head">
                            <h2>{title}</h2>
                            <IconButton label="Close" title="Close" on_click=Callback::new(move |_| state.close_drawer())>"✕"</IconButton>
                        </header>
                        <div class="drawer-body">{body}</div>
                    </aside>
                </div>
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Organization
// ---------------------------------------------------------------------------

#[component]
fn OrganizationForm(
    state: AppState,
    #[prop(optional)] existing: Option<Organization>,
) -> impl IntoView {
    let name = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Textarea>::new();
    let color = NodeRef::<leptos::html::Input>::new();
    let icon = NodeRef::<leptos::html::Input>::new();

    let editing_id = existing.as_ref().map(|item| item.id.clone());
    let init_name = existing.as_ref().map(|item| item.name.clone()).unwrap_or_default();
    let init_desc = existing
        .as_ref()
        .and_then(|item| item.description.clone())
        .unwrap_or_default();
    let init_color = existing
        .as_ref()
        .and_then(|item| item.color.clone())
        .unwrap_or_else(|| "#52725a".into());
    let init_icon = existing
        .as_ref()
        .and_then(|item| item.icon.clone())
        .unwrap_or_default();
    let archive = existing.as_ref().map(|item| (item.id.clone(), item.name.clone()));
    let submit_label = if editing_id.is_some() {
        "Save changes"
    } else {
        "Create organization"
    };

    view! {
        <form class="drawer-form" on:submit=move |event| {
            event.prevent_default();
            let editing_id = editing_id.clone();
            spawn_local(async move {
                let ok = match editing_id {
                    Some(id) => {
                        let patch = OrganizationPatch {
                            name: Some(input_value(name)),
                            slug: None,
                            description: Some(optional_text(textarea_value(description))),
                            color: Some(optional_text(input_value(color))),
                            icon: Some(optional_text(input_value(icon))),
                        };
                        finish_action(state, invoke::<Organization>("update_organization", json!({"id":id,"patch":patch})).await, "Organization updated.", "Update organization failed").await
                    }
                    None => {
                        let input = NewOrganization {
                            name: input_value(name),
                            slug: None,
                            description: optional_text(textarea_value(description)),
                            color: optional_text(input_value(color)),
                            icon: optional_text(input_value(icon)),
                        };
                        finish_action(state, invoke::<Organization>("create_organization", json!({"input":input})).await, "Organization created.", "Create organization failed").await
                    }
                };
                if ok { state.close_drawer(); }
            });
        }>
            <FormField label="Name">
                <input node_ref=name value=init_name placeholder="Acme Studio" required />
            </FormField>
            <FormField label="Description">
                <textarea node_ref=description placeholder="What this group is responsible for">{init_desc}</textarea>
            </FormField>
            <div class="form-row">
                <FormField label="Color">
                    <input node_ref=color type="color" value=init_color />
                </FormField>
                <FormField label="Icon initials" hint="Up to 4 characters">
                    <input node_ref=icon value=init_icon maxlength="4" placeholder="OM" />
                </FormField>
            </div>
            <div class="drawer-actions">
                <button class="btn btn-primary" type="submit">{submit_label}</button>
                {archive.map(|(id, archive_name)| view! {
                    <button class="btn btn-danger-soft" type="button" on:click=move |_| {
                        if !confirmed(&format!("Archive {archive_name}? Its projects and tasks will be hidden.")) { return; }
                        let id = id.clone();
                        spawn_local(async move {
                            if finish_action(state, invoke::<()>("archive_organization", json!({"id":id})).await, "Organization archived.", "Archive organization failed").await {
                                state.close_drawer();
                            }
                        });
                    }>"Archive"</button>
                })}
            </div>
        </form>
    }
}

// ---------------------------------------------------------------------------
// Project
// ---------------------------------------------------------------------------

#[component]
fn ProjectForm(
    state: AppState,
    #[prop(optional)] existing: Option<Project>,
    #[prop(optional)] preset_org: String,
) -> impl IntoView {
    let organization = NodeRef::<leptos::html::Select>::new();
    let name = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Textarea>::new();
    let project_type = NodeRef::<leptos::html::Select>::new();
    let status = NodeRef::<leptos::html::Select>::new();
    let priority = NodeRef::<leptos::html::Select>::new();
    let deadline = NodeRef::<leptos::html::Input>::new();
    let repo_url = NodeRef::<leptos::html::Input>::new();
    let notes = NodeRef::<leptos::html::Textarea>::new();

    let editing_id = existing.as_ref().map(|item| item.id.clone());
    let selected_org = existing
        .as_ref()
        .map(|item| item.organization_id.clone())
        .unwrap_or(preset_org);
    let init_name = existing.as_ref().map(|item| item.name.clone()).unwrap_or_default();
    let init_desc = existing.as_ref().and_then(|item| item.description.clone()).unwrap_or_default();
    let init_type = existing.as_ref().map(|item| item.project_type).unwrap_or(ProjectType::Software);
    let init_status = existing.as_ref().map(|item| item.status).unwrap_or(ProjectStatus::Active);
    let init_priority = existing.as_ref().map(|item| item.priority).unwrap_or(3);
    let init_deadline = datetime_local_value(existing.as_ref().and_then(|item| item.deadline));
    let init_repo = existing.as_ref().and_then(|item| item.repo_url.clone()).unwrap_or_default();
    let init_notes = existing.as_ref().and_then(|item| item.notes.clone()).unwrap_or_default();
    let archive = existing.as_ref().map(|item| (item.id.clone(), item.name.clone()));
    let submit_label = if editing_id.is_some() { "Save changes" } else { "Create project" };

    view! {
        <form class="drawer-form" on:submit=move |event| {
            event.prevent_default();
            let editing_id = editing_id.clone();
            let deadline_value = match parse_datetime_local(input_value(deadline)) {
                Ok(value) => value,
                Err(error) => { state.fail("Save project failed", error); return; }
            };
            spawn_local(async move {
                let ok = match editing_id {
                    Some(id) => {
                        let patch = ProjectPatch {
                            name: Some(input_value(name)),
                            slug: None,
                            description: Some(optional_text(textarea_value(description))),
                            project_type: select_value(project_type).parse().ok(),
                            status: select_value(status).parse().ok(),
                            priority: parse_i32(select_value(priority)),
                            deadline: Some(deadline_value),
                            repo_url: Some(optional_text(input_value(repo_url))),
                            notes: Some(optional_text(textarea_value(notes))),
                        };
                        finish_action(state, invoke::<Project>("update_project", json!({"id":id,"patch":patch})).await, "Project updated.", "Update project failed").await
                    }
                    None => {
                        let input = NewProject {
                            organization_id: select_value(organization),
                            name: input_value(name),
                            slug: None,
                            description: optional_text(textarea_value(description)),
                            project_type: select_value(project_type).parse().unwrap_or(ProjectType::Other),
                            status: select_value(status).parse().unwrap_or(ProjectStatus::Active),
                            priority: parse_i32(select_value(priority)).unwrap_or(3),
                            deadline: deadline_value,
                            repo_url: optional_text(input_value(repo_url)),
                            notes: optional_text(textarea_value(notes)),
                        };
                        finish_action(state, invoke::<Project>("create_project", json!({"input":input})).await, "Project created.", "Create project failed").await
                    }
                };
                if ok { state.close_drawer(); }
            });
        }>
            <FormField label="Organization">
                <select node_ref=organization required>
                    <option value="">"Select organization"</option>
                    {let selected = selected_org.clone(); move || {
                        let selected = selected.clone();
                        state.snapshot.get().organizations.into_iter().map(|item| {
                            let is_selected = item.id == selected;
                            view! { <option value=item.id selected=is_selected>{item.name}</option> }
                        }).collect_view()
                    }}
                </select>
            </FormField>
            <FormField label="Name">
                <input node_ref=name value=init_name placeholder="Project name" required />
            </FormField>
            <FormField label="Description">
                <textarea node_ref=description placeholder="Short summary">{init_desc}</textarea>
            </FormField>
            <div class="form-row">
                <FormField label="Type">
                    <select node_ref=project_type>
                        {project_type_options().into_iter().map(|(value, label)| {
                            let selected = value == init_type;
                            view! { <option value=value.to_string() selected=selected>{label}</option> }
                        }).collect_view()}
                    </select>
                </FormField>
                <FormField label="Status">
                    <select node_ref=status>
                        {project_status_options().into_iter().map(|(value, label)| {
                            let selected = value == init_status;
                            view! { <option value=value.to_string() selected=selected>{label}</option> }
                        }).collect_view()}
                    </select>
                </FormField>
            </div>
            <div class="form-row">
                <FormField label="Priority">
                    <select node_ref=priority>
                        {(1..=5).map(|value| view! { <option value=value selected=value==init_priority>{format!("P{value}")}</option> }).collect_view()}
                    </select>
                </FormField>
                <FormField label="Deadline">
                    <input node_ref=deadline type="datetime-local" value=init_deadline />
                </FormField>
            </div>
            <FormField label="Repository URL">
                <input node_ref=repo_url value=init_repo placeholder="https://github.com/…" />
            </FormField>
            <FormField label="Notes">
                <textarea node_ref=notes placeholder="Context, links, decisions">{init_notes}</textarea>
            </FormField>
            <div class="drawer-actions">
                <button class="btn btn-primary" type="submit">{submit_label}</button>
                {archive.map(|(id, archive_name)| view! {
                    <button class="btn btn-danger-soft" type="button" on:click=move |_| {
                        if !confirmed(&format!("Archive project {archive_name}? Its tasks will be hidden.")) { return; }
                        let id = id.clone();
                        spawn_local(async move {
                            if finish_action(state, invoke::<()>("archive_project", json!({"id":id})).await, "Project archived.", "Archive project failed").await {
                                state.close_drawer();
                            }
                        });
                    }>"Archive"</button>
                })}
            </div>
        </form>
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

#[component]
fn TaskForm(
    state: AppState,
    #[prop(optional)] existing: Option<Task>,
    #[prop(optional)] preset_project: String,
) -> impl IntoView {
    let project = NodeRef::<leptos::html::Select>::new();
    let title = NodeRef::<leptos::html::Input>::new();
    let description = NodeRef::<leptos::html::Textarea>::new();
    let status = NodeRef::<leptos::html::Select>::new();
    let priority = NodeRef::<leptos::html::Select>::new();
    let due_at = NodeRef::<leptos::html::Input>::new();
    let scheduled_at = NodeRef::<leptos::html::Input>::new();
    let estimated = NodeRef::<leptos::html::Input>::new();
    let time_limit = NodeRef::<leptos::html::Input>::new();
    let pinned = NodeRef::<leptos::html::Input>::new();
    let blocked_reason = NodeRef::<leptos::html::Input>::new();
    let tags = NodeRef::<leptos::html::Input>::new();

    let editing = existing.is_some();
    let editing_id = existing.as_ref().map(|item| item.id.clone());
    let selected_project = existing
        .as_ref()
        .map(|item| item.project_id.clone())
        .unwrap_or(preset_project);
    let init_title = existing.as_ref().map(|item| item.title.clone()).unwrap_or_default();
    let init_desc = existing.as_ref().and_then(|item| item.description.clone()).unwrap_or_default();
    let init_status = existing.as_ref().map(|item| item.status).unwrap_or(TaskStatus::Inbox);
    let init_priority = existing.as_ref().map(|item| item.priority).unwrap_or(3);
    let init_due = datetime_local_value(existing.as_ref().and_then(|item| item.due_at));
    let init_scheduled = datetime_local_value(existing.as_ref().and_then(|item| item.scheduled_at));
    let init_estimated = existing.as_ref().and_then(|item| item.estimated_minutes).map(|value| value.to_string()).unwrap_or_default();
    let init_limit = existing.as_ref().and_then(|item| item.time_limit_minutes).map(|value| value.to_string()).unwrap_or_default();
    let init_pinned = existing.as_ref().map(|item| item.pinned).unwrap_or(false);
    let init_blocked = existing.as_ref().and_then(|item| item.blocked_reason.clone()).unwrap_or_default();
    let init_tags = existing.as_ref().map(|item| item.tags.join(", ")).unwrap_or_default();
    let submit_label = if editing { "Save changes" } else { "Create task" };

    view! {
        <form class="drawer-form" on:submit=move |event| {
            event.prevent_default();
            let editing_id = editing_id.clone();
            let due = match parse_datetime_local(input_value(due_at)) {
                Ok(value) => value,
                Err(error) => { state.fail("Save task failed", error); return; }
            };
            let scheduled = match parse_datetime_local(input_value(scheduled_at)) {
                Ok(value) => value,
                Err(error) => { state.fail("Save task failed", error); return; }
            };
            spawn_local(async move {
                let ok = match editing_id {
                    Some(id) => {
                        let patch = TaskPatch {
                            title: Some(input_value(title)),
                            description: Some(optional_text(textarea_value(description))),
                            status: select_value(status).parse().ok(),
                            priority: parse_i32(select_value(priority)),
                            due_at: Some(due),
                            scheduled_at: Some(scheduled),
                            estimated_minutes: Some(parse_i32(input_value(estimated))),
                            time_limit_minutes: Some(parse_i32(input_value(time_limit))),
                            pinned: Some(checkbox_value(pinned)),
                            blocked_reason: Some(optional_text(input_value(blocked_reason))),
                            tags: Some(input_value(tags).split(',').map(str::trim).filter(|tag| !tag.is_empty()).map(str::to_owned).collect()),
                        };
                        finish_action(state, invoke::<Task>("update_task", json!({"id":id,"patch":patch})).await, "Task updated.", "Update task failed").await
                    }
                    None => {
                        let input = NewTask {
                            project_id: select_value(project),
                            title: input_value(title),
                            description: optional_text(textarea_value(description)),
                            status: select_value(status).parse().unwrap_or(TaskStatus::Inbox),
                            priority: parse_i32(select_value(priority)).unwrap_or(3),
                            due_at: due,
                            scheduled_at: scheduled,
                            estimated_minutes: parse_i32(input_value(estimated)),
                            time_limit_minutes: parse_i32(input_value(time_limit)),
                            pinned: checkbox_value(pinned),
                            tags: input_value(tags).split(',').map(str::trim).filter(|tag| !tag.is_empty()).map(str::to_owned).collect(),
                        };
                        finish_action(state, invoke::<Task>("create_task", json!({"input":input})).await, "Task created.", "Create task failed").await
                    }
                };
                if ok { state.close_drawer(); }
            });
        }>
            <FormField label="Project">
                <select node_ref=project required>
                    <option value="">"Select project"</option>
                    {let selected = selected_project.clone(); move || {
                        let selected = selected.clone();
                        state.snapshot.get().projects.into_iter().map(|item| {
                            let is_selected = item.id == selected;
                            view! { <option value=item.id selected=is_selected>{item.name}</option> }
                        }).collect_view()
                    }}
                </select>
            </FormField>
            <FormField label="Title">
                <input node_ref=title value=init_title placeholder="What needs to happen" required />
            </FormField>
            <FormField label="Description">
                <textarea node_ref=description placeholder="Optional detail">{init_desc}</textarea>
            </FormField>
            <div class="form-row">
                <FormField label="Status">
                    <select node_ref=status>
                        {task_status_options().into_iter().map(|(value, label)| {
                            let selected = value == init_status;
                            view! { <option value=value.to_string() selected=selected>{label}</option> }
                        }).collect_view()}
                    </select>
                </FormField>
                <FormField label="Priority">
                    <select node_ref=priority>
                        {(1..=5).map(|value| view! { <option value=value selected=value==init_priority>{format!("P{value}")}</option> }).collect_view()}
                    </select>
                </FormField>
            </div>
            <div class="form-row">
                <FormField label="Due">
                    <input node_ref=due_at type="datetime-local" value=init_due />
                </FormField>
                <FormField label="Scheduled">
                    <input node_ref=scheduled_at type="datetime-local" value=init_scheduled />
                </FormField>
            </div>
            <details class="form-advanced">
                <summary>"Advanced"</summary>
                <div class="form-row">
                    <FormField label="Estimate (min)">
                        <input node_ref=estimated type="number" min="1" value=init_estimated />
                    </FormField>
                    <FormField label="Time limit (min)">
                        <input node_ref=time_limit type="number" min="1" value=init_limit />
                    </FormField>
                </div>
                <FormField label="Tags" hint="Comma separated">
                    <input node_ref=tags value=init_tags placeholder="design, urgent" />
                </FormField>
                <FormField label="Blocked reason">
                    <input node_ref=blocked_reason value=init_blocked placeholder="Why is this waiting?" />
                </FormField>
                <label class="form-check">
                    <input node_ref=pinned type="checkbox" checked=init_pinned />
                    <span>"Pin to top of board"</span>
                </label>
            </details>
            <div class="drawer-actions">
                <button class="btn btn-primary" type="submit">{submit_label}</button>
            </div>
        </form>
    }
}
