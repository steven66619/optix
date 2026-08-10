//! `optix-msg` — drive a running optix terminal over its Unix socket, the
//! way `i3-msg` drives i3.
//!
//! Usage:
//!
//! ```text
//! optix-msg theme ayu      switch to the "ayu" theme
//! optix-msg themes         list the available themes
//! optix-msg ping           check that a terminal is running (replies "pong")
//! optix-msg quit           quit the terminal
//! ```
//!
//! The exit code is 0 on success, 1 when the command fails (unknown command,
//! no terminal running), and 2 for a usage error.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: optix-msg <command> [args...]");
        eprintln!();
        eprintln!("commands:");
        eprintln!("  theme <name>   switch to a theme (e.g. `optix-msg theme ayu`)");
        eprintln!("  themes         list the available themes");
        eprintln!("  ping           check that optix is running (replies `pong`)");
        eprintln!("  quit           quit the running terminal");
        return ExitCode::from(2);
    }

    let command = args.join(" ");
    let path = optix::ipc::socket_path();

    let mut stream = match UnixStream::connect(&path) {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("optix-msg: cannot connect to {}: {err}", path.display());
            eprintln!("optix-msg: is the terminal running?");
            return ExitCode::from(1);
        },
    };

    if let Err(err) = write!(stream, "{command}\n") {
        eprintln!("optix-msg: write failed: {err}");
        return ExitCode::from(1);
    }

    // The server closes the connection after writing its reply, which makes
    // this read hit EOF and terminate.
    let mut response = String::new();
    if let Err(err) = stream.read_to_string(&mut response) {
        eprintln!("optix-msg: read failed: {err}");
        return ExitCode::from(1);
    }

    print!("{response}");
    if response.trim_start().starts_with("error:") {
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
