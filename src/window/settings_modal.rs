//! Settings modal dialog
//!
//! A modal dialog for editing application settings and soft key configuration.
//! Contains two tabs: General (font settings) and Soft Keys (macropad config).

use crate::core::settings::{Settings, MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::hid::keycodes::{self, QmkKeycode};
use crate::hid::soft_keys::{self, is_builtin_preset_name, KeycodeEntry, PresetManager, SoftKeyEditState};

/// Which settings tab is active
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    SoftKeys,
}

/// Which key entry is being captured via keyboard
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureTarget {
    /// Which of the 3 soft keys (0-2)
    key_index: usize,
    /// None for single Keycode type, Some(n) for Sequence step n
    step_index: Option<usize>,
}

/// Preset edit mode for inline editing in the preset area
#[derive(Debug, Clone)]
enum PresetEditMode {
    None,
    /// Saving a new/updated preset (name being typed)
    Saving(String),
    /// Renaming a preset (original_name, new_name being typed)
    Renaming(String, String),
    /// Confirming deletion (name to delete)
    ConfirmDelete(String),
}

/// State for the settings modal
pub struct SettingsModal {
    /// Whether the modal is open
    pub is_open: bool,
    /// Working copy of settings (edited in the modal)
    pub working_settings: Settings,
    /// Original settings (to revert on cancel)
    original_settings: Settings,
    /// Active tab
    active_tab: SettingsTab,
    /// Working copy of soft keys from device
    soft_keys: Option<[SoftKeyEditState; 3]>,
    /// Original soft keys from device (for dirty checking)
    original_soft_keys: Option<[SoftKeyEditState; 3]>,
    /// Whether we've sent a read request
    soft_keys_requested: bool,
    /// Error from device communication
    soft_keys_error: Option<String>,
    /// Currently capturing keyboard input for this target
    capturing_key: Option<CaptureTarget>,
    /// User preset manager
    preset_manager: PresetManager,
    /// Current preset edit mode
    preset_edit: PresetEditMode,
}

impl SettingsModal {
    /// Create a new settings modal (closed)
    pub fn new(settings: Settings) -> Self {
        Self {
            is_open: false,
            working_settings: settings.clone(),
            original_settings: settings,
            active_tab: SettingsTab::General,
            soft_keys: None,
            original_soft_keys: None,
            soft_keys_requested: false,
            soft_keys_error: None,
            capturing_key: None,
            preset_manager: PresetManager::load().unwrap_or_default(),
            preset_edit: PresetEditMode::None,
        }
    }

    /// Open the modal with current settings
    pub fn open(&mut self, current_settings: &Settings) {
        self.original_settings = current_settings.clone();
        self.working_settings = current_settings.clone();
        self.is_open = true;
        self.active_tab = SettingsTab::General;
        self.soft_keys = None;
        self.original_soft_keys = None;
        self.soft_keys_requested = false;
        self.soft_keys_error = None;
        self.capturing_key = None;
        self.preset_manager = PresetManager::load().unwrap_or_default();
        self.preset_edit = PresetEditMode::None;
    }

    /// Close the modal without saving
    pub fn close(&mut self) {
        self.is_open = false;
        self.working_settings = self.original_settings.clone();
        self.active_tab = SettingsTab::General;
        self.capturing_key = None;
        self.preset_edit = PresetEditMode::None;
    }

    /// Check if general settings have been modified
    pub fn is_modified(&self) -> bool {
        self.working_settings.font_family != self.original_settings.font_family
            || self.working_settings.font_size != self.original_settings.font_size
    }

    /// Store soft keys read from device
    pub fn set_soft_keys(&mut self, keys: [SoftKeyEditState; 3]) {
        self.original_soft_keys = Some(keys.clone());
        self.soft_keys = Some(keys);
        self.soft_keys_error = None;
    }

    /// Store error from device communication
    pub fn set_soft_keys_error(&mut self, err: String) {
        self.soft_keys_error = Some(err);
    }

