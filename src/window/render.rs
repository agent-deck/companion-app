//! Terminal rendering logic
//!
//! Contains the main render function and terminal content rendering helpers.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use parking_lot::Mutex;
use wezterm_cell::color::ColorAttribute;
use wezterm_cell::Hyperlink;
use wezterm_term::color::ColorPalette;

use crate::core::bookmarks::BookmarkManager;
use crate::core::sessions::SessionId;
use crate::core::settings::ColorScheme;
use crate::core::themes::Theme;
use crate::terminal::Session;
use super::glyph_cache::{GlyphCache, StyleKey};
use super::new_tab::{render_new_tab_page, NewTabAction};
use super::settings_modal::SettingsModalResult;
use super::terminal::TerminalAction;

/// Tab bar height in logical pixels
pub const TAB_BAR_HEIGHT: f32 = 32.0;

/// Maximum tab title length
pub const MAX_TAB_TITLE_LEN: usize = 50;

/// Claude icon SVG (white, for tinting)
pub const CLAUDE_ICON_SVG: &[u8] = include_bytes!("../../assets/icons/claude.svg");

/// Deck connected icon SVG
pub const DECK_CONNECTED_SVG: &[u8] = include_bytes!("../../assets/icons/deck-connected.svg");

/// Deck disconnected icon SVG
pub const DECK_DISCONNECTED_SVG: &[u8] = include_bytes!("../../assets/icons/deck-disconnected.svg");

/// Claude orange color
pub const CLAUDE_ORANGE: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x77, 0x57);

/// Convert ColorAttribute to egui Color32 using the provided palette
pub fn color_attr_to_egui(
    attr: ColorAttribute,
    palette: &ColorPalette,
    is_foreground: bool,
) -> egui::Color32 {
    let srgba = if is_foreground {
        palette.resolve_fg(attr)
    } else {
        palette.resolve_bg(attr)
    };
    egui::Color32::from_rgb(
        (srgba.0 * 255.0) as u8,
        (srgba.1 * 255.0) as u8,
        (srgba.2 * 255.0) as u8,
    )
}

/// Session data tuple for rendering
/// (id, title, is_new_tab, is_running, working_dir, is_loading, terminal_title, bell_active, claude_activity, finished_in_background)
pub type SessionRenderData = (
    SessionId,
    String,
    bool,
    bool,
    String,
    bool,
    Option<String>,
    bool,
    crate::core::sessions::ClaudeActivity,
    bool,
);

/// Parameters for rendering the terminal window
pub struct RenderParams<'a> {
    pub scroll_offset: usize,
    pub font_size: f32,
    pub color_scheme: ColorScheme,
    pub current_theme: &'a Theme,
    pub has_selection_for_menu: bool,
    pub sessions_data: Vec<SessionRenderData>,
    pub active_session_idx: usize,
    pub hid_connected: bool,
    pub active_session_data: Option<(Arc<Mutex<Session>>, bool, SessionId)>,
    pub bookmark_manager: BookmarkManager,
    pub selection: Option<((i64, usize), (i64, usize))>,
    pub cached_char_width: &'a Cell<f32>,
    pub cached_line_height: &'a Cell<f32>,
    pub glyph_cache: &'a RefCell<Option<GlyphCache>>,
    pub hovered_hyperlink: &'a Option<Arc<Hyperlink>>,
}

