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
mod layout;
mod palette;
mod render;
mod terminal;

#[cfg(test)]
mod repro;

use std::sync::mpsc;

use winit::event_loop::EventLoop;

use crate::app::OtermApp;
use crate::config::Config;
use crate::event::PaneEvent;

fn main() {
    env_logger::init();

    let config = Config::load();
    let (event_tx, event_rx) = mpsc::channel::<PaneEvent>();

    let mut app = OtermApp::new(config, event_tx, event_rx);

    let event_loop = match EventLoop::new() {
        Ok(loop_) => loop_,
        Err(err) => {
            log::error!("failed to create event loop: {err}");
            std::process::exit(1);
        },
    };
    if let Err(err) = event_loop.run_app(&mut app) {
        log::error!("event loop error: {err}");
    }
}
