#[cfg(any(target_arch = "wasm32", test))]
mod sync;

#[cfg(target_arch = "wasm32")]
mod api;

#[cfg(target_arch = "wasm32")]
mod app;

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::{JsCast, JsValue};

    console_error_panic_hook::set_once();
    web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] bootstrap start"));

    let document = web_sys::window()
        .and_then(|window| window.document())
        .expect("browser document should exist");
    web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] document ready"));

    // Mount into the dedicated #app root rather than directly into <body>. This
    // lets the #om-boot fallback (see index.html) stay on screen until the app
    // has actually mounted. `mount_to` builds the view synchronously, so if
    // `App` panics during mount we never reach the boot removal below — the
    // fallback (a dark "Loading OpenMgmt Board…" surface in board mode) remains
    // visible instead of a blank white/dark window.
    match document
        .get_element_by_id("app")
        .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
    {
        Some(root) => {
            web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] #app root found"));
            web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] mount start"));
            leptos::mount::mount_to(root, app::App).forget();
            web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] mount succeeded"));
        }
        None => {
            web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(
                "[OpenMgmt] #app mount root missing; falling back to <body>",
            ));
            web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] mount start"));
            leptos::mount::mount_to_body(app::App);
            web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] mount succeeded"));
        }
    }

    // Mounting succeeded: take down the boot fallback now that real UI is up.
    if let Some(boot) = document.get_element_by_id("om-boot") {
        boot.remove();
        web_sys::console::log_1(&JsValue::from_str("[OpenMgmt] boot removed"));
    } else {
        web_sys::console::warn_1(&JsValue::from_str("[OpenMgmt] #om-boot missing"));
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("OpenMgmt UI is built for wasm32-unknown-unknown with Trunk.");
}