/// Render the tab bar
pub fn render_tab_bar(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    params: &RenderParams<'_>,
    new_actions: &mut Vec<TerminalAction>,
    need_install_loaders: bool,
) {
    let color_scheme = params.color_scheme;
    let sessions_data = &params.sessions_data;
    let active_session_idx = params.active_session_idx;
    let hid_connected = params.hid_connected;

    // Install image loaders once (for SVG support)
    if need_install_loaders {
        egui_extras::install_image_loaders(ctx);
    }

    // Override button visual style for tabs
    ui.style_mut().visuals.widgets.inactive.bg_fill = color_scheme.inactive_tab_background();
    ui.style_mut().visuals.widgets.hovered.bg_fill = color_scheme.inactive_tab_background();
    ui.style_mut().visuals.widgets.active.bg_fill = color_scheme.active_tab_background();
    ui.style_mut().visuals.widgets.inactive.weak_bg_fill = color_scheme.inactive_tab_background();
    ui.style_mut().visuals.widgets.hovered.weak_bg_fill = color_scheme.inactive_tab_background();
    ui.style_mut().visuals.widgets.active.weak_bg_fill = color_scheme.active_tab_background();
    // Remove yellow/olive stroke
    ui.style_mut().visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    ui.style_mut().visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, color_scheme.tab_hover_stroke());
    ui.style_mut().visuals.widgets.active.bg_stroke = egui::Stroke::NONE;

    // Calculate available width for tabs
    let total_width = ui.available_width();
    let right_icons_width = 70.0; // Settings + indicator + padding
    let new_tab_btn_width = 32.0;
    let tab_spacing = 1.0; // Gap between tabs
    let tabs_area_width = total_width - right_icons_width - new_tab_btn_width - 8.0;

    // Calculate per-tab width (including padding) to fill available space
    let num_tabs = sessions_data.len().max(1);
    let min_tab_width = 108.0_f32; // Minimum total tab width including padding
    // Account for spacing between tabs: (n-1) gaps for n tabs
    let total_spacing = if num_tabs > 1 { (num_tabs - 1) as f32 * tab_spacing } else { 0.0 };
    let available_for_tabs = tabs_area_width - total_spacing;
    // Calculate full tab width (including padding)
    let full_tab_width = (available_for_tabs / num_tabs as f32).max(min_tab_width);
    // Calculate how many tabs actually fit
    let max_visible_tabs = ((tabs_area_width + tab_spacing) / (full_tab_width + tab_spacing)).floor() as usize;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = tab_spacing;

        // Constrain tabs + new tab button to their allocated area
        let tabs_plus_btn_width = tabs_area_width + new_tab_btn_width + 4.0;
        ui.allocate_ui_with_layout(
            egui::vec2(tabs_plus_btn_width, TAB_BAR_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_clip_rect(ui.max_rect());

                // Render only tabs that fit
                for (idx, (id, title, _is_new, is_running, working_dir, is_loading, terminal_title, bell_active, claude_activity, finished_in_background)) in sessions_data.iter().take(max_visible_tabs).enumerate() {
                    render_single_tab(
                        ui,
                        ctx,
                        idx,
                        *id,
                        title,
                        *_is_new,
                        *is_running,
                        working_dir,
                        *is_loading,
                        terminal_title,
                        *bell_active,
                        claude_activity,
                        *finished_in_background,
                        active_session_idx,
                        full_tab_width,
                        color_scheme,
                        new_actions,
                    );
                }

                // New tab button
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("+")
                                .size(16.0)
                                .color(color_scheme.foreground()),
                        )
                        .fill(color_scheme.inactive_tab_background())
                        .stroke(egui::Stroke::NONE)
                        .rounding(egui::Rounding {
                            nw: 4.0,
                            ne: 4.0,
                            sw: 0.0,
                            se: 0.0,
                        })
                        .min_size(egui::vec2(new_tab_btn_width, TAB_BAR_HEIGHT - 4.0)),
                    )
                    .on_hover_text("New Tab (Cmd+T)")
                    .clicked()
                {
                    new_actions.push(TerminalAction::NewTab);
                }
            },
        );

        // Right side: settings and connection indicator
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);

            // HID connection indicator
            let (icon_bytes, icon_uri, icon_tint, indicator_text) = if hid_connected {
                (
                    DECK_CONNECTED_SVG,
                    "bytes://deck-connected.svg",
                    egui::Color32::from_rgb(34, 139, 34), // Dark green (forest green)
                    "Connected",
                )
            } else {
                (
                    DECK_DISCONNECTED_SVG,
                    "bytes://deck-disconnected.svg",
                    egui::Color32::from_rgb(100, 60, 60), // Muted dark red
                    "Disconnected",
                )
            };

            let icon_size = 16.0;
            ui.add(
                egui::Image::from_bytes(icon_uri, icon_bytes)
                    .fit_to_exact_size(egui::vec2(icon_size, icon_size))
                    .tint(icon_tint),
            )
            .on_hover_text(indicator_text);

            ui.add_space(8.0);

            // Settings button
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("\u{2699}")
                            .size(16.0)
                            .color(color_scheme.foreground()),
                    )
                    .frame(false),
                )
                .on_hover_text("Settings (Cmd+,)")
                .clicked()
            {
                new_actions.push(TerminalAction::OpenSettings);
            }
        });
    });
}