    /// Check if soft keys have been modified from device state
    pub fn soft_keys_modified(&self) -> bool {
        match (&self.soft_keys, &self.original_soft_keys) {
            (Some(current), Some(original)) => current != original,
            _ => false,
        }
    }
}

/// Result of rendering the settings modal
#[derive(Debug, Clone)]
pub enum SettingsModalResult {
    /// Modal is still open, no action
    None,
    /// User clicked Apply (general settings)
    Apply(Settings),
    /// User clicked Cancel
    Cancel,
    /// Request to read soft keys from device
    ReadSoftKeys,
    /// Apply soft keys to device
    ApplySoftKeys([SoftKeyEditState; 3]),
    /// Reset soft keys to firmware defaults
    ResetSoftKeys,
}

impl PartialEq for SettingsModalResult {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Cancel, Self::Cancel) => true,
            (Self::Apply(a), Self::Apply(b)) => a.font_family == b.font_family && a.font_size == b.font_size,
            (Self::ReadSoftKeys, Self::ReadSoftKeys) => true,
            (Self::ResetSoftKeys, Self::ResetSoftKeys) => true,
            _ => false,
        }
    }
}

/// The 4 type names for the soft key type combo box
const SOFT_KEY_TYPE_NAMES: [&str; 4] = ["Default", "Key", "Text", "Sequence"];

/// Render the settings modal and return the result
pub fn render_settings_modal(
    ctx: &egui::Context,
    modal: &mut SettingsModal,
    hid_connected: bool,
) -> SettingsModalResult {
    if !modal.is_open {
        return SettingsModalResult::None;
    }

    let mut result = SettingsModalResult::None;

    // Process key capture events before rendering
    let was_capturing = modal.capturing_key.is_some();
    if was_capturing {
        let events = ctx.input(|i| i.events.clone());
        for event in &events {
            if let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = event {
                let effective_key = physical_key.unwrap_or(*key);
                if let Some(qmk_key) = keycodes::from_egui_key(effective_key) {
                    let mods = keycodes::from_egui_modifiers(modifiers);
                    let entry = KeycodeEntry::with_mods(qmk_key, mods);
                    if let (Some(target), Some(ref mut keys)) =
                        (modal.capturing_key, &mut modal.soft_keys)
                    {
                        match target.step_index {
                            None => {
                                keys[target.key_index] = SoftKeyEditState::Keycode(entry);
                            }
                            Some(step) => {
                                if let SoftKeyEditState::Sequence(ref mut seq) =
                                    keys[target.key_index]
                                {
                                    if step < seq.len() {
                                        seq[step] = entry;
                                    }
                                }
                            }
                        }
                    }
                    modal.capturing_key = None;
                    break;
                }
            }
        }
    }

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
    let modal_size = if modal.active_tab == SettingsTab::SoftKeys {
        [520.0, 450.0]
    } else {
        [400.0, 230.0]
    };

    egui::Window::new("Settings")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size(modal_size)
        .show(ctx, |ui| {
            // Tab bar
            ui.horizontal(|ui| {
                let general_text = egui::RichText::new("General").size(13.0);
                let soft_keys_text = egui::RichText::new("Soft Keys").size(13.0);

                let general_text = if modal.active_tab == SettingsTab::General {
                    general_text.strong()
                } else {
                    general_text
                };
                let soft_keys_text = if modal.active_tab == SettingsTab::SoftKeys {
                    soft_keys_text.strong()
                } else {
                    soft_keys_text
                };

                if ui.selectable_label(modal.active_tab == SettingsTab::General, general_text).clicked() {
                    modal.active_tab = SettingsTab::General;
                    modal.capturing_key = None;
                }
                if ui.selectable_label(modal.active_tab == SettingsTab::SoftKeys, soft_keys_text).clicked() {
                    modal.active_tab = SettingsTab::SoftKeys;
                    modal.capturing_key = None;
                }
            });

            ui.separator();

            match modal.active_tab {
                SettingsTab::General => {
                    result = render_general_tab(ui, modal);
                }
                SettingsTab::SoftKeys => {
                    result = render_soft_keys_tab(ui, modal, hid_connected);
                }
            }
        });

    // Close on Escape (skip if capturing or just finished capturing this frame)
    if !was_capturing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        modal.close();
        result = SettingsModalResult::Cancel;
    }

    result
}

