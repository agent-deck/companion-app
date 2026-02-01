//! New tab page UI
//!
//! Displays a unified list of bookmarked and recent directories with session context,
//! in a single centered column layout. Features two-phase selection: first choose a
//! directory, then choose a session.

use crate::core::bookmarks::BookmarkManager;
use crate::core::claude_sessions::{get_sessions_for_directory, ClaudeSession};
use crate::core::settings::ColorScheme;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// SVG icons
const ARROW_LEFT_SVG: &[u8] = include_bytes!("../../assets/icons/arrow-left.svg");

/// Cached information about a directory's sessions
#[derive(Clone, Default)]
struct DirectoryInfo {
    session_count: usize,
    most_recent_title: Option<String>,
}

/// Cache for directory info to avoid repeated file reads
#[derive(Clone)]
struct DirectoryInfoCache {
    info: HashMap<PathBuf, DirectoryInfo>,
    last_refresh: Instant,
}

impl Default for DirectoryInfoCache {
    fn default() -> Self {
        Self {
            info: HashMap::new(),
            last_refresh: Instant::now(),
        }
    }
}

impl DirectoryInfoCache {
    /// Check if cache needs refresh (stale after 30 seconds)
    fn needs_refresh(&self) -> bool {
        self.last_refresh.elapsed().as_secs() > 30
    }

    /// Get cached info or load it
    fn get_info(&mut self, path: &PathBuf) -> DirectoryInfo {
        if self.needs_refresh() {
            self.info.clear();
            self.last_refresh = Instant::now();
        }

        if let Some(info) = self.info.get(path) {
            info.clone()
        } else {
            let sessions = get_sessions_for_directory(path);
            let info = DirectoryInfo {
                session_count: sessions.len(),
                most_recent_title: sessions.first().map(|s| s.display_title()),
            };
            self.info.insert(path.clone(), info.clone());
            info
        }
    }

