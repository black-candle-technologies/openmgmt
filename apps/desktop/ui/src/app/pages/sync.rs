use crate::app::components::{Badge, FormField, LoadingState, PageHeader, Panel};
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

    // Action handlers. Each preserves the existing Tauri command wiring and only
    // toggles the shared `action` signal so buttons can show busy/disabled state.
    let save = move || {
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
                Err(error) => state
                    .error
                    .set(Some(format!("Could not save sync settings: {error}"))),
            }
            state.reload_status().await;
            state.finish_action();
        });
    };

    let test = move || {
        state.start_action("test");
        spawn_local(async move {
            match api::test_sync_connection().await {
                Ok(result) => {
                    let message = result.message.unwrap_or_else(|| {
                        if result.ok {
                            "Connection test succeeded.".into()
                        } else {
                            "Connection test failed.".into()
                        }
                    });
                    if result.ok {
                        state.notice.set(Some(message));
                    } else {
                        state.error.set(Some(message));
                    }
                }
                Err(error) => state
                    .error
                    .set(Some(format!("Connection test failed: {error}"))),
            }
            state.reload_status().await;
            state.finish_action();
        });
    };

    let sync = move || {
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
    };

    let clear = move || {
        state.start_action("clear");
        spawn_local(async move {
            match api::clear_sync_error().await {
                Ok(status) => {
                    state.status.set(Some(status));
                    state.notice.set(Some("Sync error cleared.".into()));
                }
                Err(error) => state
                    .error
                    .set(Some(format!("Could not clear sync error: {error}"))),
            }
            state.finish_action();
        });
    };

    view! {
        <PageHeader
            eyebrow="OPTIONAL SYNC"
            title="Sync"
            description="Connect this local workspace to an OpenMgmt server when you choose."
        >
            {move || {
                let status = state.status.get();
                let (label, tone) = header_badge(
                    state.enabled.get(),
                    status.as_ref(),
                    state.error.get().is_some(),
                );
                view! { <Badge label=label tone=tone /> }
            }}
        </PageHeader>

        <Panel class="sync-intro-panel">
            <div class="sync-intro">
                <span class="sync-intro-mark">
                    <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true">
                        <rect x="3" y="11" width="18" height="10" rx="2"></rect>
                        <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                    </svg>
                </span>
                <p>
                    "OpenMgmt is local-first. Sync is optional. Manual sync pushes local changes and pulls remote changes."
                </p>
            </div>
        </Panel>

        {move || state.loading.get().then(|| view! { <LoadingState label="Loading sync settings…" /> })}
        {move || state.error.get().map(|message| view! {
            <div class="banner banner-error">
                <span>{message}</span>
                <button class="banner-dismiss" on:click=move |_| state.error.set(None)>"Dismiss"</button>
            </div>
        })}
        {move || state.notice.get().map(|message| view! {
            <div class="banner banner-notice">
                <span>{message}</span>
                <button class="banner-dismiss" on:click=move |_| state.notice.set(None)>"Dismiss"</button>
            </div>
        })}

        <div class="sync-layout">
            <Panel class="sync-config-panel">
                <div class="sync-panel-head">
                    <div>
                        <p class="eyebrow">"CONFIGURATION"</p>
                        <h2 class="sync-panel-title">"Device settings"</h2>
                    </div>
                </div>
                <div class="sync-config">
                    <label class="sync-toggle">
                        <input
                            type="checkbox"
                            prop:checked=move || state.enabled.get()
                            on:change=move |event| state.enabled.set(event_target_checked(&event))
                        />
                        <span class="sync-toggle-track"><span class="sync-toggle-thumb"></span></span>
                        <span class="sync-toggle-text">
                            <span class="sync-toggle-title">"Enable sync"</span>
                            <span class="sync-toggle-hint">"Turn on to sync this workspace with a server."</span>
                        </span>
                    </label>
                    <div class="sync-config-fields">
                        <FormField label="Server URL">
                            <input
                                type="url"
                                placeholder="http://127.0.0.1:8787"
                                prop:value=move || state.server_url.get()
                                on:input=move |event| state.server_url.set(event_target_value(&event))
                            />
                            <span class="form-field-hint">"Example: http://127.0.0.1:8787"</span>
                            {move || server_url_hint(&state.server_url.get()).map(|hint| view! {
                                <span class="form-field-hint form-field-hint-warn">{hint}</span>
                            })}
                        </FormField>
                        <FormField label="Device name" hint="Used to identify this machine during sync.">
                            <input
                                type="text"
                                placeholder="Local device"
                                prop:value=move || state.device_name.get()
                                on:input=move |event| state.device_name.set(event_target_value(&event))
                            />
                        </FormField>
                    </div>
                    <div class="sync-form-actions">
                        <button
                            class="btn btn-primary"
                            disabled=move || state.action.get().is_some()
                            on:click=move |_| save()
                        >
                            {move || if state.action.get() == Some("save") { "Saving…" } else { "Save settings" }}
                        </button>
                    </div>
                </div>
            </Panel>

            <Panel class="sync-status-panel">
                <div class="sync-panel-head">
                    <div>
                        <p class="eyebrow">"CONNECTION"</p>
                        <h2 class="sync-panel-title">"Sync status"</h2>
                    </div>
                    {move || {
                        let conn = if state.action.get() == Some("sync") {
                            SyncConnectionState::Syncing
                        } else {
                            state
                                .status
                                .get()
                                .map(|status| status.state)
                                .unwrap_or(SyncConnectionState::Disabled)
                        };
                        view! { <Badge label=status_label(conn) tone=status_badge_tone(conn) /> }
                    }}
                </div>

                {move || state.status.get().map(|status| {
                    let unsynced = status.unsynced_event_count.max(0);
                    let unsynced_class = if unsynced > 0 {
                        "metric metric-caution"
                    } else {
                        "metric metric-info"
                    };
                    let server = status.server_url.clone();
                    view! {
                        <div class="sync-metrics">
                            <div class=unsynced_class>
                                <span class="metric-label">"Unsynced events"</span>
                                <strong class="metric-value">{unsynced}</strong>
                            </div>
                            <div class="sync-fact">
                                <span class="sync-fact-label">"Device"</span>
                                <span class="sync-fact-value">{status.device_name.clone()}</span>
                            </div>
                            <div class="sync-fact">
                                <span class="sync-fact-label">"Server"</span>
                                {match server {
                                    Some(url) => view! { <span class="sync-fact-value">{url}</span> }.into_any(),
                                    None => view! { <span class="sync-fact-value is-muted">"Not configured"</span> }.into_any(),
                                }}
                            </div>
                        </div>
                        <div class="sync-rows">
                            <div class="sync-row">
                                <span class="sync-row-label">"Last successful"</span>
                                <span class="sync-row-value">{format_sync_time(status.last_successful_sync_at)}</span>
                            </div>
                            <div class="sync-row">
                                <span class="sync-row-label">"Last attempted"</span>
                                <span class="sync-row-value">{format_sync_time(status.last_attempted_sync_at)}</span>
                            </div>
                        </div>
                    }
                })}

                {move || state.status.get().and_then(|status| status.last_error).map(|error| view! {
                    <div class="banner banner-error"><span>{error}</span></div>
                })}

                <div class="sync-actions">
                    <button
                        class="btn btn-ghost"
                        disabled=move || state.action.get().is_some()
                        on:click=move |_| test()
                    >
                        {move || if state.action.get() == Some("test") { "Testing…" } else { "Test connection" }}
                    </button>
                    <button
                        class="btn btn-primary"
                        disabled=move || state.action.get().is_some()
                        on:click=move |_| sync()
                    >
                        {move || if state.action.get() == Some("sync") { "Syncing…" } else { "Sync now" }}
                    </button>
                    {move || {
                        let has_error = state.error.get().is_some()
                            || state.status.get().and_then(|status| status.last_error).is_some();
                        has_error.then(|| view! {
                            <button
                                class="btn btn-danger-soft"
                                disabled=move || state.action.get().is_some()
                                on:click=move |_| clear()
                            >
                                {move || if state.action.get() == Some("clear") { "Clearing…" } else { "Clear error" }}
                            </button>
                        })
                    }}
                </div>

                {move || state.result.get().map(|result| view! {
                    <p class="sync-result">{sync_result_summary(&result)}</p>
                })}
            </Panel>
        </div>
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
        value.timestamp_millis() as f64
    ));
    date.to_locale_string("en-US", &wasm_bindgen::JsValue::UNDEFINED)
        .into()
}

/// Tone for the per-panel connection badge (mirrors `SyncConnectionState`).
fn status_badge_tone(state: SyncConnectionState) -> &'static str {
    match state {
        SyncConnectionState::Disabled => "muted",
        SyncConnectionState::NotConfigured => "neutral",
        SyncConnectionState::Ready => "ready",
        SyncConnectionState::Syncing => "active",
        SyncConnectionState::Error => "warn",
    }
}

/// Label + tone for the page-header status badge: Disabled / Ready / Connected /
/// Error, derived from the enabled flag and the last-known status.
fn header_badge(
    enabled: bool,
    status: Option<&SyncStatus>,
    has_error: bool,
) -> (&'static str, &'static str) {
    let status_error = status
        .map(|status| status.last_error.is_some())
        .unwrap_or(false);
    if has_error || status_error {
        ("Error", "warn")
    } else if !enabled {
        ("Disabled", "muted")
    } else if status
        .and_then(|status| status.last_successful_sync_at)
        .is_some()
    {
        ("Connected", "done")
    } else {
        ("Ready", "ready")
    }
}
