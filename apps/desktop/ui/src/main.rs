#[cfg(any(target_arch = "wasm32", test))]
mod sync;

#[cfg(target_arch = "wasm32")]
mod api;

#[cfg(target_arch = "wasm32")]
mod app;

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;

    console_error_panic_hook::set_once();

    let document = web_sys::window()
        .and_then(|window| window.document())
        .expect("browser document should exist");

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
        Some(root) => leptos::mount::mount_to(root, app::App).forget(),
        None => {
            web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(
                "[OpenMgmt] #app mount root missing; falling back to <body>",
            ));
            leptos::mount::mount_to_body(app::App);
        }
    }

    // Mounting succeeded: take down the boot fallback now that real UI is up.
    if let Some(boot) = document.get_element_by_id("om-boot") {
        boot.remove();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("OpenMgmt UI is built for wasm32-unknown-unknown with Trunk.");
}
