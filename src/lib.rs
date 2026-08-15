// Several pieces of the API (selection rendering, clipboard-type handling,
// background-image loading) are wired for upcoming features and not all
// call sites exist yet, so dead code is expected during development.
#![allow(dead_code)]

pub mod app;
pub mod clipboard;
pub mod color;
pub mod config;
pub mod event;
pub mod fonts;
pub mod input;
pub mod ipc;
pub mod kitty;
pub mod layout;
pub mod magic;
pub mod palette;
pub mod pty_io;
pub mod render;
pub mod scroll;
pub mod terminal;
pub mod themes;

#[cfg(test)]
mod repro;
