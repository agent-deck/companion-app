//! macOS native menu bar and context menu implementation
//!
//! Creates a native macOS menu bar following HIG guidelines with:
//! - App menu (About, Settings, Hide, Quit)
//! - File menu (New Session, Fresh Session, Load Recent, Close Tab)
//! - Edit menu (Copy, Paste, Select All)
//! - View menu (Font size controls, Fullscreen)
//! - Window menu (Minimize, Zoom)
//! - Help menu (Help, Report Issue)
//!
//! Also provides native context menus using NSMenu.

#![allow(deprecated)] // cocoa crate deprecation warnings

use cocoa::appkit::{NSApp, NSEventModifierFlags};
use cocoa::base::{id, nil, NO};
use cocoa::foundation::{NSAutoreleasePool, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

/// Menu action types that can be triggered from the menu bar
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    // App menu
    About,
    Settings,
    HideApp,
    HideOthers,
    ShowAll,
    HideWindow,  // Cmd+Q - hides window to tray (app keeps running)
    Quit,        // Opt+Cmd+Q - really quit the application

    // File menu
    NewSession,
    FreshSession,
    CloseTab,

    // Edit menu
    Copy,
    Paste,
    SelectAll,

    // View menu
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    ToggleFullscreen,

    // Window menu
    Minimize,
    Zoom,

    // Help menu
    Help,
    ReportIssue,

    // Recent session selected (index in the recent sessions list)
    LoadRecentSession(usize),
}

/// Global sender for menu actions
static MENU_TX: OnceLock<mpsc::UnboundedSender<MenuAction>> = OnceLock::new();

/// Counter for generating unique menu item tags
static MENU_TAG_COUNTER: AtomicUsize = AtomicUsize::new(1000);

/// Storage for edit menu items that need dynamic enable/disable
struct EditMenuItems {
    copy_item: id,
    paste_item: id,
}
unsafe impl Send for EditMenuItems {}
unsafe impl Sync for EditMenuItems {}

static EDIT_MENU_ITEMS: OnceLock<EditMenuItems> = OnceLock::new();

/// Initialize the menu action sender
pub fn init_menu_sender(tx: mpsc::UnboundedSender<MenuAction>) {
    let _ = MENU_TX.set(tx);
}

/// Send a menu action to the event loop
fn send_menu_action(action: MenuAction) {
    if let Some(tx) = MENU_TX.get() {
        if let Err(e) = tx.send(action) {
            error!("Failed to send menu action: {}", e);
        }
    }
}

/// Create the full macOS menu bar
#[allow(deprecated)]
pub fn create_menu_bar() {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);

        let app = NSApp();

        // Disable automatic window tabbing (removes "Show Tab Bar" from View menu)
        let _: () = msg_send![class!(NSWindow), setAllowsAutomaticWindowTabbing: NO];

        // Create main menu bar
        let main_menu: id = msg_send![class!(NSMenu), new];
        let _: () = msg_send![main_menu, setAutoenablesItems: NO];

        // Create app menu (Agent Deck)
        let app_menu_item: id = msg_send![class!(NSMenuItem), new];
        let app_menu = create_app_menu();
        let _: () = msg_send![app_menu_item, setSubmenu: app_menu];
        let _: () = msg_send![main_menu, addItem: app_menu_item];

        // Create File menu
        let file_menu_item = create_menu_item("File", None, None);
        let file_menu = create_file_menu();
        let _: () = msg_send![file_menu_item, setSubmenu: file_menu];
        let _: () = msg_send![main_menu, addItem: file_menu_item];

        // Create Edit menu
        let edit_menu_item = create_menu_item("Edit", None, None);
        let edit_menu = create_edit_menu();
        let _: () = msg_send![edit_menu_item, setSubmenu: edit_menu];
        let _: () = msg_send![main_menu, addItem: edit_menu_item];

        // Create View menu
        let view_menu_item = create_menu_item("View", None, None);
        let view_menu = create_view_menu();
        let _: () = msg_send![view_menu_item, setSubmenu: view_menu];
        let _: () = msg_send![main_menu, addItem: view_menu_item];

        // Create Window menu
        // Note: We don't call setWindowsMenu: to prevent macOS from auto-adding tab items
        let window_menu_item = create_menu_item("Window", None, None);
        let window_menu = create_window_menu();
        let _: () = msg_send![window_menu_item, setSubmenu: window_menu];
        let _: () = msg_send![main_menu, addItem: window_menu_item];

        // Create Help menu
        let help_menu_item = create_menu_item("Help", None, None);
        let help_menu = create_help_menu();
        let _: () = msg_send![help_menu_item, setSubmenu: help_menu];
        let _: () = msg_send![main_menu, addItem: help_menu_item];
        let _: () = msg_send![app, setHelpMenu: help_menu];

        // Set as app's main menu
        let _: () = msg_send![app, setMainMenu: main_menu];

        info!("macOS menu bar created");
    }
}

