//! Unix-socket IPC so external tools can drive a running optix instance the
//! way `i3-msg` drives i3:
//!
//! ```text
//! optix-msg theme ayu     switch to the "ayu" theme
//! optix-msg themes        list the available themes
//! optix-msg ping          check that a terminal is running
//! optix-msg quit          quit the terminal
//! ```
//!
//! The terminal owns a listener socket and a background thread. Each accepted
//! connection reads one newline-terminated command, forwards it to the UI
//! thread, and blocks for the reply, which is written back to the client so
//! `optix-msg` can print it and exit. This sidesteps the in-terminal `/theme`
//! overlay entirely: there is no line-start heuristic, no shell cooperation,
//! and no timing window for the prompt to close under you.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A command forwarded from the IPC thread to the UI thread, with a channel
/// for the response text (which is then written back to the client).
pub struct IpcRequest {
    /// Raw command line, e.g. `theme ayu`.
    pub command: String,
    /// Send the response text here; the IPC thread relays it to the client.
    pub reply: mpsc::Sender<String>,
}

/// Commands understood over the socket, parsed by [`IpcCommand::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcCommand {
    /// `ping`: reachability check.
    Ping,
    /// `theme`, `themes`, `theme help`: list the available themes.
    Themes,
    /// `theme <name>`: switch to that theme.
    ThemeSet(String),
    /// `quit`: tell the terminal to exit.
    Quit,
    /// Anything unrecognized (or malformed); the original line is preserved
    /// so the error reply can echo it back.
    Unknown(String),
}

impl IpcCommand {
    /// Parse a single command line. Empty lines and anything with an
    /// unexpected shape fall through to [`IpcCommand::Unknown`].
    pub fn parse(line: &str) -> IpcCommand {
        let mut parts = line.trim().split_whitespace();
        let word = match parts.next() {
            Some(word) => word,
            None => return IpcCommand::Unknown(line.trim().to_owned()),
        };
        let lonely = |rest: &mut dyn Iterator<Item = &str>| rest.next().is_none();
        if word.eq_ignore_ascii_case("ping") && lonely(&mut parts) {
            return IpcCommand::Ping;
        }
        if word.eq_ignore_ascii_case("quit") && lonely(&mut parts) {
            return IpcCommand::Quit;
        }
        if word.eq_ignore_ascii_case("themes") && lonely(&mut parts) {
            return IpcCommand::Themes;
        }
        if word.eq_ignore_ascii_case("theme") {
            match parts.next() {
                None => return IpcCommand::Themes,
                Some(name) if name.eq_ignore_ascii_case("help") || name == "-h" || name == "--help" => {
                    return IpcCommand::Themes;
                },
                Some(name) if lonely(&mut parts) => return IpcCommand::ThemeSet(name.to_owned()),
                Some(_) => {},
            }
        }
        IpcCommand::Unknown(line.trim().to_owned())
    }
}

/// Path of the IPC socket, mirroring i3's `$XDG_RUNTIME_DIR/i3/ipc.sock`:
/// `$XDG_RUNTIME_DIR/optix/ipc.sock`, falling back to
/// `/tmp/optix-$UID/ipc.sock` when no runtime dir is set. Both the server and
/// `optix-msg` derive the path from this single function so they can never
/// disagree.
pub fn socket_path() -> PathBuf {
    let base = match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => std::env::temp_dir().join(format!(
            "optix-{}",
            std::env::var("UID").unwrap_or_else(|_| "user".into())
        )),
    };
    base.join("optix").join("ipc.sock")
}