/// Render a single tab in the tab bar
#[allow(clippy::too_many_arguments)]
fn render_single_tab(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    idx: usize,
    id: SessionId,
    title: &str,
    _is_new: bool,
    is_running: bool,
    working_dir: &str,
    is_loading: bool,
    terminal_title: &Option<String>,
    bell_active: bool,
    claude_activity: &crate::core::sessions::ClaudeActivity,
    finished_in_background: bool,
    active_session_idx: usize,
    full_tab_width: f32,
    color_scheme: ColorScheme,
    new_actions: &mut Vec<TerminalAction>,
) {
    let is_active = idx == active_session_idx;
    let tab_bg = if is_active {
        color_scheme.active_tab_background()
    } else if bell_active {
        // Visual bell indicator
        color_scheme.bell_tab_background()
    } else {
        color_scheme.inactive_tab_background()
    };
    // Check if Claude is working in this tab
    let is_claude_working = claude_activity.is_working();
    // Dim text for placeholder tabs (not yet started)
    let text_color = if is_running || _is_new || is_loading {
        color_scheme.foreground()
    } else {
        // Dimmed for placeholder tabs
        let fg = color_scheme.foreground();
        egui::Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 150)
    };

    // Allocate the entire tab area with click sensing
    let tab_size = egui::vec2(full_tab_width, TAB_BAR_HEIGHT - 4.0);
    let (tab_rect, tab_response) = ui.allocate_exact_size(
        tab_size,
        egui::Sense::click(),
    );

    // Draw tab background
    ui.painter().rect_filled(
        tab_rect,
        egui::Rounding {
            nw: 4.0,
            ne: 4.0,
            sw: 0.0,
            se: 0.0,
        },
        tab_bg,
    );

    // Close button rect (positioned at right side of tab)
    let close_size = 18.0;
    let close_btn_margin = 8.0;
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(
            tab_rect.right() - close_size - close_btn_margin,
            tab_rect.center().y - close_size / 2.0,
        ),
        egui::vec2(close_size, close_size),
    );

    // Check if close button was clicked
    let close_clicked = tab_response.clicked()
        && tab_response.interact_pointer_pos()
            .map(|pos| close_rect.contains(pos))
            .unwrap_or(false);

    // Render tab content inside the allocated area
    // Account for close button space on the right
    let content_padding = 4.0;
    let close_btn_total_width = close_size + close_btn_margin + content_padding;
    let content_rect = egui::Rect::from_min_max(
        tab_rect.min + egui::vec2(content_padding, 0.0),
        tab_rect.max - egui::vec2(close_btn_total_width, 0.0),
    );
    let mut content_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    content_ui.set_height(TAB_BAR_HEIGHT - 4.0);

    {
        let ui = &mut content_ui;

        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;

            // Left padding
            ui.add_space(6.0);

            // Claude icon or loading spinner
            let icon_size = 14.0;
            if is_loading {
                // Show spinning loader when loading
                let time = ui.input(|i| i.time);
                let angle = time * 3.0; // Rotate 3 radians per second
                let spinner_color = CLAUDE_ORANGE;

                // Draw a simple spinning arc
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(icon_size, icon_size),
                    egui::Sense::hover(),
                );
                let center = rect.center();
                let radius = icon_size / 2.0 - 1.0;
                let painter = ui.painter();

                // Draw arc segments to create spinner effect
                let segments = 8;
                for i in 0..segments {
                    let start_angle = angle as f32 + (i as f32 * std::f32::consts::TAU / segments as f32);
                    let alpha = ((i as f32 / segments as f32) * 200.0) as u8 + 55;
                    let color = egui::Color32::from_rgba_unmultiplied(
                        spinner_color.r(),
                        spinner_color.g(),
                        spinner_color.b(),
                        alpha,
                    );
                    let x = center.x + radius * start_angle.cos();
                    let y = center.y + radius * start_angle.sin();
                    painter.circle_filled(egui::pos2(x, y), 1.5, color);
                }
            } else {
                // Claude icon with pulsing for working sessions
                let icon_tint = if is_claude_working && !is_active {
                    // Pulse the icon intensity when Claude is working (background tabs)
                    let time = ui.input(|i| i.time);
                    // Slow pulse: ~2 second cycle
                    let pulse = ((time * std::f64::consts::PI).sin() * 0.5 + 0.5) as f32;
                    // Interpolate between dim (0.3) and bright (1.0)
                    let intensity = 0.3 + pulse * 0.7;
                    egui::Color32::from_rgba_unmultiplied(
                        (CLAUDE_ORANGE.r() as f32 * intensity) as u8,
                        (CLAUDE_ORANGE.g() as f32 * intensity) as u8,
                        (CLAUDE_ORANGE.b() as f32 * intensity) as u8,
                        255,
                    )
                } else if is_running {
                    CLAUDE_ORANGE // Orange for running tabs
                } else {
                    egui::Color32::from_gray(100) // Gray for inactive/new tabs
                };

                // Request repaint for pulsing animation
                if is_claude_working && !is_active {
                    ui.ctx().request_repaint();
                }

                ui.add(
                    egui::Image::from_bytes(
                        "bytes://claude-icon.svg",
                        CLAUDE_ICON_SVG,
                    )
                    .fit_to_exact_size(egui::vec2(icon_size, icon_size))
                    .tint(icon_tint),
                );
            }

            // Tab title
            let display_title = if is_loading {
                "Starting...".to_string()
            } else {
                title.to_string()
            };

            // Calculate available width for title
            let available_width = ui.available_width();

            // Measure actual text width using galley
            let font_id = egui::FontId::proportional(11.0);
            let full_text_width = ui.fonts(|f| {
                f.layout_no_wrap(display_title.clone(), font_id.clone(), text_color).size().x
            });

            let truncated_title = if full_text_width > available_width && display_title.len() > 3 {
                // Binary search for the right truncation point
                let ellipsis_width = ui.fonts(|f| {
                    f.layout_no_wrap("...".to_string(), font_id.clone(), text_color).size().x
                });
                let target_width = available_width - ellipsis_width;

                let mut end = display_title.len();
                while end > 0 {
                    let test_str = &display_title[..end];
                    let test_width = ui.fonts(|f| {
                        f.layout_no_wrap(test_str.to_string(), font_id.clone(), text_color).size().x
                    });
                    if test_width <= target_width {
                        break;
                    }
                    // Step back by char boundary
                    end = display_title[..end].char_indices().last().map(|(i, _)| i).unwrap_or(0);
                }

                if end < display_title.len() && end > 0 {
                    format!("{}...", &display_title[..end])
                } else {
                    display_title
                }
            } else {
                display_title
            };

            // Center the title in remaining space
            ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                // Add underline if Claude finished working in background
                let mut rich_text = egui::RichText::new(truncated_title)
                    .size(11.0)
                    .color(text_color);
                if finished_in_background && !is_active {
                    rich_text = rich_text.underline();
                }
                ui.add(
                    egui::Label::new(rich_text)
                        .selectable(false),
                );
            });
        });
    }

    // Draw close button
    let close_hovered = ui.rect_contains_pointer(close_rect);
    let close_color = if close_hovered {
        color_scheme.close_button_hover_color()
    } else {
        color_scheme.close_button_color()
    };
    ui.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{00d7}",
        egui::FontId::proportional(14.0),
        close_color,
    );

    // Set cursor to pointer for tab
    if tab_response.hovered() {
        ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
    }

    // Add tooltip with conversation title (if available) and full directory path
    // Skip tooltip for new tabs (empty working directory)
    if !working_dir.is_empty() {
        let tooltip_text = if let Some(term_title) = terminal_title {
            if !term_title.is_empty() {
                format!("{}\n{}", term_title, working_dir)
            } else {
                working_dir.to_string()
            }
        } else {
            working_dir.to_string()
        };
        tab_response.clone().on_hover_text(tooltip_text);
    }

    // Handle clicks
    if close_clicked {
        new_actions.push(TerminalAction::CloseTab(id));
    } else if tab_response.clicked() {
        new_actions.push(TerminalAction::SwitchTab(id));
    }

    // Middle-click to close
    if tab_response.middle_clicked() {
        new_actions.push(TerminalAction::CloseTab(id));
    }
}