/// Create a menu item with optional keyboard shortcut
#[allow(deprecated)]
unsafe fn create_menu_item(title: &str, key: Option<&str>, modifiers: Option<NSEventModifierFlags>) -> id {
    let title_str = NSString::alloc(nil).init_str(title);
    let key_str = match key {
        Some(k) => NSString::alloc(nil).init_str(k),
        None => NSString::alloc(nil).init_str(""),
    };

    let item: id = msg_send![class!(NSMenuItem), alloc];
    let item: id = msg_send![item, initWithTitle:title_str action:nil keyEquivalent:key_str];

    if let Some(mods) = modifiers {
        let _: () = msg_send![item, setKeyEquivalentModifierMask: mods];
    }

    item
}

/// Create a menu item with action callback
#[allow(deprecated)]
unsafe fn create_action_menu_item(
    title: &str,
    key: Option<&str>,
    modifiers: Option<NSEventModifierFlags>,
    action: MenuAction,
) -> id {
    let item = create_menu_item(title, key, modifiers);

    // Set action selector
    let sel = sel!(menuItemClicked:);
    let _: () = msg_send![item, setAction: sel];

    // Store action as tag (we'll use a lookup table)
    let tag = register_menu_action(action);
    let _: () = msg_send![item, setTag: tag as isize];

    // Set target to our menu handler
    let handler = get_or_create_menu_handler();
    let _: () = msg_send![item, setTarget: handler];

    item
}

/// Create a separator menu item
#[allow(deprecated)]
unsafe fn create_separator() -> id {
    msg_send![class!(NSMenuItem), separatorItem]
}

