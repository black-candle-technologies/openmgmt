//! Local AI / Ollama: connection settings, onboarding, and read-only assistant
//! actions. Everything is centralized here so the rest of the app stays calm —
//! the page tests the connection, lists local models, saves settings, and runs
//! the planning workflows against the configured local Ollama server.

use leptos::prelude::*;
use openmgmt_core::{
    LocalAiConnectionResult, LocalAiModelListResult, LocalAiSettings, LocalAiSettingsPatch,
    LocalAiWorkflowResponse,
};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use crate::app::components::*;
use crate::app::state::*;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
const DEFAULT_MODEL: &str = "qwen3:1.7b";
const DEFAULT_KEEP_ALIVE: &str = "5m";

/// Editable mirror + loaded data for the Local AI page. `Copy` so it can be
/// handed to every closure cheaply (same pattern as the Sync page).
#[derive(Clone, Copy)]
struct LocalAiView {
    settings: RwSignal<Option<LocalAiSettings>>,
    base_url: RwSignal<String>,
    default_model: RwSignal<String>,
    keep_alive: RwSignal<String>,
    temperature: RwSignal<String>,
    allow_local_network: RwSignal<bool>,
    models: RwSignal<Vec<String>>,
    connection: RwSignal<Option<LocalAiConnectionResult>>,
    /// True once a model list request has completed (so "no models" only shows
    /// after we've actually looked).
    models_loaded: RwSignal<bool>,
    loading: RwSignal<bool>,
    /// Which button is currently busy, so only that one shows a spinner label.
    action: RwSignal<Option<&'static str>>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    // Action panel selections + result.
    project_id: RwSignal<String>,
    task_id: RwSignal<String>,
    instruction: RwSignal<String>,
    result: RwSignal<Option<(String, LocalAiWorkflowResponse)>>,
}

impl LocalAiView {
    fn new() -> Self {
        Self {
            settings: RwSignal::new(None),
            base_url: RwSignal::new(DEFAULT_BASE_URL.into()),
            default_model: RwSignal::new(String::new()),
            keep_alive: RwSignal::new(DEFAULT_KEEP_ALIVE.into()),
            temperature: RwSignal::new(String::new()),
            allow_local_network: RwSignal::new(false),
            models: RwSignal::new(Vec::new()),
            connection: RwSignal::new(None),
            models_loaded: RwSignal::new(false),
            loading: RwSignal::new(true),
            action: RwSignal::new(None),
            error: RwSignal::new(None),
            notice: RwSignal::new(None),
            project_id: RwSignal::new(String::new()),
            task_id: RwSignal::new(String::new()),
            instruction: RwSignal::new(String::new()),
            result: RwSignal::new(None),
        }
    }

    fn apply(self, settings: LocalAiSettings) {
        self.base_url.set(settings.base_url.clone());
        self.default_model
            .set(settings.default_model.clone().unwrap_or_default());
        self.keep_alive
            .set(settings.keep_alive.clone().unwrap_or_default());
        self.temperature.set(
            settings
                .temperature
                .map(|value| value.to_string())
                .unwrap_or_default(),
        );
        self.allow_local_network.set(settings.allow_local_network);
        self.settings.set(Some(settings));
    }

    fn load(self) {
        spawn_local(async move {
            match invoke::<LocalAiSettings>("get_local_ai_settings", json!({})).await {
                Ok(settings) => self.apply(settings),
                Err(error) => self
                    .error
                    .set(Some(format!("Could not load Local AI settings: {error}"))),
            }
            self.loading.set(false);
            // Quietly probe models on open so the dropdown is populated and the
            // status badge is accurate without the user clicking anything.
            self.refresh_models(true).await;
        });
    }

