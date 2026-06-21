//! The global Local AI command chat: a right-docked overlay panel, available on
//! every page, that talks to the local Ollama-backed chat tools. It runs slash
//! commands, shows proposed write actions as confirm/cancel cards, and renders
//! read-tool results — never mutating data without an explicit confirm.
//!
//! All chat state lives in [`ChatState`] (created once in [`LocalAiChat`], which
//! is always mounted in the app shell) so a session persists across page
//! navigation for the lifetime of the app window.

use leptos::prelude::*;
use openmgmt_core::{
    LocalAiChatMessageRecord, LocalAiChatRole, LocalAiChatSession, LocalAiChatTurn,
    LocalAiConnectionResult, LocalAiContextScope, LocalAiModelListResult, LocalAiSettings,
    LocalAiToolCall, LocalAiToolCallStatus,
};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::Badge;
use super::state::{AppState, Page, invoke};

const DEFAULT_MODEL: &str = "qwen3:1.7b";

/// The slash commands shown in the inline hint (kept in sync with the backend
/// `slash_help`). Static because the set is fixed and documented.
const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "List commands"),
    ("/plan", "Plan today"),
    ("/board", "Current board state"),
    ("/tasks", "Active tasks"),
    ("/tasks blocked", "Blocked + waiting tasks"),
    ("/tasks overdue", "Overdue tasks"),
    ("/schedule today", "Today's schedule"),
    ("/schedule week", "This week's schedule"),
    ("/unscheduled", "Unscheduled tasks"),
    ("/models", "List local models"),
    ("/use <model>", "Switch model for this chat"),
    ("/create task <title>", "Propose a new task"),
    ("/complete task <id>", "Propose completing a task"),
    ("/start task <id>", "Propose starting a task"),
];

#[derive(Clone, Copy)]
struct ChatState {
    sessions: RwSignal<Vec<LocalAiChatSession>>,
    session_id: RwSignal<Option<String>>,
    session_title: RwSignal<String>,
    messages: RwSignal<Vec<LocalAiChatMessageRecord>>,
    proposals: RwSignal<Vec<LocalAiToolCall>>,
    models: RwSignal<Vec<String>>,
    model: RwSignal<String>,
    connected: RwSignal<Option<bool>>,
    input: RwSignal<String>,
    scope: RwSignal<LocalAiContextScope>,
    sending: RwSignal<bool>,
    busy_tool: RwSignal<Option<String>>,
    error: RwSignal<Option<String>>,
    initialized: RwSignal<bool>,
    expanded: RwSignal<bool>,
}

impl ChatState {
    fn new() -> Self {
        Self {
            sessions: RwSignal::new(Vec::new()),
            session_id: RwSignal::new(None),
            session_title: RwSignal::new("New chat".into()),
            messages: RwSignal::new(Vec::new()),
            proposals: RwSignal::new(Vec::new()),
            models: RwSignal::new(Vec::new()),
            model: RwSignal::new(String::new()),
            connected: RwSignal::new(None),
            input: RwSignal::new(String::new()),
            scope: RwSignal::new(LocalAiContextScope::Daily),
            sending: RwSignal::new(false),
            busy_tool: RwSignal::new(None),
            error: RwSignal::new(None),
            initialized: RwSignal::new(false),
            expanded: RwSignal::new(false),
        }
    }

    /// First-open setup: load the saved default model, probe Ollama, and adopt
    /// the most recent existing session (so messages survive a reopen).
    fn initialize(self) {
        self.initialized.set(true);
        spawn_local(async move {
            if let Ok(settings) =
                invoke::<LocalAiSettings>("get_local_ai_settings", json!({})).await
            {
                if let Some(model) = settings.default_model {
                    self.model.set(model);
                }
            }
            self.refresh_models().await;
            if let Ok(sessions) =
                invoke::<Vec<LocalAiChatSession>>("list_local_ai_chat_sessions", json!({})).await
            {
                if let Some(latest) = sessions.first().cloned() {
                    self.adopt_session(latest).await;
                }
                self.sessions.set(sessions);
            }
        });
    }

    async fn refresh_models(self) {
        match invoke::<LocalAiModelListResult>("list_ollama_models", json!({})).await {
            Ok(result) => {
                self.connected.set(Some(result.connected));
                self.models
                    .set(result.models.into_iter().map(|model| model.name).collect());
            }
            Err(_) => {
                // Fall back to a plain connection probe so the badge is right
                // even if the tags call shape changes.
                if let Ok(conn) =
                    invoke::<LocalAiConnectionResult>("test_ollama_connection", json!({})).await
                {
                    self.connected.set(Some(conn.connected));
                }
            }
        }
    }

