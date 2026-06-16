use crate::{
    api,
    sync::{SyncOnceResult, server_url_hint, status_label, sync_result_summary},
};
use chrono::{DateTime, Utc};
use leptos::prelude::*;
use openmgmt_core::{SyncConnectionState, SyncSettings, SyncSettingsPatch, SyncStatus};
use wasm_bindgen_futures::spawn_local;

#[derive(Clone, Copy)]
struct SyncViewState {
    settings: RwSignal<Option<SyncSettings>>,
    status: RwSignal<Option<SyncStatus>>,
    enabled: RwSignal<bool>,
    server_url: RwSignal<String>,
    device_name: RwSignal<String>,
    loading: RwSignal<bool>,
    action: RwSignal<Option<&'static str>>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    result: RwSignal<Option<SyncOnceResult>>,
}

impl SyncViewState {
    fn new() -> Self {
        Self {
            settings: RwSignal::new(None),
            status: RwSignal::new(None),
            enabled: RwSignal::new(false),
            server_url: RwSignal::new(String::new()),
            device_name: RwSignal::new(String::new()),
            loading: RwSignal::new(true),
            action: RwSignal::new(None),
            error: RwSignal::new(None),
            notice: RwSignal::new(None),
            result: RwSignal::new(None),
        }
    }

    fn load(self) {
        spawn_local(async move {
            self.reload().await;
        });
    }

    async fn reload(self) {
        self.loading.set(true);
        let settings = api::get_sync_settings().await;
        let status = api::get_sync_status().await;
        match settings {
            Ok(settings) => {
                self.enabled.set(settings.enabled);
                self.server_url
                    .set(settings.server_url.clone().unwrap_or_default());
                self.device_name.set(settings.device_name.clone());
                self.settings.set(Some(settings));
            }
            Err(error) => self
                .error
                .set(Some(format!("Could not load sync settings: {error}"))),
        }
        match status {
            Ok(status) => self.status.set(Some(status)),
            Err(error) => self
                .error
                .set(Some(format!("Could not load sync status: {error}"))),
        }
        self.loading.set(false);
    }

    async fn reload_status(self) {
        match api::get_sync_status().await {
            Ok(status) => self.status.set(Some(status)),
            Err(error) => self
                .error
                .set(Some(format!("Could not reload sync status: {error}"))),
        }
    }

    fn start_action(self, action: &'static str) {
        self.action.set(Some(action));
        self.error.set(None);
        self.notice.set(None);
    }

    fn finish_action(self) {
        self.action.set(None);
    }
}