/// Create the App menu (Agent Deck)
#[allow(deprecated)]
unsafe fn create_app_menu() -> id {
    let menu: id = msg_send![class!(NSMenu), new];
    let title = NSString::alloc(nil).init_str("Agent Deck");
    let _: () = msg_send![menu, setTitle: title];

    // About Agent Deck
    let about = create_action_menu_item("About Agent Deck", None, None, MenuAction::About);
    let _: () = msg_send![menu, addItem: about];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Settings... (Cmd+,)
    let settings = create_action_menu_item(
        "Settings...",
        Some(","),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::Settings,
    );
    let _: () = msg_send![menu, addItem: settings];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Hide Agent Deck (Cmd+H) - use standard action
    let hide_title = NSString::alloc(nil).init_str("Hide Agent Deck");
    let hide_key = NSString::alloc(nil).init_str("h");
    let hide: id = msg_send![class!(NSMenuItem), alloc];
    let hide: id = msg_send![hide, initWithTitle:hide_title action:sel!(hide:) keyEquivalent:hide_key];
    let _: () = msg_send![menu, addItem: hide];

    // Hide Others (Opt+Cmd+H) - use standard action
    let hide_others_title = NSString::alloc(nil).init_str("Hide Others");
    let hide_others_key = NSString::alloc(nil).init_str("h");
    let hide_others: id = msg_send![class!(NSMenuItem), alloc];
    let hide_others: id = msg_send![hide_others, initWithTitle:hide_others_title action:sel!(hideOtherApplications:) keyEquivalent:hide_others_key];
    let _: () = msg_send![hide_others, setKeyEquivalentModifierMask: NSEventModifierFlags::NSCommandKeyMask | NSEventModifierFlags::NSAlternateKeyMask];
    let _: () = msg_send![menu, addItem: hide_others];

    // Show All - use standard action
    let show_all_title = NSString::alloc(nil).init_str("Show All");
    let show_all_key = NSString::alloc(nil).init_str("");
    let show_all: id = msg_send![class!(NSMenuItem), alloc];
    let show_all: id = msg_send![show_all, initWithTitle:show_all_title action:sel!(unhideAllApplications:) keyEquivalent:show_all_key];
    let _: () = msg_send![menu, addItem: show_all];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Hide Agent Deck (Cmd+Q) - hides window to tray, app keeps running
    let hide_window = create_action_menu_item(
        "Hide Agent Deck",
        Some("q"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::HideWindow,
    );
    let _: () = msg_send![menu, addItem: hide_window];

    // Quit Agent Deck (Opt+Cmd+Q) - really quit the application
    let quit = create_action_menu_item(
        "Quit Agent Deck",
        Some("q"),
        Some(NSEventModifierFlags::NSCommandKeyMask | NSEventModifierFlags::NSAlternateKeyMask),
        MenuAction::Quit,
    );
    let _: () = msg_send![menu, addItem: quit];

    menu
}

/// Create the File menu
#[allow(deprecated)]
unsafe fn create_file_menu() -> id {
    let menu: id = msg_send![class!(NSMenu), new];
    let title = NSString::alloc(nil).init_str("File");
    let _: () = msg_send![menu, setTitle: title];

    // New Session (Cmd+N)
    let new_session = create_action_menu_item(
        "New Session",
        Some("n"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::NewSession,
    );
    let _: () = msg_send![menu, addItem: new_session];

    // Fresh Session Here (Shift+Cmd+N)
    let fresh_session = create_action_menu_item(
        "Fresh Session Here",
        Some("N"),
        Some(NSEventModifierFlags::NSCommandKeyMask | NSEventModifierFlags::NSShiftKeyMask),
        MenuAction::FreshSession,
    );
    let _: () = msg_send![menu, addItem: fresh_session];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Load Recent Session (submenu placeholder - will be dynamic)
    let recent_title = NSString::alloc(nil).init_str("Load Recent Session");
    let recent_key = NSString::alloc(nil).init_str("");
    let recent_item: id = msg_send![class!(NSMenuItem), alloc];
    let recent_item: id = msg_send![recent_item, initWithTitle:recent_title action:nil keyEquivalent:recent_key];

    // Create empty submenu (will be populated dynamically)
    let recent_menu: id = msg_send![class!(NSMenu), new];
    let recent_menu_title = NSString::alloc(nil).init_str("Load Recent Session");
    let _: () = msg_send![recent_menu, setTitle: recent_menu_title];

    // Add placeholder "No Recent Sessions"
    let no_recent_title = NSString::alloc(nil).init_str("No Recent Sessions");
    let no_recent_key = NSString::alloc(nil).init_str("");
    let no_recent: id = msg_send![class!(NSMenuItem), alloc];
    let no_recent: id = msg_send![no_recent, initWithTitle:no_recent_title action:nil keyEquivalent:no_recent_key];
    let _: () = msg_send![no_recent, setEnabled: NO];
    let _: () = msg_send![recent_menu, addItem: no_recent];

    let _: () = msg_send![recent_item, setSubmenu: recent_menu];
    let _: () = msg_send![menu, addItem: recent_item];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Close Tab (Cmd+W)
    let close_tab = create_action_menu_item(
        "Close Tab",
        Some("w"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::CloseTab,
    );
    let _: () = msg_send![menu, addItem: close_tab];

    menu
}

/// Create the Edit menu
#[allow(deprecated)]
unsafe fn create_edit_menu() -> id {
    let menu: id = msg_send![class!(NSMenu), new];
    let title = NSString::alloc(nil).init_str("Edit");
    let _: () = msg_send![menu, setTitle: title];

    // Copy (Cmd+C) - our custom handler
    let copy = create_action_menu_item(
        "Copy",
        Some("c"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::Copy,
    );
    let _: () = msg_send![copy, setEnabled: NO]; // Disabled by default until selection
    let _: () = msg_send![menu, addItem: copy];

    // Paste (Cmd+V) - our custom handler
    let paste = create_action_menu_item(
        "Paste",
        Some("v"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::Paste,
    );
    let _: () = msg_send![paste, setEnabled: NO]; // Disabled by default until clipboard has content
    let _: () = msg_send![menu, addItem: paste];

    // Store references for dynamic enable/disable
    let _ = EDIT_MENU_ITEMS.set(EditMenuItems {
        copy_item: copy,
        paste_item: paste,
    });

    // Select All (Cmd+A) - our custom handler (always enabled)
    let select_all = create_action_menu_item(
        "Select All",
        Some("a"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::SelectAll,
    );
    let _: () = msg_send![menu, addItem: select_all];

    menu
}

/// Create the View menu
#[allow(deprecated)]
unsafe fn create_view_menu() -> id {
    let menu: id = msg_send![class!(NSMenu), new];
    let title = NSString::alloc(nil).init_str("View");
    let _: () = msg_send![menu, setTitle: title];

    // Increase Font Size (Cmd+=, shown as Cmd++)
    // Using = key because + requires shift on US keyboards, but display as +
    let increase = create_action_menu_item(
        "Increase Font Size",
        Some("="),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::IncreaseFontSize,
    );
    let _: () = msg_send![menu, addItem: increase];

    // Decrease Font Size (Cmd+-)
    let decrease = create_action_menu_item(
        "Decrease Font Size",
        Some("-"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::DecreaseFontSize,
    );
    let _: () = msg_send![menu, addItem: decrease];

    // Reset Font Size (Cmd+0)
    let reset = create_action_menu_item(
        "Reset Font Size",
        Some("0"),
        Some(NSEventModifierFlags::NSCommandKeyMask),
        MenuAction::ResetFontSize,
    );
    let _: () = msg_send![menu, addItem: reset];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Toggle Fullscreen (Ctrl+Cmd+F) - use standard action
    let fullscreen_title = NSString::alloc(nil).init_str("Toggle Fullscreen");
    let fullscreen_key = NSString::alloc(nil).init_str("f");
    let fullscreen: id = msg_send![class!(NSMenuItem), alloc];
    let fullscreen: id = msg_send![fullscreen, initWithTitle:fullscreen_title action:sel!(toggleFullScreen:) keyEquivalent:fullscreen_key];
    let _: () = msg_send![fullscreen, setKeyEquivalentModifierMask: NSEventModifierFlags::NSCommandKeyMask | NSEventModifierFlags::NSControlKeyMask];
    let _: () = msg_send![menu, addItem: fullscreen];

    menu
}

/// Create the Window menu
#[allow(deprecated)]
unsafe fn create_window_menu() -> id {
    let menu: id = msg_send![class!(NSMenu), new];
    let title = NSString::alloc(nil).init_str("Window");
    let _: () = msg_send![menu, setTitle: title];

    // Minimize (Cmd+M) - standard action
    let minimize_title = NSString::alloc(nil).init_str("Minimize");
    let minimize_key = NSString::alloc(nil).init_str("m");
    let minimize: id = msg_send![class!(NSMenuItem), alloc];
    let minimize: id = msg_send![minimize, initWithTitle:minimize_title action:sel!(performMiniaturize:) keyEquivalent:minimize_key];
    let _: () = msg_send![menu, addItem: minimize];

    // Zoom - standard action
    let zoom_title = NSString::alloc(nil).init_str("Zoom");
    let zoom_key = NSString::alloc(nil).init_str("");
    let zoom: id = msg_send![class!(NSMenuItem), alloc];
    let zoom: id = msg_send![zoom, initWithTitle:zoom_title action:sel!(performZoom:) keyEquivalent:zoom_key];
    let _: () = msg_send![menu, addItem: zoom];

    menu
}

/// Create the Help menu
#[allow(deprecated)]
unsafe fn create_help_menu() -> id {
    let menu: id = msg_send![class!(NSMenu), new];
    let title = NSString::alloc(nil).init_str("Help");
    let _: () = msg_send![menu, setTitle: title];

    // Agent Deck Help
    let help = create_action_menu_item(
        "Agent Deck Help",
        None,
        None,
        MenuAction::Help,
    );
    let _: () = msg_send![menu, addItem: help];

    let _: () = msg_send![menu, addItem: create_separator()];

    // Report Issue...
    let report = create_action_menu_item(
        "Report Issue...",
        None,
        None,
        MenuAction::ReportIssue,
    );
    let _: () = msg_send![menu, addItem: report];

    menu
}

// Menu action registry (maps tags to actions)
use std::collections::HashMap;
use std::sync::Mutex;

static MENU_ACTIONS: OnceLock<Mutex<HashMap<usize, MenuAction>>> = OnceLock::new();

fn get_menu_actions() -> &'static Mutex<HashMap<usize, MenuAction>> {
    MENU_ACTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_menu_action(action: MenuAction) -> usize {
    let tag = MENU_TAG_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut actions = get_menu_actions().lock().unwrap();
    actions.insert(tag, action);
    tag
}

fn get_menu_action(tag: usize) -> Option<MenuAction> {
    let actions = get_menu_actions().lock().unwrap();
    actions.get(&tag).copied()
}

// Menu handler class - wrap raw pointer in a thread-safe wrapper
struct MenuHandlerPtr(*mut Object);
unsafe impl Send for MenuHandlerPtr {}
unsafe impl Sync for MenuHandlerPtr {}

static MENU_HANDLER: OnceLock<MenuHandlerPtr> = OnceLock::new();

fn get_or_create_menu_handler() -> id {
    use objc::runtime::Class;

    MENU_HANDLER.get_or_init(|| {
        unsafe {
            // Check if class already exists (from previous run or hot reload)
            let class_name = "AgentDeckMenuHandler";
            let existing_class = Class::get(class_name);

            let handler_class = if let Some(cls) = existing_class {
                cls
            } else {
                // Create handler class
                let superclass = class!(NSObject);
                let mut decl = ClassDecl::new(class_name, superclass)
                    .expect("Failed to create AgentDeckMenuHandler class");

                // Add method to handle menu item clicks
                extern "C" fn menu_item_clicked(_this: &Object, _cmd: Sel, sender: id) {
                    unsafe {
                        let tag: isize = msg_send![sender, tag];
                        debug!("Menu item clicked with tag: {}", tag);

                        if let Some(action) = get_menu_action(tag as usize) {
                            debug!("Found action: {:?}", action);
                            send_menu_action(action);
                        }
                    }
                }

                decl.add_method(
                    sel!(menuItemClicked:),
                    menu_item_clicked as extern "C" fn(&Object, Sel, id),
                );

                decl.register()
            };

            // Create instance
            let handler: id = msg_send![handler_class, new];
            MenuHandlerPtr(handler)
        }
    }).0
}

/// Update the recent sessions submenu
#[allow(deprecated)]
pub fn update_recent_sessions_menu(sessions: &[(String, String)]) {
    unsafe {
        let app = NSApp();
        let main_menu: id = msg_send![app, mainMenu];
        if main_menu == nil {
            return;
        }

        // Find File menu (index 1)
        let file_item: id = msg_send![main_menu, itemAtIndex: 1_isize];
        if file_item == nil {
            return;
        }

        let file_menu: id = msg_send![file_item, submenu];
        if file_menu == nil {
            return;
        }

        // Find "Load Recent Session" item (index 3, after New Session, Fresh Session, separator)
        let recent_item: id = msg_send![file_menu, itemAtIndex: 3_isize];
        if recent_item == nil {
            return;
        }

        let recent_menu: id = msg_send![recent_item, submenu];
        if recent_menu == nil {
            return;
        }

        // Clear existing items
        let _: () = msg_send![recent_menu, removeAllItems];

        if sessions.is_empty() {
            // Add placeholder
            let no_recent_title = NSString::alloc(nil).init_str("No Recent Sessions");
            let no_recent_key = NSString::alloc(nil).init_str("");
            let no_recent: id = msg_send![class!(NSMenuItem), alloc];
            let no_recent: id = msg_send![no_recent, initWithTitle:no_recent_title action:nil keyEquivalent:no_recent_key];
            let _: () = msg_send![no_recent, setEnabled: NO];
            let _: () = msg_send![recent_menu, addItem: no_recent];
        } else {
            // Add session items
            for (idx, (session_id, display_name)) in sessions.iter().enumerate() {
                let item = create_action_menu_item(
                    display_name,
                    None,
                    None,
                    MenuAction::LoadRecentSession(idx),
                );
                // Store session ID in represented object (for retrieval later)
                let session_str = NSString::alloc(nil).init_str(session_id);
                let _: () = msg_send![item, setRepresentedObject: session_str];
                let _: () = msg_send![recent_menu, addItem: item];
            }
        }
    }
}

/// Update the enabled state of edit menu items based on current selection and clipboard
#[allow(deprecated)]
pub fn update_edit_menu_state(has_selection: bool, clipboard_has_text: bool) {
    if let Some(items) = EDIT_MENU_ITEMS.get() {
        unsafe {
            let _: () = msg_send![items.copy_item, setEnabled: has_selection];
            let _: () = msg_send![items.paste_item, setEnabled: clipboard_has_text];
        }
    }
}

// =============================================================================
// Context Menu Implementation
// =============================================================================

/// Context menu action types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextMenuAction {
    NewSession,
    FreshSessionHere,
    LoadSession { session_id: String },
    Copy,
    Paste,
}

/// Session info for context menu display
#[derive(Debug, Clone)]
pub struct ContextMenuSession {
    pub session_id: String,
    pub title: String,
    pub time_ago: String,
}

// Shared state for context menu callback communication
use std::sync::atomic::AtomicI32;
use std::sync::Mutex as StdMutex;
static CONTEXT_MENU_SELECTED_TAG: AtomicI32 = AtomicI32::new(-1);
static CONTEXT_MENU_SESSION_IDS: OnceLock<StdMutex<Vec<String>>> = OnceLock::new();

/// Show a native context menu at the specified position
///
/// # Arguments
/// * `view` - The NSView pointer (from raw_window_handle)
/// * `x` - X coordinate in view coordinates
/// * `y` - Y coordinate in view coordinates
/// * `has_selection` - Whether text is selected (enables Copy)
/// * `has_clipboard` - Whether clipboard has text (enables Paste)
/// * `sessions` - List of recent sessions for the submenu
///
/// # Returns
/// The selected action, or None if menu was dismissed
#[allow(deprecated)]
pub fn show_context_menu(
    view: *mut std::ffi::c_void,
    x: f64,
    y: f64,
    has_selection: bool,
    has_clipboard: bool,
    sessions: &[ContextMenuSession],
) -> Option<ContextMenuAction> {
    unsafe {
        let _pool = NSAutoreleasePool::new(nil);
        let view: id = view as id;

        if view == nil {
            error!("Context menu: view is nil");
            return None;
        }

        // Reset selected action
        CONTEXT_MENU_SELECTED_TAG.store(-1, Ordering::SeqCst);

        // Store session IDs for lookup after menu closes
        let session_ids: Vec<String> = sessions.iter().map(|s| s.session_id.clone()).collect();
        let mutex = CONTEXT_MENU_SESSION_IDS.get_or_init(|| StdMutex::new(Vec::new()));
        *mutex.lock().unwrap() = session_ids;

        // Create the context menu
        let menu: id = msg_send![class!(NSMenu), new];
        let _: () = msg_send![menu, setAutoenablesItems: NO];

        // New Session
        let new_session_item = create_context_menu_item("New Session", 0);
        let _: () = msg_send![menu, addItem: new_session_item];

        // Separator
        let sep: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: sep];

        // Fresh Session Here
        let fresh_session_item = create_context_menu_item("Fresh Session Here", 1);
        let _: () = msg_send![menu, addItem: fresh_session_item];

        // Load Recent Session (submenu or disabled)
        if sessions.is_empty() {
            let title = NSString::alloc(nil).init_str("Load Recent Session");
            let key = NSString::alloc(nil).init_str("");
            let item: id = msg_send![class!(NSMenuItem), alloc];
            let item: id = msg_send![item, initWithTitle:title action:nil keyEquivalent:key];
            let _: () = msg_send![item, setEnabled: NO];
            let _: () = msg_send![menu, addItem: item];
        } else {
            // Create submenu with sessions
            let title = NSString::alloc(nil).init_str("Load Recent Session");
            let key = NSString::alloc(nil).init_str("");
            let item: id = msg_send![class!(NSMenuItem), alloc];
            let item: id = msg_send![item, initWithTitle:title action:nil keyEquivalent:key];

            let submenu: id = msg_send![class!(NSMenu), new];
            let _: () = msg_send![submenu, setAutoenablesItems: NO];

            for (idx, session) in sessions.iter().enumerate() {
                let session_item = create_context_menu_item(&session.title, 100 + idx as i32);
                let _: () = msg_send![submenu, addItem: session_item];
            }

            let _: () = msg_send![item, setSubmenu: submenu];
            let _: () = msg_send![menu, addItem: item];
        }

        // Separator
        let sep: id = msg_send![class!(NSMenuItem), separatorItem];
        let _: () = msg_send![menu, addItem: sep];

        // Copy
        let copy_item = create_context_menu_item("Copy", 2);
        if !has_selection {
            let _: () = msg_send![copy_item, setEnabled: NO];
        }
        let _: () = msg_send![menu, addItem: copy_item];

        // Paste
        let paste_item = create_context_menu_item("Paste", 3);
        if !has_clipboard {
            let _: () = msg_send![paste_item, setEnabled: NO];
        }
        let _: () = msg_send![menu, addItem: paste_item];

        // Get view bounds for coordinate conversion
        let bounds: cocoa::foundation::NSRect = msg_send![view, bounds];

        // Check if view uses flipped coordinates (origin at top-left)
        let is_flipped: bool = msg_send![view, isFlipped];

        // Convert coordinates based on view's coordinate system
        let location = if is_flipped {
            // View is already flipped (origin at top-left), use coordinates directly
            cocoa::foundation::NSPoint::new(x, y)
        } else {
            // View uses standard macOS coordinates (origin at bottom-left)
            // Convert from top-left origin (our coordinates) to bottom-left origin
            cocoa::foundation::NSPoint::new(x, bounds.size.height - y)
        };

        debug!("Context menu at ({}, {}), view bounds: {}x{}, flipped: {}",
               x, y, bounds.size.width, bounds.size.height, is_flipped);

        // popUpMenuPositioningItem:atLocation:inView: shows the menu
        // This call blocks until the menu is dismissed
        let _: () = msg_send![menu, popUpMenuPositioningItem:nil atLocation:location inView:view];

        // After menu closes, check what was selected
        let selected = CONTEXT_MENU_SELECTED_TAG.load(Ordering::SeqCst);

        match selected {
            0 => Some(ContextMenuAction::NewSession),
            1 => Some(ContextMenuAction::FreshSessionHere),
            2 => Some(ContextMenuAction::Copy),
            3 => Some(ContextMenuAction::Paste),
            n if n >= 100 => {
                // Session selected
                let idx = (n - 100) as usize;
                let session_ids = CONTEXT_MENU_SESSION_IDS.get()?.lock().ok()?;
                session_ids.get(idx).map(|id| ContextMenuAction::LoadSession {
                    session_id: id.clone()
                })
            }
            _ => None,
        }
    }
}

/// Create a context menu item with an action tag
#[allow(deprecated)]
unsafe fn create_context_menu_item(title: &str, tag: i32) -> id {
    let title_str = NSString::alloc(nil).init_str(title);
    let key_str = NSString::alloc(nil).init_str("");

    let item: id = msg_send![class!(NSMenuItem), alloc];
    let item: id = msg_send![item, initWithTitle:title_str action:sel!(contextMenuItemClicked:) keyEquivalent:key_str];

    // Set tag for identification
    let _: () = msg_send![item, setTag: tag as isize];

    // Set target to our handler
    let handler = get_or_create_context_menu_handler();
    let _: () = msg_send![item, setTarget: handler];

    item
}

// Context menu handler class
struct ContextMenuHandlerPtr(*mut Object);
unsafe impl Send for ContextMenuHandlerPtr {}
unsafe impl Sync for ContextMenuHandlerPtr {}

static CONTEXT_MENU_HANDLER: OnceLock<ContextMenuHandlerPtr> = OnceLock::new();

fn get_or_create_context_menu_handler() -> id {
    use objc::runtime::Class;

    CONTEXT_MENU_HANDLER.get_or_init(|| {
        unsafe {
            let class_name = "AgentDeckContextMenuHandler";
            let existing_class = Class::get(class_name);

            let handler_class = if let Some(cls) = existing_class {
                cls
            } else {
                let superclass = class!(NSObject);
                let mut decl = ClassDecl::new(class_name, superclass)
                    .expect("Failed to create AgentDeckContextMenuHandler class");

                extern "C" fn context_menu_item_clicked(_this: &Object, _cmd: Sel, sender: id) {
                    unsafe {
                        let tag: isize = msg_send![sender, tag];
                        debug!("Context menu item clicked with tag: {}", tag);
                        // Store in the module-level static
                        CONTEXT_MENU_SELECTED_TAG.store(tag as i32, Ordering::SeqCst);
                    }
                }

                decl.add_method(
                    sel!(contextMenuItemClicked:),
                    context_menu_item_clicked as extern "C" fn(&Object, Sel, id),
                );

                decl.register()
            };

            let handler: id = msg_send![handler_class, new];
            ContextMenuHandlerPtr(handler)
        }
    }).0
}