    async fn adopt_session(self, session: LocalAiChatSession) {
        self.session_id.set(Some(session.id.clone()));
        self.session_title.set(session.title.clone());
        if let Some(model) = session.model.clone() {
            self.model.set(model);
        }
        self.load_messages(&session.id).await;
        self.proposals.set(Vec::new());
    }

    async fn load_messages(self, session_id: &str) {
        if let Ok(messages) = invoke::<Vec<LocalAiChatMessageRecord>>(
            "list_local_ai_chat_messages",
            json!({ "sessionId": session_id }),
        )
        .await
        {
            self.messages.set(messages);
        }
    }

    fn apply_turn(self, turn: LocalAiChatTurn) {
        self.session_id.set(Some(turn.session.id.clone()));
        self.session_title.set(turn.session.title.clone());
        if let Some(model) = turn.session.model.clone() {
            self.model.set(model);
        }
        self.messages.set(turn.messages);
        self.proposals.set(turn.proposed_tool_calls);
    }
}

/// Always-mounted host for the chat overlay. Renders nothing until opened.
#[component]
pub fn LocalAiChat(state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let chat = ChatState::new();

    // Initialize on first open, then keep state across opens/closes.
    Effect::new(move |_| {
        if state.chat_open.get() && !chat.initialized.get_untracked() {
            chat.initialize();
        }
    });

    let send = move || {
        let message = chat.input.get().trim().to_owned();
        if message.is_empty() || chat.sending.get() {
            return;
        }
        chat.sending.set(true);
        chat.error.set(None);
        chat.input.set(String::new());
        let model = {
            let value = chat.model.get();
            (!value.is_empty()).then_some(value)
        };
        // `input` is the top-level command arg (no casing conversion needed for a
        // lowercase name). Its NESTED fields are deserialized by serde directly,
        // so they must use the struct's snake_case field names — Tauri's
        // camelCase→snake_case conversion applies only to top-level args.
        let input = json!({
            "input": {
                "session_id": chat.session_id.get(),
                "message": message,
                "model": model,
                "context_scope": chat.scope.get(),
                "allow_write_proposals": true,
            }
        });
        spawn_local(async move {
            match invoke::<LocalAiChatTurn>("send_local_ai_chat_message", input).await {
                Ok(turn) => {
                    chat.apply_turn(turn);
                    // Pick up a freshly created session in the session list.
                    if let Ok(sessions) =
                        invoke::<Vec<LocalAiChatSession>>("list_local_ai_chat_sessions", json!({}))
                            .await
                    {
                        chat.sessions.set(sessions);
                    }
                }
                Err(error) => chat.error.set(Some(error)),
            }
            chat.sending.set(false);
        });
    };

    let confirm = move |call_id: String| {
        chat.busy_tool.set(Some(call_id.clone()));
        chat.error.set(None);
        spawn_local(async move {
            // Confirm marks the call ready; execute performs the known operation
            // and appends a tool-result message. A write needs both steps.
            let confirmed =
                invoke::<LocalAiToolCall>("confirm_local_ai_tool_call", json!({ "id": call_id }))
                    .await;
            let result = match confirmed {
                Ok(_) => {
                    invoke::<LocalAiToolCall>(
                        "execute_local_ai_tool_call",
                        json!({ "id": call_id }),
                    )
                    .await
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(_) => {
                    if let Some(session_id) = chat.session_id.get() {
                        chat.load_messages(&session_id).await;
                    }
                    chat.proposals.update(|calls| {
                        calls.retain(|call| call.id != call_id);
                    });
                    // A write executed: refresh the rest of the app.
                    state.refresh();
                }
                Err(error) => chat.error.set(Some(error)),
            }
            chat.busy_tool.set(None);
        });
    };

    let cancel = move |call_id: String| {
        chat.busy_tool.set(Some(call_id.clone()));
        spawn_local(async move {
            let _ =
                invoke::<LocalAiToolCall>("cancel_local_ai_tool_call", json!({ "id": call_id }))
                    .await;
            chat.proposals.update(|calls| {
                calls.retain(|call| call.id != call_id);
            });
            chat.busy_tool.set(None);
        });
    };

    let new_chat = move || {
        chat.error.set(None);
        chat.session_id.set(None);
        chat.session_title.set("New chat".into());
        chat.messages.set(Vec::new());
        chat.proposals.set(Vec::new());
        chat.input.set(String::new());
    };

    view! {
        {move || state.chat_open.get().then(|| {
            view! {
                <div class="chat-overlay" class:chat-expanded=move || chat.expanded.get()>
                    <ChatHeader chat state page />
                    <ChatSessions chat new_chat=Callback::new(move |_| new_chat()) />
                    <ChatBody chat confirm=Callback::new(confirm) cancel=Callback::new(cancel) />
                    <ChatFooter chat send=Callback::new(move |_| send()) />
                </div>
            }
        })}
    }
}

#[component]
fn ChatHeader(chat: ChatState, state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let refresh_models = move || spawn_local(async move { chat.refresh_models().await });
    view! {
        <header class="chat-head">
            <div class="chat-head-lead">
                <span class="chat-head-title">"OpenMgmt Local AI"</span>
                {move || {
                    let (label, tone) = status_badge(chat.connected.get(), &chat.model.get());
                    view! { <Badge label=label tone=tone /> }
                }}
            </div>
            <div class="chat-head-controls">
                <select
                    class="chat-model"
                    prop:value=move || chat.model.get()
                    on:change=move |event| chat.model.set(event_target_value(&event))
                >
                    <option value="">{format!("Auto ({DEFAULT_MODEL})")}</option>
                    <For
                        each=move || model_options(&chat.models.get(), &chat.model.get())
                        key=|name| name.clone()
                        let:name
                    >
                        <option value=name.clone()>{name.clone()}</option>
                    </For>
                </select>
                <button class="chat-icon-btn" title="Refresh models" on:click=move |_| refresh_models()>"⟳"</button>
                <button
                    class="chat-icon-btn"
                    title="Local AI settings"
                    on:click=move |_| { page.set(Page::LocalAi); }
                >"⚙"</button>
                <button
                    class="chat-icon-btn"
                    title=move || if chat.expanded.get() { "Collapse" } else { "Expand" }
                    on:click=move |_| chat.expanded.update(|value| *value = !*value)
                >{move || if chat.expanded.get() { "⤡" } else { "⤢" }}</button>
                <button class="chat-icon-btn" title="Close" on:click=move |_| state.chat_open.set(false)>"✕"</button>
            </div>
        </header>
    }
}

#[component]
fn ChatBody(chat: ChatState, confirm: Callback<String>, cancel: Callback<String>) -> impl IntoView {
    view! {
        <div class="chat-body">
            {move || chat.error.get().map(|message| view! {
                <div class="chat-error">
                    <span>{message}</span>
                    <button class="banner-dismiss" on:click=move |_| chat.error.set(None)>"Dismiss"</button>
                </div>
            })}

            {move || (chat.connected.get() == Some(false)).then(|| view! {
                <div class="chat-notice">
                    <strong>"Ollama is not running"</strong>
                    <p>"Start Ollama, then refresh models. Slash commands like /board and /tasks still work without a model."</p>
                    <code>"ollama serve"</code>
                </div>
            })}
            {move || (chat.connected.get() == Some(true) && chat.models.get().is_empty()).then(|| view! {
                <div class="chat-notice">
                    <strong>"No models installed"</strong>
                    <p>"Pull a lightweight model, then refresh."</p>
                    <code>{format!("ollama pull {DEFAULT_MODEL}")}</code>
                </div>
            })}

            {move || {
                let messages = chat.messages.get();
                if messages.is_empty() {
                    view! {
                        <div class="chat-empty">
                            <p class="chat-empty-title">"Ask about your work, or run a command."</p>
                            <p class="chat-empty-hint">"Try "<code>"/help"</code>", "<code>"/plan"</code>", or \"What should I work on next?\""</p>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <For each=move || chat.messages.get() key=|message| message.id.clone() let:message>
                            {message_view(message)}
                        </For>
                    }.into_any()
                }
            }}

            // Pending proposed write actions.
            <For each=move || chat.proposals.get() key=|call| call.id.clone() let:call>
                {proposal_card(call, chat, confirm, cancel)}
            </For>

            {move || chat.sending.get().then(|| view! {
                <div class="chat-thinking"><span class="spinner"></span>"Thinking…"</div>
            })}
        </div>
    }
}

#[component]
fn ChatFooter(chat: ChatState, send: Callback<()>) -> impl IntoView {
    let on_keydown = move |event: leptos::ev::KeyboardEvent| {
        if event.key() == "Enter" && !event.shift_key() {
            event.prevent_default();
            send.run(());
        }
    };
    view! {
        <div class="chat-foot">
            {move || chat.input.get().starts_with('/').then(|| view! {
                <div class="chat-slash">
                    <For
                        each=move || slash_matches(&chat.input.get())
                        key=|(cmd, _)| cmd.to_string()
                        let:item
                    >
                        <button
                            class="chat-slash-item"
                            on:click=move |_| chat.input.set(fill_command(item.0))
                        >
                            <code>{item.0}</code><span>{item.1}</span>
                        </button>
                    </For>
                </div>
            })}
            <div class="chat-foot-controls">
                <select
                    class="chat-scope"
                    title="Context sent to the model"
                    prop:value=move || scope_value(chat.scope.get())
                    on:change=move |event| chat.scope.set(scope_from_value(&event_target_value(&event)))
                >
                    <option value="daily">"Daily"</option>
                    <option value="schedule">"Schedule"</option>
                    <option value="project">"Project"</option>
                    <option value="full_summary">"Full summary"</option>
                    <option value="minimal">"Minimal"</option>
                </select>
                <span class="chat-slash-hint">"/ for commands"</span>
            </div>
            <div class="chat-input-row">
                <textarea
                    class="chat-input"
                    rows="2"
                    placeholder="Message Local AI…  (Enter to send, Shift+Enter for newline)"
                    prop:value=move || chat.input.get()
                    on:input=move |event| chat.input.set(event_target_value(&event))
                    on:keydown=on_keydown
                ></textarea>
                <button
                    class="btn btn-primary chat-send"
                    disabled=move || chat.sending.get() || chat.input.get().trim().is_empty()
                    on:click=move |_| send.run(())
                >{move || if chat.sending.get() { "…" } else { "Send" }}</button>
            </div>
        </div>
    }
}

#[component]
fn ChatSessions(chat: ChatState, new_chat: Callback<()>) -> impl IntoView {
    view! {
        <div class="chat-sessions">
            <button class="chat-session-new" on:click=move |_| new_chat.run(())>"+ New chat"</button>
            <div class="chat-session-list">
                <For each=move || chat.sessions.get() key=|session| session.id.clone() let:session>
                    {
                        let id = session.id.clone();
                        let active = move || chat.session_id.get().as_deref() == Some(id.as_str());
                        let open_id = session.id.clone();
                        view! {
                            <button
                                class="chat-session"
                                class:active=active
                                title=session.title.clone()
                                on:click=move |_| {
                                    let session_id = open_id.clone();
                                    spawn_local(async move {
                                        if let Ok(messages) = invoke::<Vec<LocalAiChatMessageRecord>>(
                                            "list_local_ai_chat_messages",
                                            json!({ "sessionId": session_id }),
                                        ).await {
                                            chat.session_id.set(Some(session_id.clone()));
                                            chat.messages.set(messages);
                                            chat.proposals.set(Vec::new());
                                        }
                                    });
                                }
                            >{session.title.clone()}</button>
                        }
                    }
                </For>
            </div>
        </div>
    }
}

/// One chat message bubble. Tool/JSON-ish content renders in a scrollable mono
/// block; everything else as wrapped text.
fn message_view(message: LocalAiChatMessageRecord) -> impl IntoView {
    let role_class = match message.role {
        LocalAiChatRole::User => "chat-msg chat-msg-user",
        LocalAiChatRole::Assistant => "chat-msg chat-msg-assistant",
        LocalAiChatRole::System => "chat-msg chat-msg-system",
        LocalAiChatRole::Tool => "chat-msg chat-msg-tool",
    };
    let label = match message.role {
        LocalAiChatRole::User => "You",
        LocalAiChatRole::Assistant => "Local AI",
        LocalAiChatRole::System => "System",
        LocalAiChatRole::Tool => "Result",
    };
    let mono = matches!(message.role, LocalAiChatRole::Tool) || looks_like_data(&message.content);
    let body = if mono {
        view! { <pre class="chat-msg-pre">{message.content.clone()}</pre> }.into_any()
    } else {
        view! { <div class="chat-msg-text">{message.content.clone()}</div> }.into_any()
    };
    view! {
        <div class=role_class>
            <span class="chat-msg-label">{label}</span>
            {body}
        </div>
    }
}

/// A proposed write action: tool name + arguments, with Confirm / Cancel.
fn proposal_card(
    call: LocalAiToolCall,
    chat: ChatState,
    confirm: Callback<String>,
    cancel: Callback<String>,
) -> impl IntoView {
    let confirm_id = call.id.clone();
    let cancel_id = call.id.clone();
    let busy_id = call.id.clone();
    let is_busy = Signal::derive(move || chat.busy_tool.get().as_deref() == Some(busy_id.as_str()));
    let confirmed = matches!(call.status, LocalAiToolCallStatus::Confirmed);
    let rows = argument_rows(&call.arguments_json);
    view! {
        <div class="chat-proposal">
            <div class="chat-proposal-head">
                <span class="chat-proposal-eyebrow">"PROPOSED ACTION"</span>
                <span class="chat-proposal-tool">{humanize_tool(&call.tool_name)}</span>
            </div>
            <div class="chat-proposal-args">
                <For each=move || rows.clone() key=|(k, _)| k.clone() let:row>
                    <div class="chat-proposal-arg">
                        <span class="chat-proposal-key">{row.0}</span>
                        <span class="chat-proposal-val">{row.1}</span>
                    </div>
                </For>
            </div>
            <div class="chat-proposal-actions">
                <button
                    class="btn btn-primary chat-proposal-confirm"
                    disabled=move || is_busy.get()
                    on:click=move |_| confirm.run(confirm_id.clone())
                >{move || if is_busy.get() { "Working…" } else if confirmed { "Run" } else { "Confirm" }}</button>
                <button
                    class="btn btn-ghost"
                    disabled=move || is_busy.get()
                    on:click=move |_| cancel.run(cancel_id.clone())
                >"Cancel"</button>
            </div>
        </div>
    }
}

// --- helpers ---------------------------------------------------------------

fn status_badge(connected: Option<bool>, model: &str) -> (&'static str, &'static str) {
    match connected {
        None => ("Connecting…", "muted"),
        Some(false) => ("Ollama not running", "warn"),
        Some(true) if model.is_empty() => ("Connected", "done"),
        Some(true) => ("Connected", "done"),
    }
}

fn model_options(models: &[String], selected: &str) -> Vec<String> {
    let mut options = models.to_vec();
    if !selected.is_empty() && !options.iter().any(|name| name == selected) {
        options.insert(0, selected.to_owned());
    }
    options
}

/// True when content looks like JSON/structured output we should render mono.
fn looks_like_data(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.starts_with("- ")
}

fn humanize_tool(name: &str) -> String {
    let mut text = name.replace('_', " ");
    if let Some(first) = text.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    text
}

/// Flatten a tool's JSON arguments object into display rows.
fn argument_rows(args: &serde_json::Value) -> Vec<(String, String)> {
    args.as_object()
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    let display = match value {
                        serde_json::Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    (humanize_tool(key), display)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn slash_matches(input: &str) -> Vec<(&'static str, &'static str)> {
    let needle = input.trim();
    SLASH_COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(needle) || needle == "/")
        .copied()
        .collect()
}

/// Turn a help entry like `/use <model>` into a ready-to-edit input prefix.
fn fill_command(command: &str) -> String {
    match command.split_once(" <") {
        Some((base, _)) => format!("{base} "),
        None => command.to_owned(),
    }
}

fn scope_value(scope: LocalAiContextScope) -> &'static str {
    match scope {
        LocalAiContextScope::Minimal => "minimal",
        LocalAiContextScope::Daily => "daily",
        LocalAiContextScope::Project => "project",
        LocalAiContextScope::Task => "task",
        LocalAiContextScope::Schedule => "schedule",
        LocalAiContextScope::FullSummary => "full_summary",
    }
}

fn scope_from_value(value: &str) -> LocalAiContextScope {
    match value {
        "minimal" => LocalAiContextScope::Minimal,
        "schedule" => LocalAiContextScope::Schedule,
        "project" => LocalAiContextScope::Project,
        "task" => LocalAiContextScope::Task,
        "full_summary" => LocalAiContextScope::FullSummary,
        _ => LocalAiContextScope::Daily,
    }
}
