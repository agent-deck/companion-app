//! Capture the user's login shell environment for PTY processes.
//!
//! When launched from macOS Finder or a Linux desktop launcher, the process
//! inherits a bare-bones environment (PATH = /usr/bin:/bin:/usr/sbin:/sbin).
//! This module runs the user's login shell once at startup to capture their
//! full environment (PATH, LANG, etc.), similar to what iTerm2 and VS Code do.

use std::collections::HashMap;
use tracing::{info, warn};

/// Run the user's login shell to capture their full environment.
///
/// On Unix (macOS + Linux): detects shell from `$SHELL` (fallback `/bin/sh`),
/// runs `$SHELL -l -c '/usr/bin/env -0'`, parses NUL-delimited output.
///
/// On Windows: returns an empty map (environment is correct when launched
/// from Explorer).
///
/// Falls back to a minimal hardcoded PATH on any failure so we never regress
/// from current behavior.
pub fn resolve_login_env() -> HashMap<String, String> {
    #[cfg(unix)]
    {
        resolve_login_env_unix()
    }
    #[cfg(not(unix))]
    {
        HashMap::new()
    }
}

#[cfg(unix)]
fn resolve_login_env_unix() -> HashMap<String, String> {
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    info!("Resolving login environment from shell: {}", shell);

    let mut child = match Command::new(&shell)
        .args(["-l", "-c", "/usr/bin/env -0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to spawn login shell '{}': {}", shell, e);
            return fallback_env();
        }
    };

    // Wait with a 5-second timeout using a channel
    let timeout = Duration::from_secs(5);
    let (tx, rx) = mpsc::channel();

    // Take stdout before moving child to the thread
    let stdout = child.stdout.take();

    std::thread::spawn(move || {
        let result = child.wait();
        let _ = tx.send(result);
    });

    // Read stdout while waiting (the pipe has finite capacity, so we must
    // drain it to prevent the child from blocking)
    let stdout_data = if let Some(mut stdout) = stdout {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    } else {
        Vec::new()
    };

    match rx.recv_timeout(timeout) {
        Ok(Ok(status)) if status.success() => {
            let env = parse_env_null_delimited(&stdout_data);
            if env.is_empty() || !env.contains_key("PATH") {
                warn!("Login shell produced no usable environment, using fallback");
                return fallback_env();
            }
            info!(
                "Captured {} environment variables from login shell",
                env.len()
            );
            env
        }
        Ok(Ok(status)) => {
            warn!(
                "Login shell '{}' exited with status: {}",
                shell, status
            );
            fallback_env()
        }
        Ok(Err(e)) => {
            warn!("Login shell '{}' wait failed: {}", shell, e);
            fallback_env()
        }
        Err(_) => {
            warn!("Login shell '{}' timed out after {:?}", shell, timeout);
            fallback_env()
        }
    }
}

/// Parse NUL-delimited environment output (`env -0` format).
#[cfg(unix)]
fn parse_env_null_delimited(data: &[u8]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(data);
    let mut env = HashMap::new();

    for entry in text.split('\0') {
        if entry.is_empty() {
            continue;
        }
        if let Some((key, value)) = entry.split_once('=') {
            if !key.is_empty() {
                env.insert(key.to_string(), value.to_string());
            }
        }
    }

    env
}

/// Minimal fallback environment so we never regress from the previous
/// hardcoded PATH behavior.
#[cfg(unix)]
fn fallback_env() -> HashMap<String, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let current_path = std::env::var("PATH").unwrap_or_default();

    let mut paths = vec![
        "/opt/homebrew/bin".to_string(),
        "/opt/homebrew/sbin".to_string(),
        "/usr/local/bin".to_string(),
        "/usr/local/sbin".to_string(),
        format!("{}/.npm-global/bin", home),
        format!("{}/.claude/local", home),
        format!("{}/.cargo/bin", home),
        format!("{}/.local/bin", home),
    ];
    if !current_path.is_empty() {
        paths.push(current_path);
    }

    let mut env = HashMap::new();
    env.insert("PATH".to_string(), paths.join(":"));
    if !home.is_empty() {
        env.insert("HOME".to_string(), home);
    }
    env
}