    async fn refresh_models(self, quiet: bool) {
        match invoke::<LocalAiModelListResult>("list_ollama_models", json!({})).await {
            Ok(result) => {
                self.models
                    .set(result.models.into_iter().map(|model| model.name).collect());
                self.models_loaded.set(true);
                // A successful tags call also confirms the connection.
                self.connection.set(Some(LocalAiConnectionResult {
                    connected: result.connected,
                    version: self.connection.get_untracked().and_then(|c| c.version),
                    error: result.error.clone(),
                }));
                if !quiet {
                    if result.connected {
                        self.notice.set(Some("Model list refreshed.".into()));
                    } else if let Some(error) = result.error {
                        self.error
                            .set(Some(friendly_error(&error, &self.base_url.get_untracked())));
                    }
                }
            }
            Err(error) if !quiet => self.error.set(Some(error)),
            Err(_) => {}
        }
    }

    fn start(self, action: &'static str) {
        self.action.set(Some(action));
        self.error.set(None);
        self.notice.set(None);
    }

    fn finish(self) {
        self.action.set(None);
    }
}

#[component]
pub fn LocalAiPage(state: AppState) -> impl IntoView {
    let view = LocalAiView::new();
    view.load();

    let test = move || {
        view.start("test");
        spawn_local(async move {
            match invoke::<LocalAiConnectionResult>("test_ollama_connection", json!({})).await {
                Ok(result) => {
                    if result.connected {
                        view.notice.set(Some(match result.version.clone() {
                            Some(version) => format!("Connected to Ollama {version}."),
                            None => "Connected to Ollama.".into(),
                        }));
                    } else if let Some(error) = result.error.clone() {
                        view.error
                            .set(Some(friendly_error(&error, &view.base_url.get())));
                    }
                    view.connection.set(Some(result));
                    // Refresh models on a successful test so the dropdown fills.
                    view.refresh_models(true).await;
                }
                Err(error) => view.error.set(Some(error)),
            }
            view.finish();
        });
    };

    let refresh_models = move || {
        view.start("models");
        spawn_local(async move {
            view.refresh_models(false).await;
            view.finish();
        });
    };

    let save = move || {
        view.start("save");
        let temperature = match parse_temperature(&view.temperature.get()) {
            Ok(value) => value,
            Err(error) => {
                view.error.set(Some(error));
                view.finish();
                return;
            }
        };
        let patch = LocalAiSettingsPatch {
            enabled: Some(true),
            base_url: Some(view.base_url.get().trim().to_owned()),
            default_model: Some(optional_text(view.default_model.get())),
            keep_alive: Some(optional_text(view.keep_alive.get())),
            temperature: Some(temperature),
            allow_local_network: Some(view.allow_local_network.get()),
        };
        spawn_local(async move {
            match invoke::<LocalAiSettings>("update_local_ai_settings", json!({ "patch": patch }))
                .await
            {
                Ok(settings) => {
                    view.apply(settings);
                    view.notice.set(Some("Local AI settings saved.".into()));
                }
                Err(error) => view.error.set(Some(error)),
            }
            view.finish();
        });
    };

    let reset = move || {
        if !confirmed("Reset Local AI settings to defaults?") {
            return;
        }
        view.start("reset");
        spawn_local(async move {
            match invoke::<LocalAiSettings>("reset_local_ai_settings", json!({})).await {
                Ok(settings) => {
                    view.apply(settings);
                    view.notice
                        .set(Some("Local AI settings reset to defaults.".into()));
                }
                Err(error) => view.error.set(Some(error)),
            }
            view.finish();
        });
    };

    // A workflow action that takes no extra args (plan / suggest / triage).
    let run_simple = move |label: &'static str, command: &'static str| {
        view.start(label);
        view.result.set(None);
        spawn_local(async move {
            run_workflow(view, label, command, json!({})).await;
            view.finish();
        });
    };

    let summarize = move || {
        let project_id = view.project_id.get();
        if project_id.is_empty() {
            view.error
                .set(Some("Select a project to summarize.".into()));
            return;
        }
        view.start("summarize");
        view.result.set(None);
        spawn_local(async move {
            run_workflow(
                view,
                "summarize",
                "summarize_project_with_local_ai",
                json!({ "projectId": project_id }),
            )
            .await;
            view.finish();
        });
    };

    let rewrite = move || {
        let task_id = view.task_id.get();
        if task_id.is_empty() {
            view.error.set(Some("Select a task to rewrite.".into()));
            return;
        }
        let instruction = view.instruction.get();
        let instruction = if instruction.trim().is_empty() {
            "Make it clearer and more actionable.".to_owned()
        } else {
            instruction
        };
        view.start("rewrite");
        view.result.set(None);
        spawn_local(async move {
            run_workflow(
                view,
                "rewrite",
                "rewrite_task_description_with_local_ai",
                json!({ "taskId": task_id, "instruction": instruction }),
            )
            .await;
            view.finish();
        });
    };

    let busy = move || view.action.get().is_some();

    view! {
        <PageHeader
            eyebrow="LOCAL ASSISTANT"
            title="Local AI"
            description="Run planning and triage with a local Ollama model. Nothing leaves your machine."
        >
            {move || {
                let (label, tone) = status_badge(
                    view.connection.get().as_ref(),
                    view.models_loaded.get(),
                    view.models.get().is_empty(),
                );
                view! { <Badge label=label tone=tone /> }
            }}
        </PageHeader>

        <Panel class="localai-privacy">
            <span class="localai-privacy-mark">
                <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
                    <rect x="3" y="11" width="18" height="10" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                </svg>
            </span>
            <p>
                "Local AI sends selected OpenMgmt context only to your configured Ollama server. \
                 The default URL is your own machine. Do not point this at a public server unless you trust it."
            </p>
        </Panel>

        {move || view.loading.get().then(|| view! { <LoadingState label="Loading Local AI settings…" /> })}
        {move || view.error.get().map(|message| view! {
            <div class="banner banner-error">
                <span>{message}</span>
                <button class="banner-dismiss" on:click=move |_| view.error.set(None)>"Dismiss"</button>
            </div>
        })}
        {move || view.notice.get().map(|message| view! {
            <div class="banner banner-notice">
                <span>{message}</span>
                <button class="banner-dismiss" on:click=move |_| view.notice.set(None)>"Dismiss"</button>
            </div>
        })}

        // Onboarding: only shown once we've probed and found a problem.
        {move || onboarding(view)}

        <div class="localai-layout">
            // --- Connection & settings -------------------------------------
            <Panel class="localai-config">
                <div class="section-head">
                    <div class="section-head-title"><h2>"Connection"</h2></div>
                    {move || {
                        let (label, tone) = status_badge(
                            view.connection.get().as_ref(),
                            view.models_loaded.get(),
                            view.models.get().is_empty(),
                        );
                        view! { <Badge label=label tone=tone /> }
                    }}
                </div>

                <FormField label="Provider">
                    <input type="text" value="Ollama" disabled=true />
                </FormField>
                <FormField label="Base URL" hint="Default: http://127.0.0.1:11434">
                    <input
                        type="url"
                        placeholder=DEFAULT_BASE_URL
                        prop:value=move || view.base_url.get()
                        on:input=move |event| view.base_url.set(event_target_value(&event))
                    />
                </FormField>

                {move || view.connection.get().and_then(|c| c.version).map(|version| view! {
                    <p class="localai-version">"Ollama version: "<strong>{version}</strong></p>
                })}

                <div class="localai-model-row">
                    <FormField label="Default model" hint="Used by every action below.">
                        <select
                            prop:value=move || view.default_model.get()
                            on:change=move |event| view.default_model.set(event_target_value(&event))
                        >
                            <option value="">{format!("Auto ({DEFAULT_MODEL})")}</option>
                            <For
                                each=move || model_options(&view.models.get(), &view.default_model.get())
                                key=|name| name.clone()
                                let:name
                            >
                                <option value=name.clone()>{name.clone()}</option>
                            </For>
                        </select>
                    </FormField>
                    <button
                        class="btn btn-ghost localai-refresh"
                        disabled=busy
                        on:click=move |_| refresh_models()
                    >
                        {move || if view.action.get() == Some("models") { "Refreshing…" } else { "Refresh models" }}
                    </button>
                </div>

                {move || model_note(&view.default_model.get()).map(|note| view! {
                    <p class="localai-model-note">{note}</p>
                })}

                <div class="form-row">
                    <FormField label="Keep alive" hint="e.g. 5m, or 0 to unload after each request.">
                        <input
                            type="text"
                            placeholder=DEFAULT_KEEP_ALIVE
                            prop:value=move || view.keep_alive.get()
                            on:input=move |event| view.keep_alive.set(event_target_value(&event))
                        />
                    </FormField>
                    <FormField label="Temperature" hint="Blank = model default. 0–1 is typical.">
                        <input
                            type="number"
                            step="0.1"
                            min="0"
                            placeholder="0.7"
                            prop:value=move || view.temperature.get()
                            on:input=move |event| view.temperature.set(event_target_value(&event))
                        />
                    </FormField>
                </div>

                <label class="form-check">
                    <input
                        type="checkbox"
                        prop:checked=move || view.allow_local_network.get()
                        on:change=move |event| view.allow_local_network.set(event_target_checked(&event))
                    />
                    <span>"Allow a private local-network address (advanced)"</span>
                </label>

                <div class="settings-actions">
                    <button class="btn btn-ghost" disabled=busy on:click=move |_| test()>
                        {move || if view.action.get() == Some("test") { "Testing…" } else { "Test connection" }}
                    </button>
                    <button class="btn btn-primary" disabled=busy on:click=move |_| save()>
                        {move || if view.action.get() == Some("save") { "Saving…" } else { "Save settings" }}
                    </button>
                    <button class="btn btn-subtle" disabled=busy on:click=move |_| reset()>
                        {move || if view.action.get() == Some("reset") { "Resetting…" } else { "Reset defaults" }}
                    </button>
                </div>
            </Panel>

            // --- Actions ---------------------------------------------------
            <Panel class="localai-actions">
                <div class="section-head"><div class="section-head-title"><h2>"Run a local action"</h2></div></div>
                <p class="settings-note">
                    "These read your data and return suggestions. They never change anything on their own."
                </p>

                <div class="localai-action-buttons">
                    <button class="btn btn-primary" disabled=busy on:click=move |_| run_simple("plan", "plan_day_with_local_ai")>
                        {move || if view.action.get() == Some("plan") { "Planning…" } else { "Plan my day" }}
                    </button>
                    <button class="btn btn-ghost" disabled=busy on:click=move |_| run_simple("suggest", "suggest_next_task_with_local_ai")>
                        {move || if view.action.get() == Some("suggest") { "Thinking…" } else { "Suggest next task" }}
                    </button>
                    <button class="btn btn-ghost" disabled=busy on:click=move |_| run_simple("triage", "triage_tasks_with_local_ai")>
                        {move || if view.action.get() == Some("triage") { "Triaging…" } else { "Triage tasks" }}
                    </button>
                </div>

                // Summarize project
                <div class="localai-action-row">
                    {move || {
                        let projects = state.snapshot.get().projects;
                        if projects.is_empty() {
                            view! { <p class="localai-empty">"No projects yet — create one to summarize it."</p> }.into_any()
                        } else {
                            view! {
                                <FormField label="Summarize project">
                                    <select
                                        prop:value=move || view.project_id.get()
                                        on:change=move |event| view.project_id.set(event_target_value(&event))
                                    >
                                        <option value="">"Select a project…"</option>
                                        <For each=move || state.snapshot.get().projects key=|p| p.id.clone() let:project>
                                            <option value=project.id.clone()>{project.name.clone()}</option>
                                        </For>
                                    </select>
                                </FormField>
                                <button class="btn btn-ghost localai-go" disabled=busy on:click=move |_| summarize()>
                                    {move || if view.action.get() == Some("summarize") { "Summarizing…" } else { "Summarize" }}
                                </button>
                            }.into_any()
                        }
                    }}
                </div>

                // Rewrite task description
                <div class="localai-action-row localai-action-rewrite">
                    {move || {
                        let tasks = state.snapshot.get().tasks;
                        if tasks.is_empty() {
                            view! { <p class="localai-empty">"No tasks yet — create one to rewrite its description."</p> }.into_any()
                        } else {
                            view! {
                                <FormField label="Rewrite task description">
                                    <select
                                        prop:value=move || view.task_id.get()
                                        on:change=move |event| view.task_id.set(event_target_value(&event))
                                    >
                                        <option value="">"Select a task…"</option>
                                        <For each=move || state.snapshot.get().tasks key=|t| t.id.clone() let:task>
                                            <option value=task.id.clone()>{task.title.clone()}</option>
                                        </For>
                                    </select>
                                </FormField>
                                <FormField label="Instruction (optional)">
                                    <input
                                        type="text"
                                        placeholder="Make it clearer and more actionable."
                                        prop:value=move || view.instruction.get()
                                        on:input=move |event| view.instruction.set(event_target_value(&event))
                                    />
                                </FormField>
                                <button class="btn btn-ghost localai-go" disabled=busy on:click=move |_| rewrite()>
                                    {move || if view.action.get() == Some("rewrite") { "Rewriting…" } else { "Suggest rewrite" }}
                                </button>
                                <p class="localai-empty">"The suggestion is shown below and is never saved automatically."</p>
                            }.into_any()
                        }
                    }}
                </div>

                // Result panel
                {move || view.result.get().map(|(label, response)| result_view(label, response))}
            </Panel>
        </div>
    }
}