#[component]
pub fn SyncPage() -> impl IntoView {
    let state = SyncViewState::new();
    state.load();

    view! {
        <header class="page-header">
            <div>
                <p class="eyebrow">"OPTIONAL SYNC"</p>
                <h1>"Sync"</h1>
                <p>"Connect this local workspace to an OpenMgmt server when you choose."</p>
            </div>
        </header>

        <section class="panel">
            <p>"OpenMgmt is local-first. Sync is optional. Manual sync pushes local changes and pulls remote changes."</p>
        </section>

        {move || state.loading.get().then(|| view! { <div class="loading-bar">"Loading sync settings..."</div> })}
        {move || state.error.get().map(|message| view! {
            <div class="feedback error">
                <span>{message}</span>
                <button on:click=move |_| state.error.set(None)>"Dismiss"</button>
            </div>
        })}
        {move || state.notice.get().map(|message| view! {
            <div class="feedback notice">
                <span>{message}</span>
                <button on:click=move |_| state.notice.set(None)>"Dismiss"</button>
            </div>
        })}

        <section class="cards">
            <form class="panel" on:submit=move |event| {
                event.prevent_default();
                state.start_action("save");
                let patch = SyncSettingsPatch {
                    enabled: Some(state.enabled.get()),
                    server_url: Some(optional_text(state.server_url.get())),
                    device_name: Some(state.device_name.get()),
                    ..SyncSettingsPatch::default()
                };
                spawn_local(async move {
                    match api::update_sync_settings(patch).await {
                        Ok(settings) => {
                            state.settings.set(Some(settings));
                            state.notice.set(Some("Sync settings saved.".into()));
                        }
                        Err(error) => state.error.set(Some(format!("Could not save sync settings: {error}"))),
                    }
                    state.reload_status().await;
                    state.finish_action();
                });
            }>
                <div class="section-title">
                    <div>
                        <p class="eyebrow">"CONFIGURATION"</p>
                        <h2>"Device settings"</h2>
                    </div>
                </div>
                <label class="field">
                    <span>"Enable sync"</span>
                    <input
                        type="checkbox"
                        prop:checked=move || state.enabled.get()
                        on:change=move |event| state.enabled.set(event_target_checked(&event))
                    />
                </label>
                <label class="field">
                    <span>"Server URL"</span>
                    <input
                        type="url"
                        placeholder="http://127.0.0.1:8787"
                        prop:value=move || state.server_url.get()
                        on:input=move |event| state.server_url.set(event_target_value(&event))
                    />
                    {move || server_url_hint(&state.server_url.get()).map(|hint| view! {
                        <small class="field-hint warning">{hint}</small>
                    })}
                </label>
                <label class="field">
                    <span>"Device name"</span>
                    <input
                        type="text"
                        placeholder="Local device"
                        prop:value=move || state.device_name.get()
                        on:input=move |event| state.device_name.set(event_target_value(&event))
                    />
                </label>
                <button class="primary" disabled=move || state.action.get().is_some()>
                    {move || if state.action.get() == Some("save") { "Saving..." } else { "Save settings" }}
                </button>
            </form>

            <section class="panel">
                <div class="section-title">
                    <div>
                        <p class="eyebrow">"CONNECTION"</p>
                        <h2>"Sync status"</h2>
                    </div>
                    {move || {
                        let syncing = state.action.get() == Some("sync");
                        let status = state.status.get();
                        let connection_state = if syncing {
                            SyncConnectionState::Syncing
                        } else {
                            status.as_ref().map(|status| status.state).unwrap_or(SyncConnectionState::Disabled)
                        };
                        view! {
                            <span class=format!("status {}", status_class(connection_state))>
                                {status_label(connection_state)}
                            </span>
                        }
                    }}
                </div>
                {move || state.status.get().map(|status| view! {
                    <dl class="sync-details">
                        <div><dt>"Device name"</dt><dd>{status.device_name}</dd></div>
                        <div><dt>"Unsynced events"</dt><dd>{status.unsynced_event_count}</dd></div>
                        <div><dt>"Server"</dt><dd>{status.server_url.unwrap_or_else(|| "Not configured".into())}</dd></div>
                        <div><dt>"Last successful"</dt><dd>{format_sync_time(status.last_successful_sync_at)}</dd></div>
                        <div><dt>"Last attempted"</dt><dd>{format_sync_time(status.last_attempted_sync_at)}</dd></div>
                    </dl>
                })}
                {move || state.status.get().and_then(|status| status.last_error).map(|error| view! {
                    <div class="feedback error"><span>{error}</span></div>
                })}
                <div class="actions">
                    <button class="ghost" disabled=move || state.action.get().is_some() on:click=move |_| {
                        state.start_action("test");
                        spawn_local(async move {
                            match api::test_sync_connection().await {
                                Ok(result) => {
                                    let message = result.message.unwrap_or_else(|| {
                                        if result.ok { "Connection test succeeded.".into() } else { "Connection test failed.".into() }
                                    });
                                    if result.ok {
                                        state.notice.set(Some(message));
                                    } else {
                                        state.error.set(Some(message));
                                    }
                                }
                                Err(error) => state.error.set(Some(format!("Connection test failed: {error}"))),
                            }
                            state.reload_status().await;
                            state.finish_action();
                        });
                    }>
                        {move || if state.action.get() == Some("test") { "Testing..." } else { "Test connection" }}
                    </button>
                    <button class="primary" disabled=move || state.action.get().is_some() on:click=move |_| {
                        state.start_action("sync");
                        spawn_local(async move {
                            match api::sync_now().await {
                                Ok(result) => {
                                    state.notice.set(Some(sync_result_summary(&result)));
                                    state.result.set(Some(result));
                                }
                                Err(error) => state.error.set(Some(format!("Sync failed: {error}"))),
                            }
                            state.reload_status().await;
                            state.finish_action();
                        });
                    }>
                        {move || if state.action.get() == Some("sync") { "Syncing..." } else { "Sync now" }}
                    </button>
                    <button class="ghost" disabled=move || state.action.get().is_some() on:click=move |_| {
                        state.start_action("clear");
                        spawn_local(async move {
                            match api::clear_sync_error().await {
                                Ok(status) => {
                                    state.status.set(Some(status));
                                    state.notice.set(Some("Sync error cleared.".into()));
                                }
                                Err(error) => state.error.set(Some(format!("Could not clear sync error: {error}"))),
                            }
                            state.finish_action();
                        });
                    }>"Clear error"</button>
                </div>
                {move || state.result.get().map(|result| view! {
                    <p class="device-id">{sync_result_summary(&result)}</p>
                })}
            </section>
        </section>
    }
}

fn optional_text(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn format_sync_time(value: Option<DateTime<Utc>>) -> String {
    let Some(value) = value else {
        return "Never".into();
    };
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
        value.timestamp_millis() as f64,
    ));
    date.to_locale_string("en-US", &wasm_bindgen::JsValue::UNDEFINED)
        .into()
}

fn status_class(state: SyncConnectionState) -> &'static str {
    match state {
        SyncConnectionState::Disabled => "disabled",
        SyncConnectionState::NotConfigured => "not-configured",
        SyncConnectionState::Ready => "ready",
        SyncConnectionState::Syncing => "syncing",
        SyncConnectionState::Error => "error",
    }
}
