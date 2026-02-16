//! HID command helpers
//!
//! Convenience functions for building common HID commands.
//! All builders return `Vec<HidPacket>` using the chunked protocol.

#![allow(dead_code)]

use super::protocol::{build_chunked_packets, DeviceMode, HidCommand, HidPacket, SoftKeyType};

/// Build a display update with session name, current task, tab states, and active tab index
pub fn build_display_update(session: &str, task: Option<&str>, tabs: &[u8], active: usize) -> Vec<HidPacket> {
    let json = serde_json::json!({
        "session": session,
        "task": task.unwrap_or(""),
        "tabs": tabs,
        "active": active,
    });

    tracing::info!("HID display payload: {}", json);

    build_chunked_packets(HidCommand::UpdateDisplay, json.to_string().as_bytes())
}

/// Build a ping packet (single packet)
pub fn build_ping() -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::Ping, &[])
}

/// Build a brightness control packet (single packet)
pub fn build_set_brightness(level: u8, save: bool) -> Vec<HidPacket> {
    let payload = [level, if save { 0x01 } else { 0x00 }];
    build_chunked_packets(HidCommand::SetBrightness, &payload)
}

/// Build a set soft key command (may be multi-packet for long string data)
pub fn build_set_soft_key(index: u8, key_type: SoftKeyType, data: &[u8], save: bool) -> Vec<HidPacket> {
    let mut payload = vec![index, key_type as u8, if save { 0x01 } else { 0x00 }];
    payload.extend_from_slice(data);
    build_chunked_packets(HidCommand::SetSoftKey, &payload)
}

/// Build a get soft key query (single packet)
pub fn build_get_soft_key(index: u8) -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::GetSoftKey, &[index])
}

/// Build a reset soft keys command (single packet)
pub fn build_reset_soft_keys() -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::ResetSoftKeys, &[])
}

/// Build a set mode command (single packet)
pub fn build_set_mode(mode: DeviceMode) -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::SetMode, &[mode as u8])
}

/// Build an alert command to show an overlay on the device
pub fn build_alert(tab: usize, session: &str, text: &str) -> Vec<HidPacket> {
    let json = serde_json::json!({
        "tab": tab,
        "session": session,
        "text": text,
    });
    build_chunked_packets(HidCommand::Alert, json.to_string().as_bytes())
}

/// Build a clear alert command (no text field = clear)
pub fn build_clear_alert(tab: usize) -> Vec<HidPacket> {
    let json = serde_json::json!({
        "tab": tab,
    });
    build_chunked_packets(HidCommand::Alert, json.to_string().as_bytes())
}


#[cfg(test)]
mod tests {
    use super::*;
    use super::super::protocol::{FLAG_END, FLAG_START};

    #[test]
    fn test_build_display_update() {
        let packets = build_display_update("my-session", Some("Reading files"), &[0, 1, 2], 1);
        assert!(!packets.is_empty());
        assert!(packets[0].is_start());
        assert!(packets.last().unwrap().is_end());
        for p in &packets {
            assert_eq!(p.command(), Some(HidCommand::UpdateDisplay));
        }
    }

    #[test]
    fn test_build_display_update_no_task() {
        let packets = build_display_update("my-session", None, &[1], 0);
        assert!(!packets.is_empty());
        assert!(packets[0].is_start());
    }

    #[test]
    fn test_build_ping() {
        let packets = build_ping();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].flags(), FLAG_START | FLAG_END);
        assert_eq!(packets[0].command(), Some(HidCommand::Ping));
    }

    #[test]
    fn test_build_set_brightness() {
        let packets = build_set_brightness(200, true);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::SetBrightness));
        assert_eq!(packets[0].payload()[0], 200);
        assert_eq!(packets[0].payload()[1], 0x01);
    }

    #[test]
    fn test_build_set_soft_key() {
        let packets = build_set_soft_key(0, SoftKeyType::String, b"hello", true);
        assert!(!packets.is_empty());
        assert_eq!(packets[0].command(), Some(HidCommand::SetSoftKey));
        let payload = packets[0].payload();
        assert_eq!(payload[0], 0); // index
        assert_eq!(payload[1], 2); // SoftKeyType::String
        assert_eq!(payload[2], 1); // save
        assert_eq!(&payload[3..8], b"hello");
    }

    #[test]
    fn test_build_get_soft_key() {
        let packets = build_get_soft_key(2);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::GetSoftKey));
        assert_eq!(packets[0].payload()[0], 2);
    }

    #[test]
    fn test_build_reset_soft_keys() {
        let packets = build_reset_soft_keys();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::ResetSoftKeys));
    }

    #[test]
    fn test_build_set_mode() {
        let packets = build_set_mode(DeviceMode::Plan);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::SetMode));
        assert_eq!(packets[0].payload()[0], 2); // Plan = 2
    }

}
