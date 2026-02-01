//! New tab page UI
//!
//! Displays bookmarks, recent directories, and directory picker for new tabs.

use crate::core::bookmarks::{Bookmark, BookmarkManager, RecentEntry};
use crate::core::settings::ColorScheme;
use std::path::PathBuf;

/// Actions that can be triggered from the new tab page
#[derive(Debug, Clone)]
pub enum NewTabAction {
    /// Open a directory and start Claude there (with resume flag: true = continue, false = fresh)
    OpenDirectory { path: PathBuf, resume: bool },
    /// Open the native directory picker
    BrowseDirectory,
    /// Add a bookmark
    AddBookmark(PathBuf),
    /// Remove a bookmark
    RemoveBookmark(PathBuf),
    /// Remove a recent entry
    RemoveRecent(PathBuf),
    /// Clear all recent entries
    ClearRecent,
}

/// Renders the new tab page and returns any action triggered
pub fn render_new_tab_page(
    ui: &mut egui::Ui,
    bookmark_manager: &BookmarkManager,
    color_scheme: ColorScheme,
) -> Option<NewTabAction> {
    let mut action = None;

    let fg_color = color_scheme.foreground();

    // Get/set fresh session toggle state from egui memory
    let fresh_session_id = egui::Id::new("new_tab_fresh_session");
    let mut fresh_session = ui.data_mut(|d| *d.get_temp_mut_or(fresh_session_id, false));

    // Center the content
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);

        // Title
        ui.label(
            egui::RichText::new("New Tab")
                .size(28.0)
                .color(fg_color)
                .strong(),
        );

        ui.add_space(20.0);

        // Session mode toggle
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Session mode:")
                    .size(14.0)
                    .color(egui::Color32::GRAY),
            );
            ui.add_space(10.0);

            // Resume button
            let resume_selected = !fresh_session;
            if ui
                .add(
                    egui::SelectableLabel::new(
                        resume_selected,
                        egui::RichText::new("Resume conversation")
                            .size(13.0),
                    ),
                )
                .clicked()
            {
                fresh_session = false;
            }

            ui.add_space(5.0);

            // Fresh button
            let fresh_selected = fresh_session;
            if ui
                .add(
                    egui::SelectableLabel::new(
                        fresh_selected,
                        egui::RichText::new("Fresh session")
                            .size(13.0),
                    ),
                )
                .clicked()
            {
                fresh_session = true;
            }
        });

        ui.add_space(20.0);

        // Browse button
        if ui
            .add(
                egui::Button::new(
                    egui::RichText::new("Browse for directory...")
                        .size(16.0)
                        .color(fg_color),
                )
                .min_size(egui::vec2(200.0, 40.0))
                .fill(color_scheme.active_tab_background()),
            )
            .clicked()
        {
            action = Some(NewTabAction::BrowseDirectory);
        }

        ui.add_space(40.0);

        // Two-column layout for bookmarks and recent
        let available_width = ui.available_width();
        let column_width = (available_width - 40.0) / 2.0;

        ui.horizontal(|ui| {
            ui.add_space(20.0);

            // Bookmarks column
            ui.vertical(|ui| {
                ui.set_width(column_width);

                ui.label(
                    egui::RichText::new("Bookmarks")
                        .size(18.0)
                        .color(fg_color)
                        .strong(),
                );
                ui.add_space(10.0);

                if bookmark_manager.get_bookmarks().is_empty() {
                    ui.label(
                        egui::RichText::new("No bookmarks yet")
                            .size(14.0)
                            .color(egui::Color32::GRAY)
                            .italics(),
                    );
                    ui.add_space(5.0);
                    ui.label(
                        egui::RichText::new("Star a directory to add it here")
                            .size(12.0)
                            .color(egui::Color32::GRAY),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for bookmark in bookmark_manager.get_bookmarks() {
                                if let Some(a) =
                                    render_bookmark_item(ui, bookmark, color_scheme, fresh_session)
                                {
                                    action = Some(a);
                                }
                            }
                        });
                }
            });

            // Recent column
            ui.vertical(|ui| {
                ui.set_width(column_width);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Recent")
                            .size(18.0)
                            .color(fg_color)
                            .strong(),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if !bookmark_manager.get_recent().is_empty() {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("Clear")
                                            .size(12.0)
                                            .color(egui::Color32::GRAY),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                action = Some(NewTabAction::ClearRecent);
                            }
                        }
                    });
                });

                ui.add_space(10.0);

                if bookmark_manager.get_recent().is_empty() {
                    ui.label(
                        egui::RichText::new("No recent directories")
                            .size(14.0)
                            .color(egui::Color32::GRAY)
                            .italics(),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for recent in bookmark_manager.get_recent() {
                                let is_bookmarked =
                                    bookmark_manager.is_bookmarked(&recent.path);
                                if let Some(a) = render_recent_item(
                                    ui,
                                    recent,
                                    is_bookmarked,
                                    color_scheme,
                                    fresh_session,
                                ) {
                                    action = Some(a);
                                }
                            }
                        });
                }
            });

            ui.add_space(20.0);
        });

        ui.add_space(40.0);

        // SSH placeholder
        ui.separator();
        ui.add_space(20.0);

        ui.label(
            egui::RichText::new("SSH Connections")
                .size(18.0)
                .color(egui::Color32::GRAY)
                .strong(),
        );
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Coming soon...")
                .size(14.0)
                .color(egui::Color32::DARK_GRAY)
                .italics(),
        );
    });

    // Save fresh_session state back to memory
    ui.data_mut(|d| d.insert_temp(fresh_session_id, fresh_session));

    action
}