/// Start the IPC server on a background thread. Commands are forwarded to the
/// UI thread over `tx`; `wakeup` pokes the winit loop so the reply is prompt
/// even while the loop idles in `ControlFlow::Wait`.
pub fn spawn(tx: mpsc::Sender<IpcRequest>, wakeup: winit::event_loop::EventLoopProxy<()>) {
    thread::spawn(move || {
        let path = socket_path();
        if let Some(dir) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(dir) {
                log::error!("ipc: cannot create {}: {err}", dir.display());
                return;
            }
        }
        // A stale socket from a previous (possibly crashed) instance would
        // make bind fail; unlink it first.
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(err) => {
                log::error!("ipc: failed to bind {}: {err}", path.display());
                return;
            },
        };
        log::info!("ipc: listening on {}", path.display());
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let tx = tx.clone();
                    let wakeup = wakeup.clone();
                    thread::spawn(move || handle_client(stream, tx, wakeup));
                },
                Err(err) => log::warn!("ipc: accept failed: {err}"),
            }
        }
    });
}

/// Serve one client: read a line, ask the UI thread to run it, write the
/// reply back (then drop the stream, which signals EOF to the client).
fn handle_client(
    stream: UnixStream,
    tx: mpsc::Sender<IpcRequest>,
    wakeup: winit::event_loop::EventLoopProxy<()>,
) {
    let mut reader = match stream.try_clone() {
        Ok(dup) => BufReader::new(dup),
        Err(err) => {
            log::warn!("ipc: dup failed: {err}");
            return;
        },
    };
    let mut line = String::new();
    if let Err(err) = reader.read_line(&mut line) {
        log::debug!("ipc: read failed: {err}");
        return;
    }
    let command = line.trim().to_string();
    if command.is_empty() {
        return;
    }
    let (reply_tx, reply_rx) = mpsc::channel::<String>();
    let request = IpcRequest { command, reply: reply_tx };
    if tx.send(request).is_err() {
        // The UI thread is gone (app shutting down); nothing to reply to.
        return;
    }
    let _ = wakeup.send_event(());
    let response = match reply_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(text) => text,
        Err(_) => "error: no response from optix (is it still running?)".to_string(),
    };
    let mut writer = stream;
    let _ = writer.write_all(response.as_bytes());
    let _ = writer.write_all(b"\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert_eq!(IpcCommand::parse("ping"), IpcCommand::Ping);
        assert_eq!(IpcCommand::parse("quit"), IpcCommand::Quit);
        assert_eq!(IpcCommand::parse("themes"), IpcCommand::Themes);
        assert_eq!(IpcCommand::parse("theme"), IpcCommand::Themes);
        assert_eq!(IpcCommand::parse("theme help"), IpcCommand::Themes);
        assert_eq!(IpcCommand::parse("theme -h"), IpcCommand::Themes);
        assert_eq!(IpcCommand::parse("theme ayu"), IpcCommand::ThemeSet("ayu".into()));
    }

    #[test]
    fn parse_is_case_insensitive_and_trims() {
        assert_eq!(IpcCommand::parse("  THEME  AYU  "), IpcCommand::ThemeSet("AYU".into()));
        assert_eq!(IpcCommand::parse("Ping"), IpcCommand::Ping);
        assert_eq!(IpcCommand::parse("  theme  "), IpcCommand::Themes);
    }

    #[test]
    fn parse_rejects_unknown_and_malformed() {
        assert!(matches!(IpcCommand::parse("ls"), IpcCommand::Unknown(_)));
        assert!(matches!(IpcCommand::parse("theme ayu extra"), IpcCommand::Unknown(_)));
        assert!(matches!(IpcCommand::parse("ping now"), IpcCommand::Unknown(_)));
        assert!(matches!(IpcCommand::parse(""), IpcCommand::Unknown(_)));
        assert!(matches!(IpcCommand::parse("   "), IpcCommand::Unknown(_)));
    }

    #[test]
    fn socket_path_is_deterministic_and_inside_runtime_dir() {
        let a = socket_path();
        let b = socket_path();
        assert_eq!(a, b);
        assert!(a.to_string_lossy().ends_with("optix/ipc.sock"));
        assert!(a.to_string_lossy().contains("optix"));
    }
}
