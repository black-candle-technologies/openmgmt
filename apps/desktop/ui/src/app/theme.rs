//! Appearance is webview-local, unlike the database-backed ranking settings.
use leptos::prelude::*;

pub fn initialize() -> RwSignal<bool> {
    let dark = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("openmgmt.theme").ok().flatten())
        .is_some_and(|v| v == "dark");
    let theme = RwSignal::new(dark);
    provide_context(theme);
    Effect::new(move |_| {
        let value = if theme.get() { "dark" } else { "light" };
        if let Some(window) = web_sys::window() {
            if let Some(root) = window.document().and_then(|d| d.document_element()) {
                let _ = root.set_attribute("data-theme", value);
            }
            if let Ok(Some(storage)) = window.local_storage() {
                let _ = storage.set_item("openmgmt.theme", value);
            }
        }
    });
    theme
}

#[component]
pub fn ThemeToggle() -> impl IntoView {
    let theme = expect_context::<RwSignal<bool>>();
    view! {
        <button class="btn btn-ghost" aria-label="Toggle light or dark appearance"
            aria-pressed=move || theme.get().to_string()
            on:click=move |_| theme.update(|dark| *dark = !*dark)>
            {move || if theme.get() { "Dark" } else { "Light" }}
        </button>
    }
}