/// Render the main terminal content area
pub fn render_terminal_content(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    params: &RenderParams<'_>,
    new_actions: &mut Vec<TerminalAction>,
) {
    let color_scheme = params.color_scheme;
    let font_size = params.font_size;
    let scroll_offset = params.scroll_offset;
    let selection = params.selection;
    let cached_char_width = params.cached_char_width;
    let cached_line_height = params.cached_line_height;
    let glyph_cache = params.glyph_cache;
    let hovered_hyperlink = params.hovered_hyperlink;
    let bookmark_manager = &params.bookmark_manager;

    if let Some((session, is_new_tab, session_id)) = &params.active_session_data {
        if *is_new_tab {
            // Render new tab page with session_id for per-tab state
            if let Some(action) =
                render_new_tab_page(ui, bookmark_manager, color_scheme, *session_id)
            {
                match action {
                    NewTabAction::OpenDirectory { path, resume_session } => {
                        new_actions.push(TerminalAction::OpenDirectory { path, resume_session });
                    }
                    NewTabAction::BrowseDirectory => {
                        new_actions.push(TerminalAction::BrowseDirectory);
                    }
                    NewTabAction::AddBookmark(path) => {
                        new_actions.push(TerminalAction::AddBookmark(path));
                    }
                    NewTabAction::RemoveBookmark(path) => {
                        new_actions.push(TerminalAction::RemoveBookmark(path));
                    }
                    NewTabAction::RemoveRecent(path) => {
                        new_actions.push(TerminalAction::RemoveRecent(path));
                    }
                    NewTabAction::ClearRecent => {
                        new_actions.push(TerminalAction::ClearRecent);
                    }
                }
            }
        } else {
            // Render terminal
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            ui.style_mut().spacing.interact_size = egui::vec2(0.0, 0.0);

            let sess = session.lock();
            let palette = params.current_theme.to_color_palette();

            sess.with_terminal_mut(|term| {
                render_terminal_cells(
                    ui,
                    ctx,
                    term,
                    &palette,
                    scroll_offset,
                    selection,
                    font_size,
                    color_scheme,
                    params.current_theme,
                    cached_char_width,
                    cached_line_height,
                    glyph_cache,
                    hovered_hyperlink,
                );
            });
        }
    } else {
        // No active session - show welcome message
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("No tabs open. Press + to create a new tab.")
                    .size(16.0)
                    .color(color_scheme.secondary_foreground()),
            );
        });
    }
}

