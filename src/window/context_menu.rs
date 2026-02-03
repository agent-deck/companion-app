//! Context menu for right-click popup in terminal window
//!
//! Provides an egui-based context menu as a fallback for non-macOS platforms
//! or when the native context menu fails to display.

use arboard::Clipboard;
use crate::core::claude_sessions::ClaudeSession;
use crate::core::settings::ColorScheme;
use super::terminal::TerminalAction;

/// Context menu state for right-click popup
#[derive(Default)]
pub struct ContextMenuState {
    pub is_open: bool,
    pub position: egui::Pos2,
    pub submenu_open: bool,
    pub available_sessions: Vec<ClaudeSession>,
    /// Time when menu was opened (to prevent immediate close)
    pub opened_time: f64,
}

/// Render context menu and return any triggered actions
pub fn render_context_menu(
    ctx: &egui::Context,
    context_menu: &mut ContextMenuState,
    color_scheme: ColorScheme,
    has_selection: bool,
) -> Vec<TerminalAction> {
    let mut actions = Vec::new();
    let mut close_menu = false;

    // Get current time
    let current_time = ctx.input(|i| i.time);

    // If menu just opened, record the time and don't process close events yet
    if context_menu.opened_time == 0.0 {
        context_menu.opened_time = current_time;
    }

    // Don't allow closing for 150ms after opening (prevents immediate close from right-click)
    let can_close = current_time > context_menu.opened_time + 0.15;

    // Get menu position, clamped to window bounds
    let screen_rect = ctx.screen_rect();
    let menu_width = 220.0;
    let menu_item_height = 28.0;
    let has_sessions = !context_menu.available_sessions.is_empty();

    // Calculate menu height
    let menu_height = (5.0 * menu_item_height) + 24.0;

    let mut pos = context_menu.position;
    if pos.x + menu_width > screen_rect.max.x {
        pos.x = screen_rect.max.x - menu_width - 10.0;
    }
    if pos.y + menu_height > screen_rect.max.y {
        pos.y = screen_rect.max.y - menu_height - 10.0;
    }

    // Check clipboard
    let has_clipboard = Clipboard::new()
        .ok()
        .and_then(|mut c| c.get_text().ok())
        .map(|t| !t.is_empty())
        .unwrap_or(false);

    let mut submenu_rect = egui::Rect::NOTHING;
    let mut load_session_rect = egui::Rect::NOTHING;
    let submenu_sessions = context_menu.available_sessions.clone();

    // Main context menu using egui's menu styling
    let menu_response = egui::Area::new(egui::Id::new("context_menu"))
        .fixed_pos(pos)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(color_scheme.popup_background())
                .stroke(egui::Stroke::new(1.0, color_scheme.popup_border()))
                .rounding(6.0)
                .inner_margin(egui::Margin::symmetric(6.0, 6.0))
                .show(ui, |ui| {
                    ui.set_width(menu_width - 12.0);
                    ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 2.0);
                    ui.style_mut().visuals.widgets.hovered.bg_fill = color_scheme.selection_background();

                    // New Session
                    let btn = egui::Button::new(
                        egui::RichText::new("New Session").size(13.0).color(color_scheme.foreground())
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(menu_width - 12.0, menu_item_height));

                    if ui.add(btn).clicked() {
                        actions.push(TerminalAction::NewTab);
                        close_menu = true;
                    }

                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);

                    // Fresh Session
                    let btn = egui::Button::new(
                        egui::RichText::new("Fresh session from here").size(13.0).color(color_scheme.foreground())
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(menu_width - 12.0, menu_item_height));

                    if ui.add(btn).clicked() {
                        actions.push(TerminalAction::FreshSessionCurrentDir);
                        close_menu = true;
                    }

                    // Load session submenu trigger
                    let label_text = if has_sessions { "Load recent session  >" } else { "Load recent session" };
                    let text_color = if has_sessions { color_scheme.foreground() } else { color_scheme.disabled_foreground() };

                    let btn = egui::Button::new(
                        egui::RichText::new(label_text).size(13.0).color(text_color)
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(menu_width - 12.0, menu_item_height));

                    let load_response = ui.add(btn);
                    load_session_rect = load_response.rect;

                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);

                    // Copy
                    let text_color = if has_selection { color_scheme.foreground() } else { color_scheme.disabled_foreground() };
                    let btn = egui::Button::new(
                        egui::RichText::new("Copy").size(13.0).color(text_color)
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(menu_width - 12.0, menu_item_height));

                    if ui.add(btn).clicked() && has_selection {
                        actions.push(TerminalAction::Copy);
                        close_menu = true;
                    }

                    // Paste
                    let text_color = if has_clipboard { color_scheme.foreground() } else { color_scheme.disabled_foreground() };
                    let btn = egui::Button::new(
                        egui::RichText::new("Paste").size(13.0).color(text_color)
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(menu_width - 12.0, menu_item_height));

                    if ui.add(btn).clicked() && has_clipboard {
                        actions.push(TerminalAction::Paste);
                        close_menu = true;
                    }
                });
        });

    let main_menu_rect = menu_response.response.rect;

    // Determine if submenu should be shown
    let load_session_hovered = ctx.input(|i| {
        i.pointer.hover_pos().map(|p| load_session_rect.contains(p)).unwrap_or(false)
    });

    if has_sessions && load_session_hovered {
        context_menu.submenu_open = true;
    }

    // Render submenu if open
    if context_menu.submenu_open && has_sessions {
        let submenu_x = load_session_rect.right() + 2.0;
        let submenu_y = load_session_rect.top() - 6.0;

        let submenu_width = 260.0;
        let submenu_item_height = 44.0;
        let max_visible_sessions = 8;
        let visible_sessions = submenu_sessions.len().min(max_visible_sessions);
        let submenu_height = visible_sessions as f32 * submenu_item_height + 12.0;

        let mut submenu_pos = egui::pos2(submenu_x, submenu_y);
        if submenu_pos.x + submenu_width > screen_rect.max.x {
            submenu_pos.x = load_session_rect.left() - submenu_width - 2.0;
        }
        if submenu_pos.y + submenu_height > screen_rect.max.y {
            submenu_pos.y = screen_rect.max.y - submenu_height - 10.0;
        }

        let submenu_response = egui::Area::new(egui::Id::new("context_submenu"))
            .fixed_pos(submenu_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(color_scheme.popup_background())
                    .stroke(egui::Stroke::new(1.0, color_scheme.popup_border()))
                    .rounding(6.0)
                    .inner_margin(egui::Margin::symmetric(6.0, 6.0))
                    .show(ui, |ui| {
                        ui.set_width(submenu_width - 12.0);
                        ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 2.0);
                        ui.style_mut().visuals.widgets.hovered.bg_fill = color_scheme.selection_background();

                        egui::ScrollArea::vertical()
                            .id_salt("context_submenu_scroll")
                            .max_height(max_visible_sessions as f32 * submenu_item_height)
                            .show(ui, |ui| {
                                for (idx, session) in submenu_sessions.iter().enumerate() {
                                    let title = session.display_title();
                                    let time_str = session.relative_modified_time();

                                    // Truncate title if too long
                                    let max_title_chars = 35;
                                    let display_title = if title.chars().count() > max_title_chars {
                                        format!("{}...", title.chars().take(max_title_chars - 3).collect::<String>())
                                    } else {
                                        title
                                    };

                                    // Use a vertical layout inside a button-like container
                                    let btn_id = ui.make_persistent_id(format!("session_btn_{}", idx));
                                    let response = ui.push_id(btn_id, |ui| {
                                        let item_width = ui.available_width();
                                        let (rect, response) = ui.allocate_exact_size(
                                            egui::vec2(item_width, submenu_item_height),
                                            egui::Sense::click(),
                                        );

                                        // Draw background on hover
                                        if response.hovered() {
                                            ui.painter().rect_filled(rect, 4.0, color_scheme.selection_background());
                                        }

                                        // Draw title and time with clipping
                                        let clip_rect = rect.shrink(4.0);
                                        ui.painter().with_clip_rect(clip_rect).text(
                                            egui::pos2(rect.left() + 8.0, rect.top() + 14.0),
                                            egui::Align2::LEFT_CENTER,
                                            &display_title,
                                            egui::FontId::proportional(12.0),
                                            if response.hovered() { egui::Color32::WHITE } else { color_scheme.foreground() },
                                        );

                                        ui.painter().with_clip_rect(clip_rect).text(
                                            egui::pos2(rect.left() + 8.0, rect.top() + 32.0),
                                            egui::Align2::LEFT_CENTER,
                                            &time_str,
                                            egui::FontId::proportional(10.0),
                                            if response.hovered() { egui::Color32::from_gray(200) } else { color_scheme.secondary_foreground() },
                                        );

                                        response
                                    }).inner;

                                    if response.clicked() {
                                        actions.push(TerminalAction::LoadSession {
                                            session_id: session.session_id.clone(),
                                        });
                                        close_menu = true;
                                    }
                                }
                            });
                    });
            });

        submenu_rect = submenu_response.response.rect;
    }

    // Check if mouse is outside all menu areas
    let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
    if let Some(mouse) = mouse_pos {
        let expanded_load_rect = load_session_rect.expand(4.0);
        let in_main_menu = main_menu_rect.contains(mouse);
        let in_submenu = submenu_rect.contains(mouse);
        let in_load_item = expanded_load_rect.contains(mouse);

        // Close submenu if mouse is not in relevant areas
        if context_menu.submenu_open && !in_submenu && !in_load_item {
            let gap_rect = egui::Rect::from_min_max(
                egui::pos2(load_session_rect.right(), load_session_rect.top() - 10.0),
                egui::pos2(submenu_rect.left(), load_session_rect.bottom() + 10.0),
            );
            if !gap_rect.contains(mouse) && !in_main_menu {
                context_menu.submenu_open = false;
            }
        }

        // Close menu on left-click outside (after grace period)
        if can_close {
            let left_clicked = ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
            if left_clicked && !in_main_menu && !in_submenu {
                close_menu = true;
            }
        }
    }

    if close_menu {
        context_menu.is_open = false;
        context_menu.submenu_open = false;
        context_menu.opened_time = 0.0;
    }

    actions
}
