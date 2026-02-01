//! New tab page UI
//!
//! Displays bookmarks, recent directories, and directory picker for new tabs.
//! Now with two-phase selection: first choose a directory, then choose a session.

use crate::core::bookmarks::{Bookmark, BookmarkManager, RecentEntry};
use crate::core::claude_sessions::{get_session_count, get_sessions_for_directory, ClaudeSession};
use crate::core::settings::ColorScheme;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Cache for session counts to avoid repeated file reads
#[derive(Clone)]
struct SessionCountCache {
    counts: HashMap<PathBuf, usize>,
    last_refresh: Instant,
}

impl Default for SessionCountCache {
    fn default() -> Self {
        Self {
            counts: HashMap::new(),
            last_refresh: Instant::now(),
        }
    }
}

impl SessionCountCache {
    /// Check if cache needs refresh (stale after 30 seconds)
    fn needs_refresh(&self) -> bool {
        self.last_refresh.elapsed().as_secs() > 30
    }

    /// Get cached count or load it
    fn get_count(&mut self, path: &PathBuf) -> usize {
        if self.needs_refresh() {
            self.counts.clear();
            self.last_refresh = Instant::now();
        }

        if let Some(&count) = self.counts.get(path) {
            count
        } else {
            let count = get_session_count(path);
            self.counts.insert(path.clone(), count);
            count
        }
    }

    /// Preload counts for multiple directories
    fn preload(&mut self, paths: &[PathBuf]) {
        if self.needs_refresh() {
            self.counts.clear();
            self.last_refresh = Instant::now();
        }

        for path in paths {
            if !self.counts.contains_key(path) {
                let count = get_session_count(path);
                self.counts.insert(path.clone(), count);
            }
        }
    }
}

/// State machine for the new tab UI
#[derive(Debug, Clone)]
pub enum NewTabState {
    /// User is selecting a directory
    SelectDirectory,
    /// User is selecting a session for the chosen directory
    SelectSession {
        path: PathBuf,
        sessions: Vec<ClaudeSession>,
    },
}

impl Default for NewTabState {
    fn default() -> Self {
        Self::SelectDirectory
    }
}

/// Actions that can be triggered from the new tab page
#[derive(Debug, Clone)]
pub enum NewTabAction {
    /// Open a directory with optional session resume
    /// None = fresh start, Some(id) = --resume {session-id}
    OpenDirectory {
        path: PathBuf,
        resume_session: Option<String>,
    },
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
/// The session_id is used to maintain separate state per new-tab instance
pub fn render_new_tab_page(
    ui: &mut egui::Ui,
    bookmark_manager: &BookmarkManager,
    color_scheme: ColorScheme,
    session_id: usize,
) -> Option<NewTabAction> {
    let fg_color = color_scheme.foreground();

    // Get/set state from egui memory - keyed by session_id for per-tab state
    let state_id = egui::Id::new(("new_tab_state", session_id));
    let cache_id = egui::Id::new("new_tab_session_cache");

    let mut state: NewTabState = ui.data_mut(|d| d.get_temp(state_id).unwrap_or_default());
    let mut cache: SessionCountCache = ui.data_mut(|d| d.get_temp(cache_id).unwrap_or_default());

    // Preload session counts for visible directories
    let all_paths: Vec<PathBuf> = bookmark_manager
        .get_bookmarks()
        .iter()
        .map(|b| b.path.clone())
        .chain(bookmark_manager.get_recent().iter().map(|r| r.path.clone()))
        .collect();
    cache.preload(&all_paths);

    let action = match state.clone() {
        NewTabState::SelectDirectory => {
            render_directory_selection(ui, bookmark_manager, color_scheme, fg_color, &mut cache, &mut state)
        }
        NewTabState::SelectSession {
            path,
            sessions,
        } => {
            render_session_selection(ui, color_scheme, fg_color, &path, &sessions, &mut state)
        }
    };

    // Save state back to memory
    ui.data_mut(|d| d.insert_temp(state_id, state));
    ui.data_mut(|d| d.insert_temp(cache_id, cache));

    action
}

/// Render the directory selection phase
fn render_directory_selection(
    ui: &mut egui::Ui,
    bookmark_manager: &BookmarkManager,
    color_scheme: ColorScheme,
    fg_color: egui::Color32,
    cache: &mut SessionCountCache,
    state: &mut NewTabState,
) -> Option<NewTabAction> {
    let mut action = None;

    // Center the content
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);

        // Title
        ui.label(
            egui::RichText::new("New Session")
                .size(28.0)
                .color(fg_color)
                .strong(),
        );

