# Shared Type Reference

JSON schemas for all types used across the HTTP REST and WebSocket APIs. These types are defined in the `agentdeck-protocol` crate.

## DaemonStatus

Returned by `GET /api/status`.

```json
{
  "device_available": true,
  "device_connected": true,
  "device_name": "Agent Deck",
  "firmware_version": "1.0.0",
  "device_mode": "Default",
  "device_yolo": false,
  "ws_locked": false
}
```

| Field | Type | Description |
|-------|------|-------------|
| `device_available` | boolean | USB device is physically present (enumerated on the bus) |
| `device_connected` | boolean | HID interface is open and communicating |
| `device_name` | string \| null | Device product name (if available) |
| `firmware_version` | string \| null | Firmware version string (if connected) |
| `device_mode` | [DeviceMode](#devicemode) | Current operating mode |
| `device_yolo` | boolean | YOLO toggle switch state |
| `ws_locked` | boolean | Whether a WebSocket client holds the exclusive lock |

## DisplayUpdateRequest

Request body for `POST /api/display`.

```json
{
  "session": "my-project",
  "task": "Reading files",
  "task2": "",
  "tabs": [0, 2, 1],
  "active": 1
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `session` | string | required | Session/project name (max 128 bytes) |
| `task` | string | `""` | Current task description (max 128 bytes) |
| `task2` | string | `""` | Second task line, pre-split (max 128 bytes) |
| `tabs` | u8[] | `[]` | Tab state values (max 16 entries). See [tab states](#tab-states). |
| `active` | integer | `0` | Index into `tabs` for the active tab |

## DisplayUpdate

Used as the JSON payload for the WebSocket `UpdateDisplay` command (tag `0x01`). Same structure as `DisplayUpdateRequest` but `task2` is omitted when empty.

```json
{
  "session": "my-project",
  "task": "Reading files",
  "tabs": [0, 2, 1],
  "active": 1
}
```

| Field | Type | Description |
|-------|------|-------------|
| `session` | string | Session/project name |
| `task` | string | Current task description |
| `task2` | string | Second task line (omitted from JSON when empty) |
| `tabs` | u8[] | Tab state values |
| `active` | integer | Active tab index |

## AlertRequest

Request body for `POST /api/alert` and payload for WS `Alert` command (tag `0x08`).

```json
{
  "tab": 0,
  "session": "my-project",
  "text": "Task complete",
  "details": "All 42 tests passed"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `tab` | integer | Tab index (0-15) |
| `session` | string | Session name for this tab (max 128 bytes) |
| `text` | string | Alert text (max 128 bytes) |
| `details` | string \| null | Extended details shown on hold (max 128 bytes) |

## ClearAlertRequest

Request body for `POST /api/alert/clear` and alternative payload for WS `ClearAlert` command (tag `0x0A`).

```json
{
  "tab": 0
}
```

| Field | Type | Description |
|-------|------|-------------|
| `tab` | integer | Tab index to clear (0-15) |

## BrightnessRequest

Request body for `POST /api/brightness`.

```json
{
  "level": 200,
  "save": true
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `level` | integer | required | Brightness level (0-255) |
| `save` | boolean | `false` | Persist setting to EEPROM |

## SetModeRequest

Request body for `POST /api/mode`.

```json
{
  "mode": "Accept"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `mode` | [DeviceMode](#devicemode) | Target mode |

## DeviceMode

String enum for the device operating mode.

| Value | Byte | Description |
|-------|------|-------------|
| `"Default"` | 0 | Normal operating mode |
| `"Accept"` | 1 | Accept/approve mode |
| `"Plan"` | 2 | Planning mode |

## DeviceState

Binary-encoded device state (used in WS `StateChanged` events).

Single byte with bit fields:

| Bits | Field | Values |
|------|-------|--------|
| 1:0 | mode | 0=Default, 1=Accept, 2=Plan |
| 2 | yolo | 0=off, 1=on |
| 7:3 | reserved | 0 |

## DeviceInfo

JSON payload of the WS `DeviceConnected` event (tag `0x80`).

```json
{
  "name": "Agent Deck",
  "firmware": "1.0.0"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Device product name |
| `firmware` | string | Firmware version string |

## SoftKeyType

Enum for soft key assignment types (used in WS `SetSoftKey`/`GetSoftKey` commands).

| Value | Name | Description |
|-------|------|-------------|
| 0 | Default | Use the keymap default action |
| 1 | Keycode | Single 16-bit QMK keycode |
| 2 | String | Type a string on press |
| 3 | Sequence | Tap a sequence of keycodes |

## SoftKeyConfig

Soft key configuration (used in WS `SoftKeyResponse`).

| Field | Type | Description |
|-------|------|-------------|
| `index` | u8 | Key index (0-2) |
| `key_type` | [SoftKeyType](#softkeytype) | Assignment type |
| `data` | u8[] | Type-specific data (max 128 bytes) |

## ApiError

Error response returned by all REST endpoints on failure.

```json
{
  "error": "device locked by WebSocket client"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `error` | string | Human-readable error message |

## Tab States

Constants for the tab state values used in `tabs` arrays:

| Value | Name | Description |
|-------|------|-------------|
| 0 | Inactive | Tab exists but no active process |
| 1 | Started | Process started, waiting |
| 2 | Working | Process actively running |
