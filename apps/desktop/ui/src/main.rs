#[cfg(any(target_arch = "wasm32", test))]
mod sync;

#[cfg(target_arch = "wasm32")]
mod api;

#[cfg(target_arch = "wasm32")]
mod app;

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("OpenMgmt UI is built for wasm32-unknown-unknown with Trunk.");
}