/// Render the General settings tab
fn render_general_tab(ui: &mut egui::Ui, modal: &mut SettingsModal) -> SettingsModalResult {
    let mut result = SettingsModalResult::None;

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
                .step_by(1.0)
                .fixed_decimals(0),
            );
        });
    });

    ui.add_space(20.0);

    // Color theme hint
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(
            egui::RichText::new("Color theme is synced with Claude Code. Use ")
                .size(12.0)
                .color(egui::Color32::GRAY),
        );
        ui.label(
            egui::RichText::new("/theme")
                .size(12.0)
                .color(egui::Color32::GRAY)
                .code(),
        );
        ui.label(
            egui::RichText::new(" command in Claude Code to change it.")
                .size(12.0)
                .color(egui::Color32::GRAY),
        );
    });

    ui.add_space(15.0);

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

    result
}

/// Render the Soft Keys configuration tab
fn render_soft_keys_tab(
    ui: &mut egui::Ui,
    modal: &mut SettingsModal,
    hid_connected: bool,
) -> SettingsModalResult {
    let mut result = SettingsModalResult::None;

    if !hid_connected {
        // Show "not connected" message
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("Connect Agent Deck to configure soft keys.")
                    .size(14.0)
                    .color(egui::Color32::GRAY),
            );
        });
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    modal.close();
                    result = SettingsModalResult::Cancel;
                }
            });
        });
        return result;
    }

    // Check if we need to request data from device
    if modal.soft_keys.is_none() && !modal.soft_keys_requested {
        modal.soft_keys_requested = true;
        return SettingsModalResult::ReadSoftKeys;
    }

    // Show error if any
    if let Some(err) = modal.soft_keys_error.clone() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new(format!("Error: {}", err))
                    .size(13.0)
                    .color(egui::Color32::from_rgb(200, 80, 80)),
            );
            ui.add_space(10.0);
            if ui.button("Retry").clicked() {
                modal.soft_keys_requested = false;
                modal.soft_keys_error = None;
            }
        });
        return result;
    }

    // Show loading if not yet loaded
    if modal.soft_keys.is_none() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.spinner();
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Reading soft key configuration...")
                    .size(13.0)
                    .color(egui::Color32::GRAY),
            );
        });
        return result;
    }

    // We have soft keys loaded — render editors
    let all_presets = soft_keys::presets();

    // Preset selector area
    ui.add_space(6.0);
    render_preset_area(ui, modal, &all_presets);

    ui.add_space(6.0);
    ui.separator();

    // Soft key editors in scroll area
    let mut capturing = modal.capturing_key;
    egui::ScrollArea::vertical()
        .max_height(280.0)
        .show(ui, |ui| {
            let keys = modal.soft_keys.as_mut().unwrap();
            for i in 0..3 {
                ui.add_space(6.0);
                ui.group(|ui| {
                    ui.set_min_width(ui.available_width() - 4.0);
                    render_soft_key_editor(ui, i, &mut keys[i], &mut capturing);
                });
            }
            ui.add_space(4.0);
        });
    modal.capturing_key = capturing;

    ui.add_space(4.0);

    // Bottom buttons
    ui.horizontal(|ui| {
        if ui.button("Reset to Defaults").clicked() {
            result = SettingsModalResult::ResetSoftKeys;
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Cancel").clicked() {
                modal.close();
                result = SettingsModalResult::Cancel;
            }

            if ui
                .add_enabled(
                    modal.soft_keys_modified(),
                    egui::Button::new("Apply to Device"),
                )
                .clicked()
            {
                if let Some(ref keys) = modal.soft_keys {
                    result = SettingsModalResult::ApplySoftKeys(keys.clone());
                    // Update original to reflect saved state
                    modal.original_soft_keys = modal.soft_keys.clone();
                }
            }
        });
    });

    result
}