/// Runs a workflow command, storing the `(label, response)` result or surfacing
/// an error. Workflow commands return a `LocalAiWorkflowResponse` whose own
/// `error` field is set when Ollama was unavailable, so we check both layers.
async fn run_workflow(
    view: LocalAiView,
    label: &'static str,
    command: &str,
    args: serde_json::Value,
) {
    match invoke::<LocalAiWorkflowResponse>(command, args).await {
        Ok(response) => {
            if let Some(error) = response.error.clone() {
                view.error
                    .set(Some(friendly_error(&error, &view.base_url.get())));
                // A fallback (e.g. suggest-next-task) is still useful to show.
                if response.fallback_used {
                    view.result.set(Some((label.to_owned(), response)));
                }
            } else {
                view.result.set(Some((label.to_owned(), response)));
            }
        }
        Err(error) => view
            .error
            .set(Some(friendly_error(&error, &view.base_url.get()))),
    }
}

fn result_view(label: String, response: LocalAiWorkflowResponse) -> impl IntoView {
    let model = response.model.clone().unwrap_or_else(|| "—".into());
    let fallback = response.fallback_used.then(|| {
        let title = response
            .fallback_task
            .as_ref()
            .map(|task| task.task.title.clone())
            .unwrap_or_default();
        view! {
            <p class="localai-fallback">
                "Ollama was unavailable, so this is a non-AI fallback by urgency"
                {(!title.is_empty()).then(|| format!(": {title}"))}"."
            </p>
        }
    });
    view! {
        <div class="localai-result">
            <div class="localai-result-head">
                <span class="localai-result-label">{action_title(&label)}</span>
                <span class="localai-result-model">"Model: "{model}</span>
            </div>
            {fallback}
            <pre class="localai-result-body">{response.content}</pre>
        </div>
    }
}

