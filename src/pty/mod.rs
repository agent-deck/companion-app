//! PTY module - Claude CLI wrapper

pub mod parser;
mod wrapper;

pub use parser::{AnsiParser, ParsedElement};
pub use wrapper::PtyWrapper;
