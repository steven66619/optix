// Several pieces of the API (selection rendering, clipboard-type handling,
// background-image loading) are wired for upcoming features and not all
// call sites exist yet, so dead code is expected during development.
#![allow(dead_code)]

mod app;
mod color;
mod config;
mod event;
mod fonts;
mod input;
mod kitty;
mod layout;
mod palette;
mod pty_io;
mod render;
mod terminal;

#[cfg(test)]
mod repro;

use std::sync::mpsc;

use winit::event_loop::EventLoop;

use crate::app::OptixApp;
use crate::config::Config;
use crate::event::PaneEvent;

fn main() {
    env_logger::init();

    let config = Config::load();
    let (event_tx, event_rx) = mpsc::channel::<PaneEvent>();

    let event_loop = match EventLoop::new() {
        Ok(loop_) => loop_,
        Err(err) => {
            log::error!("failed to create event loop: {err}");
            std::process::exit(1);
        },
    };
    let el_wakeup = event_loop.create_proxy();

    let mut app = OptixApp::new(config, event_tx, event_rx, el_wakeup);

    if let Err(err) = event_loop.run_app(&mut app) {
        log::error!("event loop error: {err}");
    }
}