/// The onboarding card, shown only after a probe reveals a fixable problem.
fn onboarding(view: LocalAiView) -> Option<impl IntoView> {
    let connection = view.connection.get()?;
    if !connection.connected {
        // Not running / unreachable.
        return Some(
            view! {
                <Panel class="localai-onboard">
                    <h2 class="localai-onboard-title">"Ollama is not running"</h2>
                    <p>"Start Ollama, then test the connection. If it is not installed yet, get it from "
                        <span class="localai-link">"ollama.com"</span>"."</p>
                    {command_snippet("ollama serve")}
                    {command_snippet(format!("ollama pull {DEFAULT_MODEL}"))}
                </Panel>
            }
            .into_any(),
        );
    }
    if view.models_loaded.get() && view.models.get().is_empty() {
        // Connected but no models.
        return Some(
            view! {
                <Panel class="localai-onboard">
                    <h2 class="localai-onboard-title">"No local models installed"</h2>
                    <p>"Pull a lightweight model to get started, then refresh the model list."</p>
                    {command_snippet(format!("ollama pull {DEFAULT_MODEL}"))}
                </Panel>
            }
            .into_any(),
        );
    }
    None
}

/// A monospace command line with a Copy button (clipboard write is best-effort).
fn command_snippet(command: impl Into<String>) -> impl IntoView {
    let command = command.into();
    let to_copy = command.clone();
    view! {
        <div class="localai-snippet">
            <code>{command}</code>
            <button
                class="btn btn-subtle localai-copy"
                on:click=move |_| copy_to_clipboard(&to_copy)
            >"Copy"</button>
        </div>
    }
}

