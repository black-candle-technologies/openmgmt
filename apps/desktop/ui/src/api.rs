use crate::sync::{SyncConnectionTestResult, SyncOnceResult};
use openmgmt_core::{SyncSettings, SyncSettingsPatch, SyncStatus};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    fn tauri_invoke(command: &str, args: JsValue) -> js_sys::Promise;
}

async fn invoke<T: DeserializeOwned>(command: &str, args: impl Serialize) -> Result<T, String> {
    let args = serde_wasm_bindgen::to_value(&args).map_err(|error| error.to_string())?;
    let value = JsFuture::from(tauri_invoke(command, args))
        .await
        .map_err(js_error_message)?;
    serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
}

fn js_error_message(value: JsValue) -> String {
    value.as_string().unwrap_or_else(|| {
        js_sys::JSON::stringify(&value)
            .ok()
            .and_then(|value| value.as_string())
            .unwrap_or_else(|| "Unknown Tauri invoke error".into())
    })
}

pub async fn get_sync_settings() -> Result<SyncSettings, String> {
    invoke("get_sync_settings", serde_json::json!({})).await
}

pub async fn update_sync_settings(patch: SyncSettingsPatch) -> Result<SyncSettings, String> {
    invoke(
        "update_sync_settings",
        serde_json::json!({ "patch": patch }),
    )
    .await
}

pub async fn get_sync_status() -> Result<SyncStatus, String> {
    invoke("get_sync_status", serde_json::json!({})).await
}

pub async fn sync_now() -> Result<SyncOnceResult, String> {
    invoke("sync_now", serde_json::json!({})).await
}

pub async fn test_sync_connection() -> Result<SyncConnectionTestResult, String> {
    invoke("test_sync_connection", serde_json::json!({})).await
}

pub async fn clear_sync_error() -> Result<SyncStatus, String> {
    invoke("clear_sync_error", serde_json::json!({})).await
}