        ui.add_space(30.0);

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
                        .id_salt("bookmarks_scroll")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for bookmark in bookmark_manager.get_bookmarks() {
                                let session_count = cache.get_count(&bookmark.path);
                                if let Some(a) = render_bookmark_item(
                                    ui,
                                    bookmark,
                                    session_count,
                                    color_scheme,
                                    state,
                                ) {
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
                        .id_salt("recent_scroll")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for recent in bookmark_manager.get_recent() {
                                let is_bookmarked = bookmark_manager.is_bookmarked(&recent.path);
                                let session_count = cache.get_count(&recent.path);
                                if let Some(a) = render_recent_item(
                                    ui,
                                    recent,
                                    is_bookmarked,
                                    session_count,
                                    color_scheme,
                                    state,
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

    action
}

/// Render the session selection phase
fn render_session_selection(
    ui: &mut egui::Ui,
    color_scheme: ColorScheme,
    fg_color: egui::Color32,
    path: &PathBuf,
    sessions: &[ClaudeSession],
    state: &mut NewTabState,
) -> Option<NewTabAction> {
    let mut action = None;

    // Fixed width for all session items
    let item_width = 500.0;

    ui.vertical(|ui| {
        ui.add_space(20.0);

        // Header with back button and path
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("<").size(18.0).color(fg_color),
                    )
                    .frame(false),
                )
                .on_hover_text("Back to directory selection")
                .clicked()
            {
                *state = NewTabState::SelectDirectory;
            }

            ui.add_space(8.0);

            ui.label(
                egui::RichText::new(path.display().to_string())
                    .size(16.0)
                    .color(fg_color),
            );
        });

        ui.add_space(20.0);
        ui.separator();
        ui.add_space(20.0);

        // Center the content
        ui.vertical_centered(|ui| {
            // "Start New Session" button - click opens directly
            let (new_rect, new_response) = ui.allocate_exact_size(
                egui::vec2(item_width, 44.0),
                egui::Sense::click(),
            );

            if ui.is_rect_visible(new_rect) {
                let bg = if new_response.hovered() {
                    color_scheme.active_tab_background()
                } else {
                    color_scheme.inactive_tab_background()
                };
                ui.painter().rect_filled(new_rect, 6.0, bg);

                let text_pos = new_rect.left_center() + egui::vec2(16.0, 0.0);
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    "+",
                    egui::FontId::proportional(18.0),
                    egui::Color32::from_rgb(100, 200, 100),
                );
                ui.painter().text(
                    text_pos + egui::vec2(24.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    "Start New Session",
                    egui::FontId::proportional(15.0),
                    fg_color,
                );
            }

            if new_response.clicked() {
                // Empty string = explicit fresh start (no --continue flag)
                action = Some(NewTabAction::OpenDirectory {
                    path: path.clone(),
                    resume_session: Some(String::new()),
                });
            }

            ui.add_space(20.0);

            if !sessions.is_empty() {
                ui.label(
                    egui::RichText::new("Recent Sessions")
                        .size(14.0)
                        .color(egui::Color32::GRAY),
                );
                ui.add_space(10.0);

                // Session list with scroll - click opens directly
                egui::ScrollArea::vertical()
                    .id_salt("session_list_scroll")
                    .max_height(400.0)
                    .show(ui, |ui| {
                        ui.set_min_width(item_width);

                        for (idx, session) in sessions.iter().enumerate() {
                            let is_most_recent = idx == 0;

                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(item_width, 56.0),
                                egui::Sense::click(),
                            );

                            if ui.is_rect_visible(rect) {
                                let bg = if response.hovered() {
                                    color_scheme.active_tab_background()
                                } else {
                                    color_scheme.inactive_tab_background()
                                };
                                ui.painter().rect_filled(rect, 6.0, bg);

                                let title_x = rect.left() + 16.0;
                                let title_y = rect.top() + 18.0;

                                // Session title
                                ui.painter().text(
                                    egui::pos2(title_x, title_y),
                                    egui::Align2::LEFT_CENTER,
                                    session.display_title(),
                                    egui::FontId::proportional(14.0),
                                    fg_color,
                                );

                                // "(last session)" label for most recent
                                if is_most_recent {
                                    // Measure title width to position label after it
                                    let title_galley = ui.painter().layout_no_wrap(
                                        session.display_title().to_string(),
                                        egui::FontId::proportional(14.0),
                                        fg_color,
                                    );
                                    ui.painter().text(
                                        egui::pos2(title_x + title_galley.size().x + 8.0, title_y),
                                        egui::Align2::LEFT_CENTER,
                                        "(last session)",
                                        egui::FontId::proportional(12.0),
                                        egui::Color32::from_rgb(100, 149, 237),
                                    );
                                }

                                // Metadata line
                                ui.painter().text(
                                    egui::pos2(title_x, rect.top() + 40.0),
                                    egui::Align2::LEFT_CENTER,
                                    format!(
                                        "{} messages  {}",
                                        session.message_count,
                                        session.relative_modified_time()
                                    ),
                                    egui::FontId::proportional(12.0),
                                    egui::Color32::GRAY,
                                );
                            }

                            // Click opens directly
                            if response.clicked() {
                                // For most recent session, use --continue (None) which is more reliable
                                // For other sessions, use --resume with specific ID
                                let resume_session = if is_most_recent {
                                    None // Will use --continue
                                } else {
                                    Some(session.session_id.clone())
                                };
                                action = Some(NewTabAction::OpenDirectory {
                                    path: path.clone(),
                                    resume_session,
                                });
                            }

                            ui.add_space(4.0);
                        }
                    });
            }
        });
    });

    action
}

