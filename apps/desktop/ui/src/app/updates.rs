use super::state::invoke;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use serde_json::{Value, json};
use wasm_bindgen_futures::spawn_local;

#[derive(Clone, Copy)]
pub struct Updates {
    pub status: RwSignal<Value>,
    pub busy: RwSignal<bool>,
    pub dismissed: RwSignal<bool>,
}
impl Updates {
    pub fn initialize() {
        let updates = Self {
            status: RwSignal::new(Value::Null),
            busy: RwSignal::new(false),
            dismissed: RwSignal::new(false),
        };
        provide_context(updates);
        updates.check(false);
    }
    pub fn check(self, force: bool) {
        self.busy.set(true);
        spawn_local(async move {
            match invoke::<Value>("check_updates", json!({"force": force})).await {
                Ok(v) => {
                    self.status.set(v);
                    self.dismissed.set(false);
                }
                Err(e) => self.status.set(json!({"error": e})),
            }
            self.busy.set(false);
        });
    }
}
#[component]
pub fn UpdateBanner() -> impl IntoView {
    let updates = expect_context::<Updates>();
    let installing = RwSignal::new(false);
    let progress = RwSignal::new(String::new());
    view! {
        {move || (!updates.dismissed.get() && updates.status.get()["available"].is_object()).then(|| view! {
            <div class="banner banner-notice" role="status">
                <span>{move || format!("Update available — OpenMGMT {}. Download and install?", updates.status.get()["available"]["version"].as_str().unwrap_or(""))}</span>
                <button class="btn btn-primary" disabled=move || installing.get() on:click=move |_| {
                    installing.set(true);
                    spawn_local(async move {
                        let interval = Interval::new(750, move || spawn_local(async move {
                            if let Ok(v) = invoke::<Value>("updater_status", json!({})).await {
                                progress.set(format!("{} / {} bytes", v["downloaded"], v["total"]));
                            }
                        }));
                        match invoke::<()>("install_update", json!({})).await {
                            Ok(()) => { progress.set("Installer opened. Complete installation in the OS installer.".into()); }
                            Err(e) => progress.set(e),
                        }
                        drop(interval);
                        installing.set(false);
                    });
                }>"Download and install"</button>
                <button class="btn btn-ghost" disabled=move || installing.get() on:click=move |_| updates.dismissed.set(true)>"Later"</button>
                <span>{move || progress.get()}</span>
            </div>
        })}
    }
}
#[component]
pub fn UpdatesSettings() -> impl IntoView {
    let updates = expect_context::<Updates>();
    view! {
        <section class="panel"><h2>"Updates"</h2>
            <p>{concat!("Current version: ", env!("CARGO_PKG_VERSION"))}</p>
            <button class="btn btn-subtle" disabled=move || updates.busy.get() on:click=move |_| updates.check(true)>"Check now"</button>
            <p>{move || {
                let v = updates.status.get();
                if updates.busy.get() { "Checking…".into() }
                else if let Some(error) = v["error"].as_str() { error.to_owned() }
                else if let Some(time) = v["last_check"].as_i64() {
                    format!("Last checked: {}", chrono::DateTime::from_timestamp(time, 0).map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_default())
                } else { "Not checked yet".into() }
            }}</p>
        </section>
    }
}
#[component]
pub fn DispatchSettings() -> impl IntoView {
    let url = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());
    spawn_local(async move {
        match invoke::<String>("dispatch_settings", json!({"baseUrl": null})).await {
            Ok(v) => url.set(v),
            Err(e) => message.set(e),
        }
    });
    view! {
        <details class="panel"><summary>"Advanced — Dispatch service"</summary>
            <p>"Your account bearer token is sent to this HTTPS service. Only use a server you trust."</p>
            <label>"Base URL"<input type="url" prop:value=move || url.get() on:input=move |e| url.set(event_target_value(&e)) /></label>
            <button class="btn btn-subtle" on:click=move |_| spawn_local(async move {
                match invoke::<String>("dispatch_settings", json!({"baseUrl": url.get_untracked()})).await {
                    Ok(_) => message.set("Dispatch URL saved.".into()), Err(e) => message.set(e),
                }
            })>"Save URL"</button><p role="status">{move || message.get()}</p>
        </details>
    }
}
