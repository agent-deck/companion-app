//! Tray icon management

use tray_icon::Icon;

/// Embedded connected icon (green/active)
const CONNECTED_ICON_DATA: &[u8] = include_bytes!("../../assets/icons/tray_connected.png");

/// Embedded disconnected icon (grey/inactive)
const DISCONNECTED_ICON_DATA: &[u8] = include_bytes!("../../assets/icons/tray_disconnected.png");

/// Tray icon wrapper
pub struct TrayIcon {
    /// Connected icon
    pub connected: Icon,
    /// Disconnected icon
    pub disconnected: Icon,
}

impl TrayIcon {
    /// Create tray icons from embedded data
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            connected: load_icon_from_png(CONNECTED_ICON_DATA)?,
            disconnected: load_icon_from_png(DISCONNECTED_ICON_DATA)?,
        })
    }
}

impl Default for TrayIcon {
    fn default() -> Self {
        Self::new().expect("Failed to load tray icons")
    }
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
