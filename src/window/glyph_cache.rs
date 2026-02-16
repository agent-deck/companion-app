//! Glyph cache using WezTerm's font system for proper Unicode rendering
//!
//! This module wraps wezterm-font to provide:
//! - FreeType + HarfBuzz based font shaping and rasterization
//! - Proper support for box drawing, Braille, and special symbols
//! - Glyph caching with egui textures

use config::{ConfigHandle, FontAttributes, TextStyle};
use egui::{ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;
use std::rc::Rc;
use tracing::{debug, warn};
use wezterm_font::{FontConfiguration, FontMetrics, LoadedFont, RasterizedGlyph};

/// Key for caching glyphs - identifies a unique glyph rendering
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct GlyphKey {
    /// The text to render (usually a single character but can be grapheme cluster)
    text: String,
    /// Font variant index (for bold/italic)
    style_key: StyleKey,
}

/// Identifies a font style variant
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum StyleKey {
    Regular,
    Bold,
    Italic,
    BoldItalic,
}

impl StyleKey {
    pub fn from_attrs(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => StyleKey::Regular,
            (true, false) => StyleKey::Bold,
            (false, true) => StyleKey::Italic,
            (true, true) => StyleKey::BoldItalic,
        }
    }
}

/// A cached glyph with its texture and positioning info
/// All dimensions are in logical pixels (scaled for egui coordinates)
pub struct CachedGlyph {
    /// The egui texture containing the rasterized glyph
    pub texture: TextureHandle,
    /// Width of the glyph in logical pixels
    pub width: f64,
    /// Height of the glyph in logical pixels
    pub height: f64,
    /// Horizontal bearing in logical pixels (offset from origin to left edge of glyph)
    pub bearing_x: f64,
    /// Vertical bearing in logical pixels (offset from baseline to top of glyph)
    pub bearing_y: f64,
    /// Horizontal advance in logical pixels (how far to move after drawing this glyph)
    pub x_advance: f64,
    /// Whether this glyph has color (e.g., emoji)
    pub has_color: bool,
}

/// Glyph cache that uses WezTerm's font system for rasterization
pub struct GlyphCache {
    /// WezTerm font configuration
    font_config: FontConfiguration,
    /// Loaded fonts for different styles
    fonts: HashMap<StyleKey, Rc<LoadedFont>>,
    /// Cached glyphs (text + style -> CachedGlyph)
    cache: HashMap<GlyphKey, CachedGlyph>,
    /// Font metrics for the regular style
    metrics: FontMetrics,
    /// DPI used for rasterization (kept for debugging)
    #[allow(dead_code)]
    dpi: usize,
    /// Scale factor for converting physical pixels to logical pixels
    scale_factor: f64,
    /// Texture counter for unique names
    texture_counter: usize,
}

/// Default font size used by WezTerm's config
const WEZTERM_DEFAULT_FONT_SIZE: f64 = 12.0;

/// Base DPI used for glyph rasterization (Windows/Linux standard)
pub const BASE_DPI: f64 = 96.0;

impl GlyphCache {
    /// Create a new glyph cache with the specified scale factor and font size
    ///
    /// Uses a default font configuration from wezterm-font.
    /// Rasterizes at native DPI for crisp glyphs, then scales metrics for logical coordinates.
    /// The font_size parameter adjusts the effective DPI to achieve the desired size.
    pub fn new(scale_factor: f64, font_size: f32) -> anyhow::Result<Self> {
        // Rasterize at native DPI for crisp glyphs on HiDPI displays
        // Base DPI is 96 (Windows/Linux standard) or 72 (macOS traditional)
        // We use 96 as base and multiply by scale factor for physical pixel rendering
        // Scale DPI by (desired_font_size / wezterm_default_font_size) to achieve the right size
        let font_scale = font_size as f64 / WEZTERM_DEFAULT_FONT_SIZE;
        let dpi = (BASE_DPI * scale_factor * font_scale) as usize;
        debug!(
            "Creating GlyphCache with scale_factor={}, font_size={}, DPI={}",
            scale_factor, font_size, dpi
        );

        // Create default config handle - this uses WezTerm's default font settings
        let config_handle = ConfigHandle::default_config();

        // Create font configuration
        let font_config = FontConfiguration::new(Some(config_handle), dpi)?;

        // Load the default font and get metrics
        let default_font = font_config.default_font()?;
        let metrics = default_font.metrics();

        debug!(
            "Font metrics (physical): cell_width={:.2}, cell_height={:.2}",
            metrics.cell_width.get(),
            metrics.cell_height.get()
        );
        debug!(
            "Font metrics (logical): cell_width={:.2}, cell_height={:.2}",
            metrics.cell_width.get() / scale_factor,
            metrics.cell_height.get() / scale_factor
        );

        let mut fonts = HashMap::new();
        fonts.insert(StyleKey::Regular, default_font);

        Ok(Self {
            font_config,
            fonts,
            cache: HashMap::new(),
            metrics,
            dpi,
            scale_factor,
            texture_counter: 0,
        })
    }

