//! Tray icon management

use tray_icon::Icon;

/// Embedded icons for dark menu bar (white icons)
const CONNECTED_DARK_DATA: &[u8] = include_bytes!("../../assets/icons/tray_connected.png");
const DISCONNECTED_DARK_DATA: &[u8] = include_bytes!("../../assets/icons/tray_disconnected.png");

/// Embedded icons for light menu bar (black icons)
const CONNECTED_LIGHT_DATA: &[u8] = include_bytes!("../../assets/icons/tray_connected_light.png");
const DISCONNECTED_LIGHT_DATA: &[u8] = include_bytes!("../../assets/icons/tray_disconnected_light.png");

/// Tray icon wrapper with support for light/dark themes
pub struct TrayIcon {
    /// Connected icon for dark menu bar
    pub connected_dark: Icon,
    /// Disconnected icon for dark menu bar
    pub disconnected_dark: Icon,
    /// Connected icon for light menu bar
    pub connected_light: Icon,
    /// Disconnected icon for light menu bar
    pub disconnected_light: Icon,
}

impl TrayIcon {
    /// Create tray icons from embedded data
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            connected_dark: load_icon_from_png(CONNECTED_DARK_DATA)?,
            disconnected_dark: load_icon_from_png(DISCONNECTED_DARK_DATA)?,
            connected_light: load_icon_from_png(CONNECTED_LIGHT_DATA)?,
            disconnected_light: load_icon_from_png(DISCONNECTED_LIGHT_DATA)?,
        })
    }

    /// Get the appropriate connected icon based on system appearance
    pub fn connected(&self) -> &Icon {
        if is_dark_mode() {
            &self.connected_dark
        } else {
            &self.connected_light
        }
    }

    /// Get the appropriate disconnected icon based on system appearance
    pub fn disconnected(&self) -> &Icon {
        if is_dark_mode() {
            &self.disconnected_dark
        } else {
            &self.disconnected_light
        }
    }
}

impl Default for TrayIcon {
    fn default() -> Self {
        Self::new().expect("Failed to load tray icons")
    }
}

/// Detect if macOS is in dark mode
#[cfg(target_os = "macos")]
pub fn is_dark_mode() -> bool {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let user_defaults: id = msg_send![objc::class!(NSUserDefaults), standardUserDefaults];
        let key = NSString::alloc(nil).init_str("AppleInterfaceStyle");
        let value: id = msg_send![user_defaults, stringForKey: key];

        if value == nil {
            // No value means light mode (default)
            false
        } else {
            let utf8: *const i8 = msg_send![value, UTF8String];
            if utf8.is_null() {
                false
            } else {
                let style = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
                style == "Dark"
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn is_dark_mode() -> bool {
    // Default to dark mode on other platforms
    true
}

/// Load an icon from PNG data
fn load_icon_from_png(data: &[u8]) -> anyhow::Result<Icon> {
    // Decode PNG
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info()?;

    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;

    // Truncate buffer to actual size
    buf.truncate(info.buffer_size());

    // Convert to RGBA if needed
    let rgba_data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            // Add alpha channel
            let mut rgba = Vec::with_capacity(buf.len() * 4 / 3);
            for chunk in buf.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            // Convert to RGBA
            let mut rgba = Vec::with_capacity(buf.len() * 2);
            for chunk in buf.chunks(2) {
                let gray = chunk[0];
                let alpha = chunk[1];
                rgba.extend_from_slice(&[gray, gray, gray, alpha]);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            // Convert to RGBA
            let mut rgba = Vec::with_capacity(buf.len() * 4);
            for &gray in &buf {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
            rgba
        }
        png::ColorType::Indexed => {
            anyhow::bail!("Indexed color not supported");
        }
    };

    Icon::from_rgba(rgba_data, info.width, info.height)
        .map_err(|e| anyhow::anyhow!("Failed to create icon: {}", e))
}