    /// Preload info for multiple directories
    fn preload(&mut self, paths: &[PathBuf]) {
        if self.needs_refresh() {
            self.info.clear();
            self.last_refresh = Instant::now();
        }

        for path in paths {
            if !self.info.contains_key(path) {
                let sessions = get_sessions_for_directory(path);
                let info = DirectoryInfo {
                    session_count: sessions.len(),
                    most_recent_title: sessions.first().map(|s| s.display_title()),
                };
                self.info.insert(path.clone(), info);
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

/// Keyboard navigation state
#[derive(Debug, Clone, Default)]
struct KeyboardNavState {
    /// Currently selected index (None = no selection)
    selected_index: Option<usize>,
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
    /// Clear all recent entries (unused in new design, kept for compatibility)
    ClearRecent,
}

/// Shorten a path for display, using ~ for home directory
fn shorten_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    // Replace home directory with ~
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path_str.starts_with(home_str.as_ref()) {
            return format!("~{}", &path_str[home_str.len()..]);
        }
    }

    path_str.to_string()
}

/// A unified directory item that can be either bookmarked or recent
#[derive(Clone)]
struct DirectoryItem {
    path: PathBuf,
    name: String,
    is_bookmarked: bool,
    info: DirectoryInfo,
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
    let cache_id = egui::Id::new("new_tab_directory_cache");
    let nav_id = egui::Id::new(("new_tab_nav", session_id));

    let mut state: NewTabState = ui.data_mut(|d| d.get_temp(state_id).unwrap_or_default());
    let mut cache: DirectoryInfoCache = ui.data_mut(|d| d.get_temp(cache_id).unwrap_or_default());
    let mut nav: KeyboardNavState = ui.data_mut(|d| d.get_temp(nav_id).unwrap_or_default());

    // Preload directory info for visible directories
    let all_paths: Vec<PathBuf> = bookmark_manager
        .get_bookmarks()
        .iter()
        .map(|b| b.path.clone())
        .chain(bookmark_manager.get_recent().iter().map(|r| r.path.clone()))
        .collect();
    cache.preload(&all_paths);

    let action = match state.clone() {
        NewTabState::SelectDirectory => {
            render_directory_selection(ui, bookmark_manager, color_scheme, fg_color, &mut cache, &mut state, &mut nav)
        }
        NewTabState::SelectSession { path, sessions } => {
            render_session_selection(ui, color_scheme, fg_color, &path, &sessions, &mut state, &mut nav)
        }
    };

    // Save state back to memory
    ui.data_mut(|d| d.insert_temp(state_id, state));
    ui.data_mut(|d| d.insert_temp(cache_id, cache));
    ui.data_mut(|d| d.insert_temp(nav_id, nav));

    action
}

/// Render the directory selection phase with single centered column
fn render_directory_selection(
    ui: &mut egui::Ui,
    bookmark_manager: &BookmarkManager,
    color_scheme: ColorScheme,
    fg_color: egui::Color32,
    cache: &mut DirectoryInfoCache,
    state: &mut NewTabState,
    nav: &mut KeyboardNavState,
) -> Option<NewTabAction> {
    let mut action = None;

    // Build unified directory list first (needed for keyboard nav)
    let bookmarks = bookmark_manager.get_bookmarks();
    let recent = bookmark_manager.get_recent();

    // Track which paths are bookmarked for deduplication
    let bookmarked_paths: HashSet<&PathBuf> = bookmarks.iter().map(|b| &b.path).collect();

    // Collect all items in order
    let mut all_items: Vec<DirectoryItem> = bookmarks
        .iter()
        .map(|b| DirectoryItem {
            path: b.path.clone(),
            name: b.display_name(),
            is_bookmarked: true,
            info: cache.get_info(&b.path),
        })
        .collect();

    let recent_items: Vec<DirectoryItem> = recent
        .iter()
        .filter(|r| !bookmarked_paths.contains(&r.path))
        .map(|r| DirectoryItem {
            path: r.path.clone(),
            name: r.display_name(),
            is_bookmarked: false,
            info: cache.get_info(&r.path),
        })
        .collect();

    all_items.extend(recent_items);
    let item_count = all_items.len();

    // Handle keyboard navigation
    ui.input(|i| {
        if i.key_pressed(egui::Key::ArrowDown) {
            if item_count > 0 {
                nav.selected_index = Some(match nav.selected_index {
                    None => 0,
                    Some(idx) => (idx + 1).min(item_count - 1),
                });
            }
        }
        if i.key_pressed(egui::Key::ArrowUp) {
            if item_count > 0 {
                nav.selected_index = Some(match nav.selected_index {
                    None => 0,
                    Some(idx) => idx.saturating_sub(1),
                });
            }
        }
        if i.key_pressed(egui::Key::Enter) {
            if let Some(idx) = nav.selected_index {
                if let Some(item) = all_items.get(idx) {
                    // Trigger the same action as clicking
                    if let Some(a) = handle_directory_click(&item.path, state) {
                        action = Some(a);
                    }
                    // Reset selection when transitioning to session selection
                    nav.selected_index = Some(0);
                }
            }
        }
        if i.key_pressed(egui::Key::Escape) {
            nav.selected_index = None;
        }
    });

    // Center the content with max width
    let max_width = 550.0;
    let available_width = ui.available_width();
    let available_height = ui.available_height();
    let side_margin = ((available_width - max_width) / 2.0).max(20.0);

    // Use centered layout that preserves full height
    let content_rect = egui::Rect::from_min_size(
        ui.cursor().min + egui::vec2(side_margin, 0.0),
        egui::vec2(max_width, available_height),
    );

    // Split items back for rendering with separator
    let bookmark_count = bookmarks.len();

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
        ui.add_space(40.0);

        // Title
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("New Session")
                    .size(28.0)
                    .color(fg_color)
                    .strong(),
            );
        });

        ui.add_space(30.0);

        // Browse button (centered)
        ui.vertical_centered(|ui| {
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
        });

        ui.add_space(30.0);

        let has_bookmarks = bookmark_count > 0;
        let has_recent = item_count > bookmark_count;

        // Calculate available height for scroll area
        // Reserve space for SSH section below (separator + title + coming soon)
        let ssh_section_height = 100.0;
        let scroll_height = (ui.available_height() - ssh_section_height).max(150.0);

        // Directory list with scroll
        if !all_items.is_empty() {
            egui::ScrollArea::vertical()
                .id_salt("directory_list_scroll")
                .max_height(scroll_height)
                .show(ui, |ui| {
                    ui.set_min_width(max_width - 10.0);

                    // Render all items with index tracking
                    for (idx, item) in all_items.iter().enumerate() {
                        // Add separator between bookmarks and recent
                        if idx == bookmark_count && has_bookmarks && has_recent {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(8.0);
                        }

                        let is_selected = nav.selected_index == Some(idx);
                        if let Some(a) = render_directory_card(
                            ui,
                            item,
                            color_scheme,
                            fg_color,
                            state,
                            is_selected,
                        ) {
                            action = Some(a);
                            // Reset selection on click
                            nav.selected_index = Some(0);
                        }
                    }
                });
        } else {
            // Empty state
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("No directories yet")
                        .size(16.0)
                        .color(egui::Color32::GRAY)
                        .italics(),
                );
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Browse for a directory to get started")
                        .size(14.0)
                        .color(egui::Color32::DARK_GRAY),
                );
            });
        }

        ui.add_space(40.0);

        // SSH placeholder
        ui.separator();
        ui.add_space(20.0);

        ui.vertical_centered(|ui| {
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
    });

    action
}

