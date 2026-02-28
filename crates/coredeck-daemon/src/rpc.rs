//! HTTP REST endpoints for third-party access
//!
//! Read-only endpoints always work. Mutating endpoints return 409 when a WS
//! client holds the lock. When no WS client is connected, mutating endpoints
//! transiently open the HID device for the duration of the request, then close
//! it so the keyboard works normally.

use coredeck_protocol::{
    AlertRequest, ApiError, BrightnessRequest, ClearAlertRequest, DaemonStatus,
    DisplayUpdateRequest, SetModeRequest,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::Arc;

use crate::DaemonState;
use crate::hid::HidManager;

/// Transiently open the HID device if it's not already open.
/// Returns `true` if we opened it (caller must close after use).
fn ensure_device_open(hid: &HidManager) -> Result<bool, String> {
    if hid.is_connected() {
        return Ok(false); // Already open (WS client holds it)
    }
    if !hid.is_device_available() {
        return Err("Device not available".into());
    }
    hid.open_device().map_err(|e| e.to_string())?;
    Ok(true) // We opened it transiently
}

/// GET /api/status — always available
pub async fn get_status(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    let status = state.device_status.read().await;
    let ws_locked = state.ws_client.lock().await.is_some();

    Json(DaemonStatus {
        device_available: status.available,
        device_connected: status.connected,
        device_name: status.device_name.clone(),
        firmware_version: status.firmware_version.clone(),
        device_mode: status.mode,
        device_yolo: status.yolo,
        ws_locked,
    })
}

/// POST /api/display
pub async fn post_display(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<DisplayUpdateRequest>,
) -> impl IntoResponse {
    if state.ws_client.lock().await.is_some() {
        return (StatusCode::CONFLICT, Json(ApiError { error: "device locked by WebSocket client".into() })).into_response();
    }

    let hid = state.hid.lock().await;
    let transient = match ensure_device_open(&hid) {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError { error: e })).into_response(),
    };

    let result = hid.send_display_update(&req.session, Some(req.task.as_str()).filter(|s| !s.is_empty()), Some(req.task2.as_str()).filter(|s| !s.is_empty()), &req.tabs, req.active);

    if transient { hid.close_device(); }

    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

/// POST /api/alert
pub async fn post_alert(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<AlertRequest>,
) -> impl IntoResponse {
    if state.ws_client.lock().await.is_some() {
        return (StatusCode::CONFLICT, Json(ApiError { error: "device locked by WebSocket client".into() })).into_response();
    }

    let hid = state.hid.lock().await;
    let transient = match ensure_device_open(&hid) {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError { error: e })).into_response(),
    };

    let result = hid.send_alert(req.tab, &req.session, &req.text, req.details.as_deref());

    if transient { hid.close_device(); }

    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

/// POST /api/alert/clear
pub async fn post_alert_clear(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<ClearAlertRequest>,
) -> impl IntoResponse {
    if state.ws_client.lock().await.is_some() {
        return (StatusCode::CONFLICT, Json(ApiError { error: "device locked by WebSocket client".into() })).into_response();
    }

    let hid = state.hid.lock().await;
    let transient = match ensure_device_open(&hid) {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError { error: e })).into_response(),
    };

    let result = hid.clear_alert(req.tab);

    if transient { hid.close_device(); }

    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

/// POST /api/brightness
pub async fn post_brightness(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<BrightnessRequest>,
) -> impl IntoResponse {
    if state.ws_client.lock().await.is_some() {
        return (StatusCode::CONFLICT, Json(ApiError { error: "device locked by WebSocket client".into() })).into_response();
    }

    let hid = state.hid.lock().await;
    let transient = match ensure_device_open(&hid) {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError { error: e })).into_response(),
    };

    let result = hid.set_brightness(req.level, req.save);

    if transient { hid.close_device(); }

    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

/// POST /api/mode
pub async fn post_mode(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<SetModeRequest>,
) -> impl IntoResponse {
    if state.ws_client.lock().await.is_some() {
        return (StatusCode::CONFLICT, Json(ApiError { error: "device locked by WebSocket client".into() })).into_response();
    }

    let hid = state.hid.lock().await;
    let transient = match ensure_device_open(&hid) {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError { error: e })).into_response(),
    };

    let result = hid.set_mode(req.mode);

    if transient { hid.close_device(); }

    match result {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

/// GET /api/version
pub async fn get_version(State(state): State<Arc<DaemonState>>) -> impl IntoResponse {
    if state.ws_client.lock().await.is_some() {
        return (StatusCode::CONFLICT, Json(ApiError { error: "device locked by WebSocket client".into() })).into_response();
    }

    let hid = state.hid.lock().await;
    let transient = match ensure_device_open(&hid) {
        Ok(t) => t,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(ApiError { error: e })).into_response(),
    };

    let version = hid.query_version();

    if transient { hid.close_device(); }

    Json(serde_json::json!({ "version": version })).into_response()
}