/// Render terminal cells (the actual terminal content)
#[allow(clippy::too_many_arguments)]
fn render_terminal_cells(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    term: &mut wezterm_term::Terminal,
    palette: &ColorPalette,
    scroll_offset: usize,
    selection: Option<((i64, usize), (i64, usize))>,
    font_size: f32,
    color_scheme: ColorScheme,
    current_theme: &Theme,
    cached_char_width: &Cell<f32>,
    cached_line_height: &Cell<f32>,
    glyph_cache: &RefCell<Option<GlyphCache>>,
    hovered_hyperlink: &Option<Arc<Hyperlink>>,
) {
    use wezterm_cell::Intensity;
    use wezterm_cell::Underline;
    use wezterm_surface::{CursorShape, CursorVisibility};

    let cursor = term.cursor_pos();
    let screen = term.screen_mut();
    let physical_rows = screen.physical_rows;

    let content_min = ui.cursor().min;

    let (char_width, line_height) = {
        let cache_ref = glyph_cache.borrow();
        if let Some(ref cache) = *cache_ref {
            (cache.cell_width() as f32, cache.cell_height() as f32)
        } else {
            let font_id = egui::FontId::monospace(font_size);
            (
                ctx.fonts(|f| f.glyph_width(&font_id, 'M')),
                ctx.fonts(|f| f.row_height(&font_id)),
            )
        }
    };

    cached_char_width.set(char_width);
    cached_line_height.set(line_height);

    let total_lines = screen.scrollback_rows();
    let visible_start = total_lines.saturating_sub(physical_rows + scroll_offset);

    let painter = ui.painter();
    let default_bg = current_theme.background_color32();

    // Collect cells to render
    let mut cells_to_render: Vec<(
        usize,     // row_idx
        usize,     // col_idx
        usize,     // cell_width
        String,    // text
        egui::Color32, // fg
        Option<egui::Color32>, // bg
        bool,      // is_bold
        bool,      // is_italic
        bool,      // has_underline
        bool,      // has_strikethrough
        bool,      // is_hovered_hyperlink
    )> = Vec::new();

    for row_idx in 0..physical_rows {
        let phys_idx = visible_start + row_idx;
        if phys_idx >= total_lines {
            continue;
        }

        let line = screen.line_mut(phys_idx);
        let current_row = phys_idx as i64;

        for cell in line.visible_cells() {
            let col_idx = cell.cell_index();
            let attrs = cell.attrs();
            let mut fg = color_attr_to_egui(attrs.foreground(), palette, true);
            let bg_attr = attrs.background();
            let mut bg = if bg_attr == ColorAttribute::Default {
                None
            } else {
                Some(color_attr_to_egui(bg_attr, palette, false))
            };

            if let Some((sel_start, sel_end)) = selection {
                let in_selection = if sel_start.0 == sel_end.0 {
                    current_row == sel_start.0
                        && col_idx >= sel_start.1
                        && col_idx < sel_end.1
                } else if current_row == sel_start.0 {
                    col_idx >= sel_start.1
                } else if current_row == sel_end.0 {
                    col_idx < sel_end.1
                } else {
                    current_row > sel_start.0 && current_row < sel_end.0
                };

                if in_selection {
                    fg = current_theme.foreground_color32();
                    bg = Some(current_theme.selection_bg_color32());
                }
            }

            let is_bold = matches!(attrs.intensity(), Intensity::Bold);

            if attrs.reverse() {
                let temp_fg = fg;
                fg = bg.unwrap_or(default_bg);
                bg = Some(temp_fg);
            }

            if attrs.invisible() {
                fg = bg.unwrap_or(default_bg);
            }

            let is_italic = attrs.italic();

            let cell_hyperlink = attrs.hyperlink();
            let has_hyperlink = cell_hyperlink.is_some();

            let is_hovered_hyperlink = match (cell_hyperlink, hovered_hyperlink) {
                (Some(cell_link), Some(hovered_link)) => {
                    Arc::ptr_eq(cell_link, hovered_link)
                }
                _ => false,
            };

            let has_underline = attrs.underline() != Underline::None || has_hyperlink;
            if is_hovered_hyperlink {
                fg = color_scheme.hyperlink_hover_color();
            } else if has_hyperlink {
                fg = color_scheme.hyperlink_color();
            }

            let has_strikethrough = attrs.strikethrough();

            let text = cell.str();
            let display_text = if text.is_empty() {
                " ".to_string()
            } else {
                text.to_string()
            };

            let cell_width = cell.width();

            cells_to_render.push((
                row_idx,
                col_idx,
                cell_width,
                display_text,
                fg,
                bg,
                is_bold,
                is_italic,
                has_underline,
                has_strikethrough,
                is_hovered_hyperlink,
            ));
        }
    }

    let mut cache_ref = glyph_cache.borrow_mut();
    let use_glyph_cache = cache_ref.is_some();

    for (
        row_idx,
        col_idx,
        cell_width,
        text,
        fg,
        bg,
        is_bold,
        is_italic,
        has_underline,
        has_strikethrough,
        _is_hovered_hyperlink,
    ) in cells_to_render
    {
        let cell_x = content_min.x + col_idx as f32 * char_width;
        let cell_y = content_min.y + row_idx as f32 * line_height;
        let total_cell_width = cell_width as f32 * char_width;
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(cell_x, cell_y),
            egui::vec2(total_cell_width, line_height),
        );

        if let Some(bg_color) = bg {
            painter.rect_filled(cell_rect, 0.0, bg_color);
        }

        let style_key = StyleKey::from_attrs(is_bold, is_italic);

        if use_glyph_cache {
            if let Some(ref mut cache) = *cache_ref {
                if let Some(glyph) = cache.get_glyph(ctx, &text, style_key) {
                    let (glyph_rect, tint) = if glyph.has_color {
                        let glyph_w = glyph.width as f32;
                        let glyph_h = glyph.height as f32;
                        let scale_x = total_cell_width / glyph_w;
                        let scale_y = line_height / glyph_h;
                        let scale = scale_x.min(scale_y).min(1.0);

                        let scaled_w = glyph_w * scale;
                        let scaled_h = glyph_h * scale;

                        let offset_x = (total_cell_width - scaled_w) / 2.0;
                        let offset_y = (line_height - scaled_h) / 2.0;

                        let rect = egui::Rect::from_min_size(
                            egui::pos2(cell_x + offset_x, cell_y + offset_y),
                            egui::vec2(scaled_w, scaled_h),
                        );
                        (rect, egui::Color32::WHITE)
                    } else {
                        let glyph_x = cell_x + glyph.bearing_x as f32;
                        let baseline_y = cell_y + line_height * 0.8;
                        let glyph_y = baseline_y - glyph.bearing_y as f32;

                        let rect = egui::Rect::from_min_size(
                            egui::pos2(glyph_x, glyph_y),
                            egui::vec2(glyph.width as f32, glyph.height as f32),
                        );
                        (rect, fg)
                    };

                    painter.image(
                        glyph.texture.id(),
                        glyph_rect,
                        egui::Rect::from_min_max(
                            egui::pos2(0.0, 0.0),
                            egui::pos2(1.0, 1.0),
                        ),
                        tint,
                    );
                }
            }
        } else {
            let font_id = egui::FontId::monospace(font_size);
            painter.text(
                egui::pos2(cell_x, cell_y),
                egui::Align2::LEFT_TOP,
                &text,
                font_id,
                fg,
            );
        }

        if has_underline {
            let underline_y = cell_y + line_height - 2.0;
            painter.line_segment(
                [
                    egui::pos2(cell_x, underline_y),
                    egui::pos2(cell_x + total_cell_width, underline_y),
                ],
                egui::Stroke::new(1.0, fg),
            );
        }

        if has_strikethrough {
            let strike_y = cell_y + line_height / 2.0;
            painter.line_segment(
                [
                    egui::pos2(cell_x, strike_y),
                    egui::pos2(cell_x + total_cell_width, strike_y),
                ],
                egui::Stroke::new(1.0, fg),
            );
        }
    }

    drop(cache_ref);

    // Render cursor
    let cursor_in_bounds = cursor.y >= 0 && (cursor.y as usize) < physical_rows;
    let cursor_visible = cursor.visibility == CursorVisibility::Visible;
    let should_draw_cursor = scroll_offset == 0 && cursor_in_bounds && cursor_visible;

    if should_draw_cursor {
        let cursor_pixel_x = content_min.x + cursor.x as f32 * char_width;
        let cursor_pixel_y = content_min.y + cursor.y as f32 * line_height;

        let cursor_color = current_theme.cursor_color32();

        let cursor_rect = match cursor.shape {
            CursorShape::BlinkingBlock | CursorShape::SteadyBlock => {
                egui::Rect::from_min_size(
                    egui::pos2(cursor_pixel_x, cursor_pixel_y),
                    egui::vec2(char_width, line_height),
                )
            }
            CursorShape::BlinkingUnderline | CursorShape::SteadyUnderline => {
                egui::Rect::from_min_size(
                    egui::pos2(cursor_pixel_x, cursor_pixel_y + line_height - 2.0),
                    egui::vec2(char_width, 2.0),
                )
            }
            CursorShape::BlinkingBar | CursorShape::SteadyBar => {
                egui::Rect::from_min_size(
                    egui::pos2(cursor_pixel_x, cursor_pixel_y),
                    egui::vec2(2.0, line_height),
                )
            }
            _ => egui::Rect::from_min_size(
                egui::pos2(cursor_pixel_x, cursor_pixel_y),
                egui::vec2(char_width, line_height),
            ),
        };

        painter.rect_filled(cursor_rect, 0.0, cursor_color);
    }
}

