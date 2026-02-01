//! Settings modal dialog
//!
//! A modal dialog for editing application settings like font and color scheme.

use crate::core::settings::{ColorScheme, Settings, MAX_FONT_SIZE, MIN_FONT_SIZE};

/// State for the settings modal
pub struct SettingsModal {
    /// Whether the modal is open
    pub is_open: bool,
    /// Working copy of settings (edited in the modal)
    pub working_settings: Settings,
    /// Original settings (to revert on cancel)
    original_settings: Settings,
}

impl SettingsModal {
    /// Create a new settings modal (closed)
    pub fn new(settings: Settings) -> Self {
        Self {
            is_open: false,
            working_settings: settings.clone(),
            original_settings: settings,
        }
    }

    /// Open the modal with current settings
    pub fn open(&mut self, current_settings: &Settings) {
        self.original_settings = current_settings.clone();
        self.working_settings = current_settings.clone();
        self.is_open = true;
    }

    /// Close the modal without saving
    pub fn close(&mut self) {
        self.is_open = false;
        self.working_settings = self.original_settings.clone();
    }

    /// Check if settings have been modified
    pub fn is_modified(&self) -> bool {
        self.working_settings.font_family != self.original_settings.font_family
            || self.working_settings.font_size != self.original_settings.font_size
            || self.working_settings.color_scheme != self.original_settings.color_scheme
    }
}

/// Result of rendering the settings modal
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsModalResult {
    /// Modal is still open, no action
    None,
    /// User clicked Apply
    Apply(Settings),
    /// User clicked Cancel
    Cancel,
}

/// Render the settings modal and return the result
pub fn render_settings_modal(
    ctx: &egui::Context,
    modal: &mut SettingsModal,
) -> SettingsModalResult {
    if !modal.is_open {
        return SettingsModalResult::None;
    }

    let mut result = SettingsModalResult::None;

    // Modal background overlay
    egui::Area::new(egui::Id::new("settings_modal_backdrop"))
        .fixed_pos(egui::pos2(0.0, 0.0))
        .show(ctx, |ui| {
            let screen_rect = ctx.screen_rect();
            ui.allocate_rect(screen_rect, egui::Sense::click());
            ui.painter().rect_filled(
                screen_rect,
                0.0,
                egui::Color32::from_black_alpha(128),
            );
        });

    // Modal window
    egui::Window::new("Settings")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([400.0, 300.0])
        .show(ctx, |ui| {
            ui.add_space(10.0);

            // Font Family
            ui.horizontal(|ui| {
                ui.label("Font Family:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("font_family")
                        .selected_text(&modal.working_settings.font_family)
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for font in Settings::available_fonts() {
                                ui.selectable_value(
                                    &mut modal.working_settings.font_family,
                                    font.to_string(),
                                    *font,
                                );
                            }
                        });
                });
            });

            ui.add_space(15.0);

            // Font Size
            ui.horizontal(|ui| {
                ui.label("Font Size:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Slider::new(
                            &mut modal.working_settings.font_size,
                            MIN_FONT_SIZE..=MAX_FONT_SIZE,
                        )
                        .suffix(" pt")
                        .fixed_decimals(1),
                    );
                });
            });

            ui.add_space(15.0);

            // Color Scheme
            ui.horizontal(|ui| {
                ui.label("Color Scheme:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("color_scheme")
                        .selected_text(modal.working_settings.color_scheme.display_name())
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for scheme in ColorScheme::all() {
                                ui.selectable_value(
                                    &mut modal.working_settings.color_scheme,
                                    *scheme,
                                    scheme.display_name(),
                                );
                            }
                        });
                });
            });

            ui.add_space(30.0);

            // Preview
            ui.group(|ui| {
                ui.label("Preview:");
                ui.add_space(5.0);

                let bg = modal.working_settings.color_scheme.background();
                let fg = modal.working_settings.color_scheme.foreground();

                let preview_rect = ui.available_rect_before_wrap();
                let preview_rect = egui::Rect::from_min_size(
                    preview_rect.min,
                    egui::vec2(ui.available_width(), 60.0),
                );

                ui.painter().rect_filled(preview_rect, 4.0, bg);

                let font_id = egui::FontId::monospace(modal.working_settings.font_size);
                ui.painter().text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Hello, Claude!",
                    font_id,
                    fg,
                );

                ui.allocate_rect(preview_rect, egui::Sense::hover());
            });

            ui.add_space(20.0);

            // Buttons
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        modal.close();
                        result = SettingsModalResult::Cancel;
                    }

                    if ui
                        .add_enabled(modal.is_modified(), egui::Button::new("Apply"))
                        .clicked()
                    {
                        modal.is_open = false;
                        result = SettingsModalResult::Apply(modal.working_settings.clone());
                    }
                });
            });
        });

    // Close on Escape
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        modal.close();
        result = SettingsModalResult::Cancel;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_modal_open_close() {
        let settings = Settings::default();
        let mut modal = SettingsModal::new(settings.clone());

        assert!(!modal.is_open);

        modal.open(&settings);
        assert!(modal.is_open);
        assert!(!modal.is_modified());

        modal.close();
        assert!(!modal.is_open);
    }

    #[test]
    fn test_settings_modal_is_modified() {
        let settings = Settings::default();
        let mut modal = SettingsModal::new(settings.clone());

        modal.open(&settings);
        assert!(!modal.is_modified());

        modal.working_settings.font_size = 20.0;
        assert!(modal.is_modified());

        modal.close();
        assert!(!modal.is_modified());
    }
}
