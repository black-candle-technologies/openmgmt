#[cfg(target_arch = "wasm32")]
mod app;

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    // The pre-mount bootstrap placeholder (see index.html) keeps the window from
    // ever flashing blank/white. Remove it once we are about to mount so it does
    // not sit behind the app; if the mount panics it stays put, which is the
    // desired non-blank fallback.
    if let Some(boot) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("om-boot"))
    {
        boot.remove();
    }
    leptos::mount::mount_to_body(app::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("OpenMgmt UI is built for wasm32-unknown-unknown with Trunk.");
}