fn copy_to_clipboard(text: &str) {
    if let Some(clipboard) = web_sys::window().map(|window| window.navigator().clipboard()) {
        let _ = clipboard.write_text(text);
    }
}

/// Dropdown options: every fetched model, plus the saved default if it isn't
/// installed (so a configured-but-missing model is never silently dropped).
fn model_options(models: &[String], default_model: &str) -> Vec<String> {
    let mut options = models.to_vec();
    if !default_model.is_empty() && !options.iter().any(|name| name == default_model) {
        options.insert(0, default_model.to_owned());
    }
    options
}

/// Short capability note for the well-known recommended models.
fn model_note(model: &str) -> Option<&'static str> {
    match model {
        "qwen3:1.7b" => Some("Lightweight test model."),
        "llama3.2:3b" => Some("Good general assistant."),
        "qwen2.5-coder:3b" => Some("Code-focused."),
        _ => None,
    }
}

/// Status badge label + tone from the last probe.
fn status_badge(
    connection: Option<&LocalAiConnectionResult>,
    models_loaded: bool,
    no_models: bool,
) -> (&'static str, &'static str) {
    match connection {
        None => ("Not configured", "muted"),
        Some(connection) if connection.connected => {
            if models_loaded && no_models {
                ("No models found", "warn")
            } else {
                ("Connected", "done")
            }
        }
        Some(connection) => {
            let error = connection.error.as_deref().unwrap_or("");
            if error.contains("unavailable")
                || error.contains("refused")
                || error.contains("connect")
            {
                ("Not running", "warn")
            } else {
                ("Error", "warn")
            }
        }
    }
}

/// Maps a raw backend error into friendly, actionable copy.
fn friendly_error(error: &str, base_url: &str) -> String {
    if error.contains("disabled") {
        return "Local AI is turned off. Save settings to enable it.".into();
    }
    if error.contains("unavailable") || error.contains("refused") || error.contains("connect") {
        return format!(
            "Could not reach Ollama at {base_url}. Start Ollama (`ollama serve`), then test the connection."
        );
    }
    if error.contains("model") && error.contains("not found") {
        return format!("Model not found. Try `ollama pull {DEFAULT_MODEL}`, then refresh models.");
    }
    error.to_owned()
}

fn action_title(label: &str) -> &'static str {
    match label {
        "plan" => "Plan for today",
        "suggest" => "Next task",
        "triage" => "Triage",
        "summarize" => "Project summary",
        "rewrite" => "Suggested description",
        _ => "Result",
    }
}

/// Parse the temperature input: blank clears it, otherwise it must be a number.
/// Returns `Option<Option<f32>>`-ready inner value for the patch.
fn parse_temperature(value: &str) -> Result<Option<f32>, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed
        .parse::<f32>()
        .map(Some)
        .map_err(|_| "Temperature must be a number (e.g. 0.7) or left blank.".into())
}