/// Handle directory selection - transitions to session picker or opens directly
fn handle_directory_click(path: &PathBuf, state: &mut NewTabState) -> Option<NewTabAction> {
    let sessions = get_sessions_for_directory(path);

    match sessions.len() {
        0 => {
            // No sessions - start fresh directly (empty string = no flags)
            Some(NewTabAction::OpenDirectory {
                path: path.clone(),
                resume_session: Some(String::new()),
            })
        }
        1 => {
            // One session - open directly with that session
            Some(NewTabAction::OpenDirectory {
                path: path.clone(),
                resume_session: Some(sessions[0].session_id.clone()),
            })
        }
        _ => {
            // Multiple sessions - show session picker
            *state = NewTabState::SelectSession {
                path: path.clone(),
                sessions,
            };
            None
        }
    }
}

/// Render a bookmark item and return any action triggered
fn render_bookmark_item(
    ui: &mut egui::Ui,
    bookmark: &Bookmark,
    session_count: usize,
    color_scheme: ColorScheme,
    state: &mut NewTabState,
) -> Option<NewTabAction> {
    let mut action = None;

    let response = ui.horizontal(|ui| {
        // Star icon (filled for bookmarks)
        if ui
            .add(
                egui::Button::new(egui::RichText::new("*").size(14.0).color(egui::Color32::GOLD))
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
            action = handle_directory_click(&bookmark.path, state);
        }

        // Session count badge
        if session_count > 0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("[{}]", session_count))
                        .size(12.0)
                        .color(egui::Color32::GRAY),
                );
            });
        }
    });

    // Make the whole row clickable
    if response.response.clicked() && action.is_none() {
        action = handle_directory_click(&bookmark.path, state);
    }

    action
}

/// Render a recent item and return any action triggered
fn render_recent_item(
    ui: &mut egui::Ui,
    recent: &RecentEntry,
    is_bookmarked: bool,
    session_count: usize,
    color_scheme: ColorScheme,
    state: &mut NewTabState,
) -> Option<NewTabAction> {
    let mut action = None;

    let response = ui.horizontal(|ui| {
        // Star icon (toggle bookmark)
        let star = if is_bookmarked { "*" } else { "o" };
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
            action = handle_directory_click(&recent.path, state);
        }

        // Session count badge and close button
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Close button
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("x").size(14.0).color(egui::Color32::GRAY),
                    )
                    .frame(false),
                )
                .on_hover_text("Remove from recent")
                .clicked()
            {
                action = Some(NewTabAction::RemoveRecent(recent.path.clone()));
            }

            // Session count badge
            if session_count > 0 {
                ui.label(
                    egui::RichText::new(format!("[{}]", session_count))
                        .size(12.0)
                        .color(egui::Color32::GRAY),
                );
            } else {
                ui.label(
                    egui::RichText::new("-")
                        .size(12.0)
                        .color(egui::Color32::DARK_GRAY),
                );
            }
        });
    });

    // Make the whole row clickable
    if response.response.clicked() && action.is_none() {
        action = handle_directory_click(&recent.path, state);
    }

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tab_action_variants() {
        let path = PathBuf::from("/test");

        let _ = NewTabAction::OpenDirectory {
            path: path.clone(),
            resume_session: None,
        };
        let _ = NewTabAction::OpenDirectory {
            path: path.clone(),
            resume_session: Some("session-123".to_string()),
        };
        let _ = NewTabAction::BrowseDirectory;
        let _ = NewTabAction::AddBookmark(path.clone());
        let _ = NewTabAction::RemoveBookmark(path.clone());
        let _ = NewTabAction::RemoveRecent(path);
        let _ = NewTabAction::ClearRecent;
    }

    #[test]
    fn test_new_tab_state_default() {
        let state = NewTabState::default();
        assert!(matches!(state, NewTabState::SelectDirectory));
    }
}