    /// Get font metrics (in physical pixels)
    pub fn metrics(&self) -> &FontMetrics {
        &self.metrics
    }

    /// Get cell width in logical pixels (for egui coordinates)
    pub fn cell_width(&self) -> f64 {
        self.metrics.cell_width.get() / self.scale_factor
    }

    /// Get cell height in logical pixels (for egui coordinates)
    pub fn cell_height(&self) -> f64 {
        self.metrics.cell_height.get() / self.scale_factor
    }

    /// Get scale factor
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Determine if a character should be rendered as a color glyph (emoji)
    /// In terminal context, most symbols should be monochrome even if the font
    /// provides a color version. Only true pictographic emoji should be color.
    fn should_be_color_glyph(text: &str) -> bool {
        let Some(c) = text.chars().next() else {
            return false;
        };
        let cp = c as u32;

        // Whitelist approach: only render as color for true pictographic emoji
        // These are the main emoji ranges that should actually be colorful
        matches!(
            cp,
            // Dingbats (scissors, sparkles, stars: ✂️ ❇️ ✳️ etc.)
            0x2700..=0x27BF |
            // Emoticons (smileys, people)
            0x1F600..=0x1F64F |
            // Miscellaneous Symbols and Pictographs (animals, food, objects)
            0x1F300..=0x1F5FF |
            // Transport and Map Symbols
            0x1F680..=0x1F6FF |
            // Geometric Shapes Extended (colored circles, squares: 🟢🟡🔴 etc.)
            0x1F780..=0x1F7FF |
            // Supplemental Symbols and Pictographs
            0x1F900..=0x1F9FF |
            // Symbols and Pictographs Extended-A
            0x1FA00..=0x1FAFF |
            // Regional Indicator Symbols (flags)
            0x1F1E0..=0x1F1FF
        )
    }

    /// Get or load a font for the specified style
    fn get_font(&mut self, style: StyleKey) -> anyhow::Result<Rc<LoadedFont>> {
        if let Some(font) = self.fonts.get(&style) {
            return Ok(Rc::clone(font));
        }

        // Build TextStyle for the requested variant
        let text_style = match style {
            StyleKey::Regular => TextStyle::default(),
            StyleKey::Bold => {
                let mut attrs = FontAttributes::default();
                attrs.weight = config::FontWeight::BOLD;
                TextStyle {
                    font: vec![attrs],
                    foreground: None,
                }
            }
            StyleKey::Italic => {
                let mut attrs = FontAttributes::default();
                attrs.style = config::FontStyle::Italic;
                TextStyle {
                    font: vec![attrs],
                    foreground: None,
                }
            }
            StyleKey::BoldItalic => {
                let mut attrs = FontAttributes::default();
                attrs.weight = config::FontWeight::BOLD;
                attrs.style = config::FontStyle::Italic;
                TextStyle {
                    font: vec![attrs],
                    foreground: None,
                }
            }
        };

        let font = self.font_config.resolve_font(&text_style)?;
        self.fonts.insert(style, Rc::clone(&font));
        Ok(font)
    }

    /// Get or rasterize a glyph, returning a cached texture
    ///
    /// Returns None if the glyph cannot be rendered (e.g., missing from font)
    pub fn get_glyph(
        &mut self,
        ctx: &egui::Context,
        text: &str,
        style: StyleKey,
    ) -> Option<&CachedGlyph> {
        let key = GlyphKey {
            text: text.to_string(),
            style_key: style,
        };

        // Check cache first
        if self.cache.contains_key(&key) {
            return self.cache.get(&key);
        }

        // Rasterize the glyph
        match self.rasterize_glyph(ctx, text, style) {
            Ok(cached) => {
                self.cache.insert(key.clone(), cached);
                self.cache.get(&key)
            }
            Err(e) => {
                warn!("Failed to rasterize glyph '{}': {}", text, e);
                None
            }
        }
    }