/// Render a bookmark item and return any action triggered
fn render_bookmark_item(
    ui: &mut egui::Ui,
    bookmark: &Bookmark,
    color_scheme: ColorScheme,
    fresh_session_toggle: bool,
) -> Option<NewTabAction> {
    let mut action = None;

    // Check if Option/Alt is held (for toggling fresh session)
    let alt_held = ui.input(|i| i.modifiers.alt);
    // Alt key inverts the toggle setting
    let resume = if alt_held { fresh_session_toggle } else { !fresh_session_toggle };

    let response = ui.horizontal(|ui| {
        // Star icon (filled for bookmarks)
        if ui
            .add(
                egui::Button::new(egui::RichText::new("★").size(14.0).color(egui::Color32::GOLD))
                    .frame(false),
            )
            .on_hover_text("Remove bookmark")
            .clicked()
        {
            action = Some(NewTabAction::RemoveBookmark(bookmark.path.clone()));
        }

        // Directory name (clickable)
        let name = bookmark.display_name();
        let text = egui::RichText::new(&name)
            .size(14.0)
            .color(color_scheme.foreground());

        if ui
            .add(egui::Button::new(text).frame(false))
            .on_hover_text(bookmark.path.display().to_string())
            .clicked()
        {
            action = Some(NewTabAction::OpenDirectory { path: bookmark.path.clone(), resume });
        }
    });

    // Make the whole row clickable
    if response.response.clicked() && action.is_none() {
        action = Some(NewTabAction::OpenDirectory { path: bookmark.path.clone(), resume });
    }

    action
}

/// Render a recent item and return any action triggered
fn render_recent_item(
    ui: &mut egui::Ui,
    recent: &RecentEntry,
    is_bookmarked: bool,
    color_scheme: ColorScheme,
    fresh_session_toggle: bool,
) -> Option<NewTabAction> {
    let mut action = None;

    // Check if Option/Alt is held (for toggling fresh session)
    let alt_held = ui.input(|i| i.modifiers.alt);
    // Alt key inverts the toggle setting
    let resume = if alt_held { fresh_session_toggle } else { !fresh_session_toggle };

    let response = ui.horizontal(|ui| {
        // Star icon (toggle bookmark)
        let star = if is_bookmarked { "★" } else { "☆" };
        let star_color = if is_bookmarked {
            egui::Color32::GOLD
        } else {
            egui::Color32::GRAY
        };

        if ui
            .add(
                egui::Button::new(egui::RichText::new(star).size(14.0).color(star_color))
                    .frame(false),
            )
            .on_hover_text(if is_bookmarked {
                "Remove bookmark"
            } else {
                "Add bookmark"
            })
            .clicked()
        {
            if is_bookmarked {
                action = Some(NewTabAction::RemoveBookmark(recent.path.clone()));
            } else {
                action = Some(NewTabAction::AddBookmark(recent.path.clone()));
            }
        }

        // Directory name (clickable)
        let name = recent.display_name();
        let text = egui::RichText::new(&name)
            .size(14.0)
            .color(color_scheme.foreground());

        if ui
            .add(egui::Button::new(text).frame(false))
            .on_hover_text(recent.path.display().to_string())
            .clicked()
        {
            action = Some(NewTabAction::OpenDirectory { path: recent.path.clone(), resume });
        }

        // Close button
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("×").size(14.0).color(egui::Color32::GRAY),
                    )
                    .frame(false),
                )
                .on_hover_text("Remove from recent")
                .clicked()
            {
                action = Some(NewTabAction::RemoveRecent(recent.path.clone()));
            }
        });
    });

    // Make the whole row clickable
    if response.response.clicked() && action.is_none() {
        action = Some(NewTabAction::OpenDirectory { path: recent.path.clone(), resume });
    }

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tab_action_variants() {
        let path = PathBuf::from("/test");

        let _ = NewTabAction::OpenDirectory { path: path.clone(), resume: true };
        let _ = NewTabAction::OpenDirectory { path: path.clone(), resume: false };
        let _ = NewTabAction::BrowseDirectory;
        let _ = NewTabAction::AddBookmark(path.clone());
        let _ = NewTabAction::RemoveBookmark(path.clone());
        let _ = NewTabAction::RemoveRecent(path);
        let _ = NewTabAction::ClearRecent;
    }
}