/// Render a single directory card (two-line design)
fn render_directory_card(
    ui: &mut egui::Ui,
    item: &DirectoryItem,
    color_scheme: ColorScheme,
    fg_color: egui::Color32,
    state: &mut NewTabState,
    is_selected: bool,
) -> Option<NewTabAction> {
    let mut action: Option<NewTabAction> = None;

    // Card dimensions
    let card_height = 56.0;
    let card_padding_h = 12.0;
    let card_padding_v = 8.0;

    // Allocate space for the card
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), card_height),
        egui::Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        // Background - highlight if selected or hovered
        let bg = if is_selected || response.hovered() {
            color_scheme.active_tab_background()
        } else {
            color_scheme.inactive_tab_background()
        };
        ui.painter().rect_filled(rect, 4.0, bg);

        // Selection border
        if is_selected {
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(2.0, color_scheme.accent_color()),
            );
        }

        // Star icon position
        let star_x = rect.left() + card_padding_h;
        let star_y = rect.top() + card_padding_v + 8.0;

        // Star icon colors
        let star_color = if item.is_bookmarked {
            egui::Color32::from_rgb(255, 200, 50) // Gold
        } else {
            egui::Color32::GRAY
        };

        let star_rect = egui::Rect::from_min_size(
            egui::pos2(star_x, star_y - 7.0),
            egui::vec2(14.0, 14.0),
        );

        // Check if star was clicked
        let star_response = ui.interact(star_rect, ui.id().with(("star", &item.path)), egui::Sense::click());

        let star_tint = if star_response.hovered() {
            egui::Color32::WHITE
        } else {
            star_color
        };

        // Draw star using Unicode character to avoid layout issues
        let star_char = if item.is_bookmarked { "★" } else { "☆" };
        ui.painter().text(
            star_rect.center(),
            egui::Align2::CENTER_CENTER,
            star_char,
            egui::FontId::proportional(14.0),
            star_tint,
        );

        if star_response.clicked() {
            if item.is_bookmarked {
                action = Some(NewTabAction::RemoveBookmark(item.path.clone()));
            } else {
                action = Some(NewTabAction::AddBookmark(item.path.clone()));
            }
        }

        // Content area starts after star
        let content_x = star_x + 24.0;

        // Line 1: Name (bold) + shortened path (gray, right-aligned)
        let line1_y = rect.top() + card_padding_v + 8.0;

        // Directory name (bold)
        ui.painter().text(
            egui::pos2(content_x, line1_y),
            egui::Align2::LEFT_CENTER,
            &item.name,
            egui::FontId::new(14.0, egui::FontFamily::Proportional),
            fg_color,
        );

        // Shortened path (right-aligned, gray)
        let short_path = shorten_path(&item.path);
        ui.painter().text(
            egui::pos2(rect.right() - card_padding_h, line1_y),
            egui::Align2::RIGHT_CENTER,
            &short_path,
            egui::FontId::proportional(12.0),
            egui::Color32::GRAY,
        );

        // Line 2: Session title + session count
        let line2_y = rect.top() + card_padding_v + 30.0;

        // Session title or dash
        let session_title = item
            .info
            .most_recent_title
            .as_ref()
            .map(|t| {
                // Truncate long titles
                if t.len() > 40 {
                    format!("{}...", &t[..37])
                } else {
                    t.clone()
                }
            })
            .unwrap_or_else(|| "\u{2014}".to_string()); // em dash

        let title_color = if item.info.most_recent_title.is_some() {
            egui::Color32::from_gray(180)
        } else {
            egui::Color32::DARK_GRAY
        };

        ui.painter().text(
            egui::pos2(content_x, line2_y),
            egui::Align2::LEFT_CENTER,
            &session_title,
            egui::FontId::proportional(12.0),
            title_color,
        );

        // Session count (right-aligned)
        let session_text = if item.info.session_count > 0 {
            format!(
                "{} session{}",
                item.info.session_count,
                if item.info.session_count == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };

        // Only show session count if there are sessions
        if !session_text.is_empty() {
            ui.painter().text(
                egui::pos2(rect.right() - card_padding_h, line2_y),
                egui::Align2::RIGHT_CENTER,
                &session_text,
                egui::FontId::proportional(12.0),
                color_scheme.accent_color(),
            );
        }

        // X button for recent (non-bookmarked) items
        if !item.is_bookmarked {
            let x_button_x = rect.right() - card_padding_h - 4.0;
            let x_button_y = rect.top() + card_padding_v + 8.0;
            let x_rect = egui::Rect::from_center_size(
                egui::pos2(x_button_x, x_button_y),
                egui::vec2(16.0, 16.0),
            );

            let x_response = ui.interact(x_rect, ui.id().with(("remove", &item.path)), egui::Sense::click());

            if x_response.hovered() {
                ui.painter().text(
                    egui::pos2(x_button_x, x_button_y),
                    egui::Align2::CENTER_CENTER,
                    "\u{00D7}", // ×
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );
            }

            if x_response.clicked() {
                action = Some(NewTabAction::RemoveRecent(item.path.clone()));
            }
        }
    }

    // Tooltip with full path
    response.clone().on_hover_text(item.path.display().to_string());

    // Click on card (but not star or X) opens directory
    if response.clicked() && action.is_none() {
        action = handle_directory_click(&item.path, state);
    }

    ui.add_space(2.0); // Spacing between cards

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
    nav: &mut KeyboardNavState,
) -> Option<NewTabAction> {
    let mut action = None;

    // Total selectable items: "Start New" button + all sessions
    let item_count = 1 + sessions.len();

    // Handle keyboard navigation
    ui.input(|i| {
        if i.key_pressed(egui::Key::ArrowDown) {
            nav.selected_index = Some(match nav.selected_index {
                None => 0,
                Some(idx) => (idx + 1).min(item_count - 1),
            });
        }
        if i.key_pressed(egui::Key::ArrowUp) {
            nav.selected_index = Some(match nav.selected_index {
                None => 0,
                Some(idx) => idx.saturating_sub(1),
            });
        }
        if i.key_pressed(egui::Key::Enter) {
            if let Some(idx) = nav.selected_index {
                if idx == 0 {
                    // "Start New Session" selected
                    action = Some(NewTabAction::OpenDirectory {
                        path: path.clone(),
                        resume_session: Some(String::new()),
                    });
                } else if let Some(session) = sessions.get(idx - 1) {
                    // Session selected
                    let is_most_recent = idx == 1;
                    let resume_session = if is_most_recent {
                        None // Use --continue
                    } else {
                        Some(session.session_id.clone())
                    };
                    action = Some(NewTabAction::OpenDirectory {
                        path: path.clone(),
                        resume_session,
                    });
                }
            }
        }
        if i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Backspace) {
            // Go back to directory selection
            *state = NewTabState::SelectDirectory;
            nav.selected_index = None;
        }
    });

    // Center the content with max width
    let max_width = 550.0;
    let available_width = ui.available_width();
    let available_height = ui.available_height();
    let side_margin = ((available_width - max_width) / 2.0).max(20.0);

    // Use centered layout that preserves full height
    let content_rect = egui::Rect::from_min_size(
        ui.cursor().min + egui::vec2(side_margin, 0.0),
        egui::vec2(max_width, available_height),
    );

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
        ui.add_space(20.0);

            // Header with back button and path
            ui.horizontal(|ui| {
                let back_response = ui.add(
                    egui::Image::from_bytes("bytes://arrow-left.svg", ARROW_LEFT_SVG)
                        .fit_to_exact_size(egui::vec2(18.0, 18.0))
                        .tint(fg_color)
                        .sense(egui::Sense::click()),
                );
                if back_response.on_hover_text("Back to directory selection").clicked() {
                    *state = NewTabState::SelectDirectory;
                }

                ui.add_space(8.0);

                ui.label(
                    egui::RichText::new(shorten_path(path))
                        .size(16.0)
                        .color(fg_color),
                );
            });

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(20.0);

            // "Start New Session" button (index 0)
            let new_session_selected = nav.selected_index == Some(0);
            let (new_rect, new_response) = ui.allocate_exact_size(
                egui::vec2(max_width - 10.0, 44.0),
                egui::Sense::click(),
            );

            if ui.is_rect_visible(new_rect) {
                let bg = if new_session_selected || new_response.hovered() {
                    color_scheme.active_tab_background()
                } else {
                    color_scheme.inactive_tab_background()
                };
                ui.painter().rect_filled(new_rect, 6.0, bg);

                // Selection border
                if new_session_selected {
                    ui.painter().rect_stroke(
                        new_rect,
                        6.0,
                        egui::Stroke::new(2.0, color_scheme.accent_color()),
                    );
                }

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

                // Session list with scroll - use available height
                let scroll_height = ui.available_height().max(200.0);
                egui::ScrollArea::vertical()
                    .id_salt("session_list_scroll")
                    .max_height(scroll_height)
                    .show(ui, |ui| {
                        ui.set_min_width(max_width - 10.0);

                        for (idx, session) in sessions.iter().enumerate() {
                            let is_most_recent = idx == 0;
                            // Session items start at index 1 (after "Start New")
                            let is_selected = nav.selected_index == Some(idx + 1);

                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(max_width - 10.0, 56.0),
                                egui::Sense::click(),
                            );

                            if ui.is_rect_visible(rect) {
                                let bg = if is_selected || response.hovered() {
                                    color_scheme.active_tab_background()
                                } else {
                                    color_scheme.inactive_tab_background()
                                };
                                ui.painter().rect_filled(rect, 6.0, bg);

                                // Selection border
                                if is_selected {
                                    ui.painter().rect_stroke(
                                        rect,
                                        6.0,
                                        egui::Stroke::new(2.0, color_scheme.accent_color()),
                                    );
                                }

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

    #[test]
    fn test_shorten_path() {
        // Test with a path that doesn't start with home
        let path = Path::new("/var/log/test");
        assert_eq!(shorten_path(path), "/var/log/test");
    }
}
