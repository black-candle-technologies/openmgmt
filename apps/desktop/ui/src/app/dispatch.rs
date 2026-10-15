//! Dispatch presentation. All HTTP and token access stay in the native shell.
use super::state::*;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use openmgmt_core::{AgentState, Project, Task};
use serde_json::{Value, json};
use wasm_bindgen_futures::spawn_local;

async fn request(
    action: &str,
    task_id: &str,
    id: Option<&str>,
    body: Value,
) -> Result<Value, String> {
    invoke(
        "dispatch_request",
        json!({"action": action, "taskId": task_id, "id": id, "body": body}),
    )
    .await
}
#[component]
pub fn AgentPill(task: Task) -> impl IntoView {
    task.agent.map(|agent| view! { <span class=format!("agent-pill agent-{:?}", agent.state)>{agent.state.label()}</span> })
}
#[component]
pub fn RepositoryBinding(state: AppState, project: Project) -> impl IntoView {
    let binding = project.repo_binding;
    let owner = RwSignal::new(
        binding
            .as_ref()
            .map(|b| b.owner.clone())
            .unwrap_or_default(),
    );
    let name = RwSignal::new(binding.as_ref().map(|b| b.name.clone()).unwrap_or_default());
    let branch = RwSignal::new(
        binding
            .as_ref()
            .map(|b| b.default_branch.clone())
            .unwrap_or_else(|| "main".into()),
    );
    let id = StoredValue::new(project.id);
    view! {
        <details class="panel"><summary>"Repository binding"</summary>
            <div class="settings-grid">
                <label>"Owner"<input prop:value=move || owner.get() on:input=move |e| owner.set(event_target_value(&e)) /></label>
                <label>"Repository"<input prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e)) /></label>
                <label>"Default branch"<input prop:value=move || branch.get() on:input=move |e| branch.set(event_target_value(&e)) /></label>
            </div>
            <button class="btn btn-subtle" on:click=move |_| spawn_local(async move {
                match invoke::<()>("bind_repository", json!({"projectId": id.get_value(), "binding": {"owner": owner.get_untracked(), "name": name.get_untracked(), "default_branch": branch.get_untracked()}})).await {
                    Ok(()) => { state.refresh(); state.notice.set(Some("Repository bound.".into())); }
                    Err(e) => state.fail("Repository binding failed", e),
                }
            })>"Save binding"</button>
        </details>
    }
}
#[component]
pub fn AgentSection(state: AppState, task: Task) -> impl IntoView {
    let page = expect_context::<RwSignal<Page>>();
    let signed_in = RwSignal::new(false);
    spawn_local(async move {
        signed_in.set(
            invoke::<bool>("dispatch_signed_in", json!({}))
                .await
                .unwrap_or(false),
        );
    });
    let task = StoredValue::new(task);
    let composing = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let prompt = RwSignal::new(String::new());
    let models = RwSignal::new(Vec::<(String, String)>::new());
    let model = RwSignal::new(String::new());
    let has_repo = Signal::derive(move || {
        state
            .snapshot
            .get()
            .projects
            .iter()
            .any(|p| p.id == task.get_value().project_id && p.repo_binding.is_some())
    });
    let generate = Callback::new(move |_: ()| {
        busy.set(true);
        error.set(None);
        composing.set(true);
        spawn_local(async move {
            let t = task.get_value();
            // A failed generation never silently dispatches a template.
            prompt.set(format!(
                "Implement: {}\n\n{}\n\nRun relevant tests and open a pull request.",
                t.title,
                t.description.unwrap_or_default()
            ));
            match request("generate", &t.id, None, json!({})).await {
                Ok(v) => match v["prompt"].as_str() {
                    Some(text) => prompt.set(text.into()),
                    None => error.set(Some(
                        "Invalid draft response. Template draft shown; retry.".into(),
                    )),
                },
                Err(e) => error.set(Some(format!("{e}. Template draft shown; retry."))),
            }
            match request("models", &t.id, None, json!({})).await {
                Ok(v) => {
                    let entries = v
                        .as_array()
                        .or_else(|| v["models"].as_array())
                        .cloned()
                        .unwrap_or_default();
                    let options = entries
                        .into_iter()
                        .filter_map(|m| {
                            let id = m["id"].as_str().or(m["model"].as_str())?.to_owned();
                            Some((
                                id.clone(),
                                format!("{} / {id}", m["provider"].as_str().unwrap_or("Dispatch")),
                            ))
                        })
                        .collect::<Vec<_>>();
                    model.set(options.first().map(|m| m.0.clone()).unwrap_or_default());
                    models.set(options);
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    });
    view! {
        <section class="agent-section">
            <h3>"Agent"</h3>
            {move || if !signed_in.get() {
                view! { <button class="btn btn-ghost" on:click=move |_| { state.close_drawer(); page.set(Page::Sync); }>"Sign in on Sync to use Dispatch →"</button> }.into_any()
            } else if let Some(agent) = task.get_value().agent {
                let id = task.get_value().id;
                view! { <AgentPill task=task.get_value() /> <button class="btn btn-subtle" on:click=move |_| { state.close_drawer(); page.set(Page::AgentSession(id.clone(), agent.dispatch_id.clone())); }>"Agent session →"</button> }.into_any()
            } else {
                view! {
                    {move || (!has_repo.get()).then(|| view! { <p class="banner banner-error">"Bind a repository before dispatching."<button class="btn btn-ghost" on:click=move |_| { state.close_drawer(); page.set(Page::Project(task.get_value().project_id)); }>"Bind a repository →"</button></p> })}
                    <button class="btn btn-subtle" disabled=move || !has_repo.get() || busy.get() on:click=move |_| generate.run(())>"Dispatch agent"</button>
                }.into_any()
            }}
            {move || composing.get().then(|| view! {
                <div role="dialog" aria-label="Compose agent dispatch" aria-busy=move || busy.get()>
                    {move || busy.get().then(|| view! { <p class="skeleton">"Drafting with MiniMax M3…"</p> })}
                    <textarea aria-label="Dispatch prompt" rows="10" prop:value=move || prompt.get() on:input=move |e| prompt.set(event_target_value(&e)) />
                    <select aria-label="Provider and model" prop:value=move || model.get() on:change=move |e| model.set(event_target_value(&e))>
                        {move || models.get().into_iter().map(|(id, label)| view! { <option value=id>{label}</option> }).collect_view()}
                    </select>
                    <button class="btn btn-ghost" disabled=move || busy.get() on:click=move |_| generate.run(())>"Regenerate / retry"</button>
                    <button class="btn btn-primary" disabled=move || busy.get() || model.get().is_empty() || prompt.get().trim().is_empty() || !has_repo.get() on:click=move |_| {
                        busy.set(true); error.set(None);
                        spawn_local(async move {
                            let t = task.get_value();
                            match request("dispatch", &t.id, None, json!({"prompt": prompt.get_untracked(), "model": model.get_untracked()})).await {
                                Ok(_) => { state.reload().await; state.close_drawer(); state.notice.set(Some("Agent dispatched.".into())); }
                                Err(e) => error.set(Some(e)),
                            }
                            busy.set(false);
                        });
                    }>"Dispatch"</button>
                    <button class="btn btn-ghost" disabled=move || busy.get() on:click=move |_| composing.set(false)>"Cancel"</button>
                </div>
            })}
            {move || error.get().map(|e| view! { <p role="alert">{e}</p> })}
        </section>
    }
}
#[component]
pub fn AgentSession(state: AppState, task_id: String, id: String) -> impl IntoView {
    let task_id = StoredValue::new(task_id);
    let id = StoredValue::new(id);
    let data = RwSignal::new(Value::Null);
    let error = RwSignal::new(None::<String>);
    let followup = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let alive = RwSignal::new(true);
    let refresh = Callback::new(move |_: ()| {
        spawn_local(async move {
            let result = request(
                "get",
                &task_id.get_value(),
                Some(&id.get_value()),
                json!({}),
            )
            .await;
            if !alive.try_get().unwrap_or(false) {
                return;
            }
            match result {
                Ok(v) => {
                    data.set(v);
                    error.set(None);
                }
                Err(e) => error.set(Some(e)),
            }
        })
    });
    refresh.run(());
    let poll = Interval::new(15_000, move || refresh.run(()));
    // Store timer outside Send/Sync cleanup closure, dispose it on route teardown.
    let poll = StoredValue::new_local(Some(poll));
    on_cleanup(move || {
        alive.set(false);
        poll.update_value(|p| {
            p.take();
        });
    });
    let action = Callback::new(move |kind: &'static str| {
        busy.set(true);
        spawn_local(async move {
            let result = request(
                kind,
                &task_id.get_value(),
                Some(&id.get_value()),
                json!({"message": followup.get_untracked()}),
            )
            .await;
            if !alive.try_get().unwrap_or(false) {
                return;
            }
            match result {
                Ok(_) => {
                    followup.set(String::new());
                    refresh.run(());
                    state.refresh();
                }
                Err(e) => error.set(Some(e)),
            }
            busy.set(false);
        });
    });
    view! {
        <h1>"Agent session"</h1><p class="meta-mono">{id.get_value()}</p>
        <p>{move || { let v = data.get(); AgentState::from_api(v["state"].as_str().or(v["status"].as_str()).unwrap_or("dispatched")).label() }}</p>
        {move || error.get().map(|e| view! { <p class="banner banner-error" role="alert">{e}</p> })}
        <p>{move || data.get()["latest_activity"].as_str().unwrap_or("Waiting for activity…").to_owned()}</p>
        <ol>{move || data.get()["events"].as_array().cloned().unwrap_or_default().into_iter().map(|event| view! { <li>{event["message"].as_str().or(event["state"].as_str()).unwrap_or("State updated").to_owned()}</li> }).collect_view()}</ol>
        {move || data.get()["pr_url"].as_str().filter(|url| url.starts_with("https://github.com/")).map(|url| view! { <a class="btn btn-subtle" href=url.to_owned() target="_blank" rel="noopener noreferrer">"Open PR ↗"</a> })}
        <button class="btn btn-danger-soft" disabled=move || busy.get() || error.get().is_some() on:click=move |_| { if confirmed("Stop this agent session?") { action.run("cancel"); } }>"Stop"</button>
        <label>"Follow-up"<textarea prop:value=move || followup.get() on:input=move |e| followup.set(event_target_value(&e)) /></label>
        <button class="btn btn-primary" disabled=move || busy.get() || error.get().is_some() || followup.get().trim().is_empty() on:click=move |_| action.run("message")>"Send"</button>
    }
}