/// Render editor for a single soft key
fn render_soft_key_editor(
    ui: &mut egui::Ui,
    index: usize,
    key: &mut SoftKeyEditState,
    capturing: &mut Option<CaptureTarget>,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("Soft Key {}", index + 1))
                .size(12.0)
                .strong(),
        );

        ui.add_space(8.0);

        // Type selector
        let current_type_idx = key.type_index();
        let mut selected_type = current_type_idx;
        egui::ComboBox::from_id_salt(format!("sk_type_{}", index))
            .selected_text(SOFT_KEY_TYPE_NAMES[current_type_idx])
            .width(90.0)
            .show_ui(ui, |ui| {
                for (i, name) in SOFT_KEY_TYPE_NAMES.iter().enumerate() {
                    ui.selectable_value(&mut selected_type, i, *name);
                }
            });

        if selected_type != current_type_idx {
            *key = SoftKeyEditState::default_for_type(selected_type);
            *capturing = None;
        }
    });

    ui.add_space(4.0);

    // Type-specific editor
    match key {
        SoftKeyEditState::Default(ref resolved) => {
            let label = match resolved {
                Some(entry) => format!("Uses firmware default ({})", entry.display()),
                None => "Uses firmware default".to_string(),
            };
            ui.label(
                egui::RichText::new(label)
                    .size(12.0)
                    .color(egui::Color32::GRAY),
            );
        }
        SoftKeyEditState::Keycode(entry) => {
            render_keycode_editor(ui, index, None, entry, capturing);
        }
        SoftKeyEditState::Text(text, send_enter) => {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(text)
                        .desired_width(220.0)
                        .hint_text("Text to type..."),
                );
                // Enforce max length
                if text.len() > 126 {
                    text.truncate(126);
                    // Find valid UTF-8 boundary
                    while !text.is_char_boundary(text.len()) {
                        text.pop();
                    }
                    response.surrender_focus();
                }
                ui.label(
                    egui::RichText::new(format!("{}/126", text.len()))
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                );
                ui.checkbox(send_enter, "Enter");
            });
        }
        SoftKeyEditState::Sequence(entries) => {
            render_sequence_editor(ui, index, entries, capturing);
        }
    }
}

/// Render keycode editor as a clickable capture button
fn render_keycode_editor(
    ui: &mut egui::Ui,
    key_index: usize,
    step_index: Option<usize>,
    entry: &mut KeycodeEntry,
    capturing: &mut Option<CaptureTarget>,
) {
    let target = CaptureTarget { key_index, step_index };
    let is_capturing = *capturing == Some(target);

    ui.horizontal(|ui| {
        if is_capturing {
            // Show highlighted "Press a key..." prompt
            let btn = egui::Button::new(
                egui::RichText::new("Press a key...")
                    .size(12.0)
                    .color(egui::Color32::WHITE),
            )
            .fill(egui::Color32::from_rgb(60, 100, 180))
            .min_size(egui::vec2(140.0, 24.0));
            if ui.add(btn).clicked() {
                *capturing = None;
            }
        } else {
            // Show current key+mods as a clickable button
            let label = entry.display();
            let btn = egui::Button::new(
                egui::RichText::new(&label).size(12.0),
            )
            .min_size(egui::vec2(140.0, 24.0));
            if ui.add(btn).clicked() {
                *capturing = Some(target);
            }
        }
    });
}

/// Render sequence editor (list of keycode entries with add/remove)
fn render_sequence_editor(
    ui: &mut egui::Ui,
    key_index: usize,
    entries: &mut Vec<KeycodeEntry>,
    capturing: &mut Option<CaptureTarget>,
) {
    let mut remove_idx = None;

    for (i, entry) in entries.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("{}.", i + 1))
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );

            render_keycode_editor(ui, key_index, Some(i), entry, capturing);

            if ui
                .add(
                    egui::Button::new(egui::RichText::new("-").size(12.0))
                        .min_size(egui::vec2(20.0, 18.0)),
                )
                .clicked()
            {
                remove_idx = Some(i);
            }
        });
    }

    if let Some(idx) = remove_idx {
        // If we were capturing this step or a later one, cancel capture
        if let Some(ref target) = capturing {
            if target.key_index == key_index {
                if let Some(step) = target.step_index {
                    if step >= idx {
                        *capturing = None;
                    }
                }
            }
        }
        entries.remove(idx);
    }

    if entries.len() < 63 {
        if ui
            .add(egui::Button::new(
                egui::RichText::new("+ Add Step").size(11.0),
            ))
            .clicked()
        {
            entries.push(KeycodeEntry::new(QmkKeycode::A));
        }
    }
}