    /// Rasterize a glyph and create an egui texture
    fn rasterize_glyph(
        &mut self,
        ctx: &egui::Context,
        text: &str,
        style: StyleKey,
    ) -> anyhow::Result<CachedGlyph> {
        use wezterm_font::shaper::Direction;

        // Get the font for this style
        let font = self.get_font(style)?;

        // Shape the text to get glyph information
        let glyph_infos = font.blocking_shape(text, None, Direction::LeftToRight, None, None)?;

        if glyph_infos.is_empty() {
            anyhow::bail!("No glyphs produced for text: {}", text);
        }

        // For single-cell characters, we just use the first glyph
        // For multi-glyph sequences (ligatures, combining chars), we'd need to handle them differently
        let glyph_info = &glyph_infos[0];

        // Rasterize the glyph
        let rasterized = font.rasterize_glyph(glyph_info.glyph_pos, glyph_info.font_idx)?;

        // Convert the rasterized glyph to an egui texture
        let texture = self.create_texture(ctx, &rasterized, text)?;

        // Determine if this should be treated as a color glyph
        // Some characters should never be rendered as color emoji even if the font says so
        let has_color = rasterized.has_color && Self::should_be_color_glyph(text);

        if rasterized.has_color && !has_color {
            debug!(
                "Forcing monochrome rendering for '{}' (U+{:04X})",
                text,
                text.chars().next().map(|c| c as u32).unwrap_or(0)
            );
        }

        // Scale all metrics to logical pixels for egui coordinates
        let scale = self.scale_factor;
        Ok(CachedGlyph {
            texture,
            width: rasterized.width as f64 / scale,
            height: rasterized.height as f64 / scale,
            bearing_x: rasterized.bearing_x.get() / scale,
            bearing_y: rasterized.bearing_y.get() / scale,
            x_advance: glyph_info.x_advance.get() / scale,
            has_color,
        })
    }

    /// Create an egui texture from a rasterized glyph
    fn create_texture(
        &mut self,
        ctx: &egui::Context,
        rasterized: &RasterizedGlyph,
        text: &str,
    ) -> anyhow::Result<TextureHandle> {
        let width = rasterized.width;
        let height = rasterized.height;

        // Handle empty glyphs (like space)
        if width == 0 || height == 0 {
            // Create a 1x1 transparent texture
            let image = ColorImage::new([1, 1], egui::Color32::TRANSPARENT);
            let name = format!("glyph_{}_empty", self.texture_counter);
            self.texture_counter += 1;
            return Ok(ctx.load_texture(name, image, TextureOptions::NEAREST));
        }

        // Convert pre-multiplied RGBA to egui ColorImage
        // The rasterized.data is pre-multiplied RGBA 32bpp
        let mut pixels = Vec::with_capacity(width * height);

        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) * 4;
                let r = rasterized.data[idx];
                let g = rasterized.data[idx + 1];
                let b = rasterized.data[idx + 2];
                let a = rasterized.data[idx + 3];

                // For monochrome glyphs (has_color = false), the glyph is typically
                // rendered with white pixels and the alpha channel contains the shape.
                // We'll use white for the color and let the alpha define the shape,
                // then tint it with the foreground color when rendering.
                if rasterized.has_color {
                    // Color glyph (emoji) - use colors as-is but un-premultiply
                    if a > 0 {
                        let alpha_f = a as f32 / 255.0;
                        let r_straight = ((r as f32) / alpha_f).min(255.0) as u8;
                        let g_straight = ((g as f32) / alpha_f).min(255.0) as u8;
                        let b_straight = ((b as f32) / alpha_f).min(255.0) as u8;
                        pixels.push(egui::Color32::from_rgba_unmultiplied(
                            r_straight, g_straight, b_straight, a,
                        ));
                    } else {
                        pixels.push(egui::Color32::TRANSPARENT);
                    }
                } else {
                    // Monochrome glyph - store as white with alpha, will be tinted later
                    // The typical FreeType output has grayscale anti-aliasing in the alpha
                    pixels.push(egui::Color32::from_rgba_unmultiplied(255, 255, 255, a));
                }
            }
        }

        let image = ColorImage {
            size: [width, height],
            pixels,
        };

        let name = format!("glyph_{}_{}", self.texture_counter, text.escape_debug());
        self.texture_counter += 1;

        Ok(ctx.load_texture(name, image, TextureOptions::LINEAR))
    }

    /// Clear all cached glyphs (useful when font settings change)
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Get the number of cached glyphs
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }
}