/// Render hyperlink tooltip if hovering over a link
pub fn render_hyperlink_tooltip(
    ctx: &egui::Context,
    hovered_hyperlink: &Option<Arc<Hyperlink>>,
) {
    if let Some(ref hyperlink) = hovered_hyperlink {
        ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);

        egui::show_tooltip_at_pointer(
            ctx,
            egui::LayerId::background(),
            egui::Id::new("hyperlink_tooltip"),
            |ui: &mut egui::Ui| {
                ui.set_min_width(400.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                ui.label(hyperlink.uri());
                ui.add_space(4.0);
                ui.weak(
                    #[cfg(target_os = "macos")]
                    "Cmd+click to open",
                    #[cfg(not(target_os = "macos"))]
                    "Ctrl+click to open",
                );
            },
        );
    }
}

/// Handle settings modal result
pub fn handle_settings_modal_result(
    result: SettingsModalResult,
    new_actions: &mut Vec<TerminalAction>,
) {
    match result {
        SettingsModalResult::Apply(settings) => {
            new_actions.push(TerminalAction::ApplySettings(settings));
        }
        SettingsModalResult::ReadSoftKeys => {
            new_actions.push(TerminalAction::ReadSoftKeys);
        }
        SettingsModalResult::ApplySoftKeys(keys) => {
            new_actions.push(TerminalAction::ApplySoftKeys(keys));
        }
        SettingsModalResult::ResetSoftKeys => {
            new_actions.push(TerminalAction::ResetSoftKeys);
        }
        SettingsModalResult::Cancel => {}
        SettingsModalResult::None => {}
    }
}
