//! The global Local AI agent chat: a right-docked overlay panel, available on
//! every page, that talks to the local Ollama-backed agent runtime. It speaks
//! natural language, runs OpenMgmt tools through a typed runtime, and is gated
//! by a per-chat access mode (read only / ask first / full access) — never
//! mutating data without the access the user has granted.
//!
//! All chat state lives in [`ChatState`] (created once in [`LocalAiChat`], which
//! is always mounted in the app shell) so a session persists across page
//! navigation for the lifetime of the app window.

use leptos::prelude::*;
use openmgmt_core::{
    LocalAiAccessMode, LocalAiChatMessageRecord, LocalAiChatRole, LocalAiChatSession,
    LocalAiChatTurn, LocalAiConnectionResult, LocalAiModelListResult, LocalAiSettings,
    LocalAiToolCall, LocalAiToolCallStatus,
};
use serde_json::json;
use wasm_bindgen_futures::spawn_local;

use super::components::Badge;
use super::state::{AppState, Page, confirmed, invoke};

const DEFAULT_MODEL: &str = "qwen3:1.7b";

const FULL_ACCESS_WARNING: &str =
    "Allow Local AI to create and update OpenMgmt data without confirmation in this chat?";

#[derive(Clone, Copy)]
struct ChatState {
    sessions: RwSignal<Vec<LocalAiChatSession>>,
    session_id: RwSignal<Option<String>>,
    session_title: RwSignal<String>,
    messages: RwSignal<Vec<LocalAiChatMessageRecord>>,
    proposals: RwSignal<Vec<LocalAiToolCall>>,
    models: RwSignal<Vec<String>>,
    model: RwSignal<String>,
    /// The single user-facing control: how much this chat may change.
    access_mode: RwSignal<LocalAiAccessMode>,
    connected: RwSignal<Option<bool>>,
    input: RwSignal<String>,
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
            access_mode: RwSignal::new(LocalAiAccessMode::AskBeforeWrite),
            connected: RwSignal::new(None),
            input: RwSignal::new(String::new()),
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
        self.access_mode.set(session.access_mode);
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
        self.access_mode.set(turn.session.access_mode);
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
        // `input` is the top-level command arg. Its NESTED fields are
        // deserialized by serde directly, so they must use the struct's
        // snake_case field names — Tauri's camelCase→snake_case conversion
        // applies only to top-level args.
        let input = json!({
            "input": {
                "session_id": chat.session_id.get(),
                "message": message,
                "model": model,
                "access_mode": access_mode_value(chat.access_mode.get()),
                "allow_write_proposals": true,
            }
        });
        spawn_local(async move {
            match invoke::<LocalAiChatTurn>("send_local_ai_chat_message", input).await {
                Ok(turn) => {
                    let mutated = turn.mutated;
                    chat.apply_turn(turn);
                    if let Ok(sessions) =
                        invoke::<Vec<LocalAiChatSession>>("list_local_ai_chat_sessions", json!({}))
                            .await
                    {
                        chat.sessions.set(sessions);
                    }
                    // The agent changed data in full-access mode: refresh the app.
                    if mutated {
                        state.refresh();
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
            // Confirm marks the call ready; execute performs the operation,
            // resolves any name selectors, and appends a result message.
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

    let confirm_all = move || {
        for call in chat.proposals.get() {
            confirm(call.id.clone());
        }
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
                    <ChatBody
                        chat
                        confirm=Callback::new(confirm)
                        cancel=Callback::new(cancel)
                        confirm_all=Callback::new(move |_| confirm_all())
                    />
                    <ChatFooter chat send=Callback::new(move |_| send()) />
                </div>
            }
        })}
    }
}

#[component]
fn ChatHeader(chat: ChatState, state: AppState, page: RwSignal<Page>) -> impl IntoView {
    let refresh_models = move || spawn_local(async move { chat.refresh_models().await });

    // Switching to full access asks once (not per message); cancelling re-asserts
    // the dropdown back to the real mode.
    let on_mode_change = move |event: leptos::ev::Event| {
        let requested = access_mode_from_value(&event_target_value(&event));
        let current = chat.access_mode.get();
        if requested == current {
            return;
        }
        if matches!(requested, LocalAiAccessMode::FullAccess) && !confirmed(FULL_ACCESS_WARNING) {
            chat.access_mode.set(current);
            return;
        }
        chat.access_mode.set(requested);
        if let Some(id) = chat.session_id.get() {
            spawn_local(async move {
                let _ = invoke::<LocalAiChatSession>(
                    "update_local_ai_chat_session_access_mode",
                    json!({ "id": id, "accessMode": access_mode_value(requested) }),
                )
                .await;
            });
        }
    };

    view! {
        <header class="chat-head">
            <div class="chat-head-lead">
                <span class="chat-head-title">"OpenMgmt Local AI"</span>
                {move || {
                    let (label, tone) = status_badge(chat.connected.get());
                    view! { <Badge label=label tone=tone /> }
                }}
                {move || {
                    let (label, tone) = access_mode_badge(chat.access_mode.get());
                    view! { <Badge label=label tone=tone /> }
                }}
            </div>
            <div class="chat-head-controls">
                <select
                    class="chat-access"
                    title=move || access_mode_hint(chat.access_mode.get())
                    prop:value=move || access_mode_value(chat.access_mode.get())
                    on:change=on_mode_change
                >
                    <option value="read_only">"Read only"</option>
                    <option value="ask_before_write">"Ask first"</option>
                    <option value="full_access">"Full access"</option>
                </select>
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
fn ChatBody(
    chat: ChatState,
    confirm: Callback<String>,
    cancel: Callback<String>,
    confirm_all: Callback<()>,
) -> impl IntoView {
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
                    <p>"Start Ollama, then refresh models to chat with the local assistant."</p>
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
                    let mode = chat.access_mode.get();
                    view! {
                        <div class="chat-empty">
                            <p class="chat-empty-title">"Ask about your work, or tell me what to change."</p>
                            <p class="chat-empty-hint">"Try \"What should I work on next?\" or \"Create a project called Website and add a task to draft the brief.\""</p>
                            <p class="chat-empty-mode">{access_mode_hint(mode)}</p>
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

            // Pending proposed write actions (Ask First mode).
            {move || {
                let proposals = chat.proposals.get();
                (proposals.len() > 1).then(|| view! {
                    <div class="chat-plan-actions">
                        <span class="chat-plan-label">{format!("{} proposed actions", proposals.len())}</span>
                        <button class="btn btn-primary btn-sm" on:click=move |_| confirm_all.run(())>"Confirm all"</button>
                    </div>
                })
            }}
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
                        let open_session = session.clone();
                        view! {
                            <button
                                class="chat-session"
                                class:active=active
                                title=session.title.clone()
                                on:click=move |_| {
                                    let session = open_session.clone();
                                    spawn_local(async move {
                                        chat.adopt_session(session).await;
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

/// One chat message bubble. User/assistant render as wrapped text (or mono for
/// structured fallback output); tool results render as compact action cards.
fn message_view(message: LocalAiChatMessageRecord) -> impl IntoView {
    if matches!(message.role, LocalAiChatRole::Tool) {
        let tool = message
            .metadata_json
            .as_ref()
            .and_then(|meta| meta.get("tool_name"))
            .and_then(|value| value.as_str())
            .map(humanize_tool)
            .unwrap_or_else(|| "Action".into());
        return view! {
            <div class="chat-msg chat-msg-tool">
                <span class="chat-msg-label">{format!("✓ {tool}")}</span>
                <div class="chat-msg-text">{message.content.clone()}</div>
            </div>
        }
        .into_any();
    }

    let role_class = match message.role {
        LocalAiChatRole::User => "chat-msg chat-msg-user",
        LocalAiChatRole::Assistant => "chat-msg chat-msg-assistant",
        LocalAiChatRole::System => "chat-msg chat-msg-system",
        LocalAiChatRole::Tool => unreachable!(),
    };
    let label = match message.role {
        LocalAiChatRole::User => "You",
        LocalAiChatRole::Assistant => "Local AI",
        LocalAiChatRole::System => "System",
        LocalAiChatRole::Tool => unreachable!(),
    };
    let body = if looks_like_data(&message.content) {
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
    .into_any()
}

/// A proposed write action: human-readable tool + arguments, with Confirm /
/// Cancel. Shown only in Ask First mode.
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

fn status_badge(connected: Option<bool>) -> (&'static str, &'static str) {
    match connected {
        None => ("Connecting…", "muted"),
        Some(false) => ("Ollama not running", "warn"),
        Some(true) => ("Connected", "done"),
    }
}

fn access_mode_value(mode: LocalAiAccessMode) -> &'static str {
    match mode {
        LocalAiAccessMode::ReadOnly => "read_only",
        LocalAiAccessMode::AskBeforeWrite => "ask_before_write",
        LocalAiAccessMode::FullAccess => "full_access",
    }
}

fn access_mode_from_value(value: &str) -> LocalAiAccessMode {
    match value {
        "read_only" => LocalAiAccessMode::ReadOnly,
        "full_access" => LocalAiAccessMode::FullAccess,
        _ => LocalAiAccessMode::AskBeforeWrite,
    }
}

/// Badge label + tone per mode: read-only looks safe, full access looks risky.
fn access_mode_badge(mode: LocalAiAccessMode) -> (&'static str, &'static str) {
    match mode {
        LocalAiAccessMode::ReadOnly => ("Read only", "done"),
        LocalAiAccessMode::AskBeforeWrite => ("Ask first", "muted"),
        LocalAiAccessMode::FullAccess => ("Full access", "warn"),
    }
}

fn access_mode_hint(mode: LocalAiAccessMode) -> &'static str {
    match mode {
        LocalAiAccessMode::ReadOnly => {
            "Read only: Local AI can read your workspace but won't change anything."
        }
        LocalAiAccessMode::AskBeforeWrite => {
            "Ask first: Local AI proposes changes and waits for your confirmation."
        }
        LocalAiAccessMode::FullAccess => {
            "Full access lets Local AI create and update OpenMgmt data without confirmation."
        }
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
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn humanize_tool(name: &str) -> String {
    let mut text = name.replace('_', " ");
    if let Some(first) = text.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    text
}

/// Flatten a tool's JSON arguments into display rows. Selector keys read as the
/// plain noun ("Organization", "Project") so cards aren't full of jargon.
fn argument_rows(args: &serde_json::Value) -> Vec<(String, String)> {
    args.as_object()
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    let display = match value {
                        serde_json::Value::String(text) => text.clone(),
                        other => other.to_string(),
                    };
                    let key = key
                        .strip_suffix("_selector")
                        .or_else(|| key.strip_suffix("_id"))
                        .unwrap_or(key);
                    (humanize_tool(key), display)
                })
                .collect()
        })
        .unwrap_or_default()
}