/// Render the preset selector area with ComboBox and action buttons
fn render_preset_area(
    ui: &mut egui::Ui,
    modal: &mut SettingsModal,
    builtin_presets: &[soft_keys::SoftKeyPreset],
) {
    let current_keys = modal.soft_keys.as_ref().unwrap();

    // Detect current preset match
    let current_match = find_current_preset_match(current_keys, builtin_presets, &modal.preset_manager);
    let selected_text = match &current_match {
        PresetMatch::Builtin(name) | PresetMatch::User(name) => name.clone(),
        PresetMatch::Custom => "Custom".to_string(),
    };

    // Check if current match is a user preset
    let is_user_preset = matches!(&current_match, PresetMatch::User(_));

    match modal.preset_edit.clone() {
        PresetEditMode::None => {
            // Normal mode: ComboBox + action buttons
            ui.horizontal(|ui| {
                ui.label("Preset:");

                egui::ComboBox::from_id_salt("soft_key_preset")
                    .selected_text(&selected_text)
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        // Built-in presets
                        for preset in builtin_presets {
                            let label = format!("{} - {}", preset.name, preset.description);
                            if ui.selectable_label(false, &label).clicked() {
                                modal.soft_keys = Some(preset.keys.clone());
                            }
                        }
                        // Separator if there are user presets
                        if !modal.preset_manager.all().is_empty() {
                            ui.separator();
                            for user_preset in modal.preset_manager.all() {
                                if ui.selectable_label(false, &user_preset.name).clicked() {
                                    modal.soft_keys = Some(user_preset.keys.clone());
                                }
                            }
                        }
                    });

                // Save button (always enabled)
                if ui.button("Save").clicked() {
                    let prefill = match &current_match {
                        PresetMatch::User(name) => name.clone(),
                        _ => String::new(),
                    };
                    modal.preset_edit = PresetEditMode::Saving(prefill);
                }

                // Rename button (only for user presets)
                if ui
                    .add_enabled(is_user_preset, egui::Button::new("Rename"))
                    .clicked()
                {
                    if let PresetMatch::User(name) = &current_match {
                        modal.preset_edit =
                            PresetEditMode::Renaming(name.clone(), name.clone());
                    }
                }

                // Delete button (only for user presets)
                if ui
                    .add_enabled(is_user_preset, egui::Button::new("Delete"))
                    .clicked()
                {
                    if let PresetMatch::User(name) = &current_match {
                        modal.preset_edit = PresetEditMode::ConfirmDelete(name.clone());
                    }
                }
            });
        }

        PresetEditMode::Saving(ref name) => {
            let mut name = name.clone();
            let mut done = false;
            ui.horizontal(|ui| {
                ui.label("Save as:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(200.0)
                        .hint_text("Preset name..."),
                );

                // Auto-focus the text input
                if response.gained_focus() || ui.memory(|m| m.has_focus(response.id)) {
                    // Already focused
                } else {
                    response.request_focus();
                }

                let name_valid = !name.trim().is_empty() && !is_builtin_preset_name(name.trim());

                if ui.add_enabled(name_valid, egui::Button::new("OK")).clicked()
                    || (response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && name_valid)
                {
                    let trimmed = name.trim().to_string();
                    if let Some(ref keys) = modal.soft_keys {
                        modal.preset_manager.add(trimmed, keys.clone());
                        let _ = modal.preset_manager.save();
                    }
                    done = true;
                }

                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
            if done {
                modal.preset_edit = PresetEditMode::None;
            } else {
                modal.preset_edit = PresetEditMode::Saving(name);
            }
        }

        PresetEditMode::Renaming(ref original, ref new_name) => {
            let original = original.clone();
            let mut new_name = new_name.clone();
            let mut done = false;
            ui.horizontal(|ui| {
                ui.label("Rename:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut new_name)
                        .desired_width(200.0)
                        .hint_text("New name..."),
                );

                if response.gained_focus() || ui.memory(|m| m.has_focus(response.id)) {
                    // Already focused
                } else {
                    response.request_focus();
                }

                let trimmed = new_name.trim();
                let name_valid = !trimmed.is_empty()
                    && !is_builtin_preset_name(trimmed)
                    && (trimmed == original
                        || modal.preset_manager.find(trimmed).is_none());

                if ui.add_enabled(name_valid, egui::Button::new("OK")).clicked()
                    || (response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && name_valid)
                {
                    let trimmed = new_name.trim().to_string();
                    modal.preset_manager.rename(&original, trimmed);
                    let _ = modal.preset_manager.save();
                    done = true;
                }

                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
            if done {
                modal.preset_edit = PresetEditMode::None;
            } else {
                modal.preset_edit = PresetEditMode::Renaming(original, new_name);
            }
        }

        PresetEditMode::ConfirmDelete(ref name) => {
            let name = name.clone();
            let mut done = false;
            ui.horizontal(|ui| {
                ui.label(format!("Delete \"{}\"?", name));

                if ui.button("Yes").clicked() {
                    modal.preset_manager.remove(&name);
                    let _ = modal.preset_manager.save();
                    done = true;
                }

                if ui.button("No").clicked() {
                    done = true;
                }
            });
            if done {
                modal.preset_edit = PresetEditMode::None;
            }
        }
    }
}

/// What the current keys match
enum PresetMatch {
    Builtin(String),
    User(String),
    Custom,
}

/// Find which preset (built-in or user) matches the current keys
fn find_current_preset_match(
    keys: &[SoftKeyEditState; 3],
    builtin_presets: &[soft_keys::SoftKeyPreset],
    preset_manager: &PresetManager,
) -> PresetMatch {
    // Check built-in presets first
    for preset in builtin_presets {
        if keys_match_preset(&preset.keys, keys) {
            return PresetMatch::Builtin(preset.name.to_string());
        }
    }
    // Check user presets
    for user_preset in preset_manager.all() {
        if keys_match_preset(&user_preset.keys, keys) {
            return PresetMatch::User(user_preset.name.clone());
        }
    }
    PresetMatch::Custom
}

/// Compare two soft key configs, treating all Default variants as equal
/// (ignoring the resolved keycode which is display-only)
fn keys_match_preset(preset: &[SoftKeyEditState; 3], current: &[SoftKeyEditState; 3]) -> bool {
    preset.iter().zip(current.iter()).all(|(a, b)| match (a, b) {
        (SoftKeyEditState::Default(_), SoftKeyEditState::Default(_)) => true,
        _ => a == b,
    })
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
        assert_eq!(modal.active_tab, SettingsTab::General);

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

    #[test]
    fn test_soft_keys_modified() {
        let settings = Settings::default();
        let mut modal = SettingsModal::new(settings);

        assert!(!modal.soft_keys_modified());

        let keys = [
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
        ];
        modal.set_soft_keys(keys.clone());
        assert!(!modal.soft_keys_modified());

        // Modify one key
        if let Some(ref mut k) = modal.soft_keys {
            k[0] = SoftKeyEditState::Text("test".to_string(), false);
        }
        assert!(modal.soft_keys_modified());
    }

    #[test]
    fn test_open_resets_soft_key_state() {
        let settings = Settings::default();
        let mut modal = SettingsModal::new(settings.clone());

        // Simulate having loaded soft keys previously
        modal.soft_keys_requested = true;
        modal.soft_keys = Some([
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
        ]);

        // Re-open should reset
        modal.open(&settings);
        assert!(modal.soft_keys.is_none());
        assert!(!modal.soft_keys_requested);
        assert_eq!(modal.active_tab, SettingsTab::General);
    }
}
