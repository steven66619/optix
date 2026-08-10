use std::sync::mpsc;
use std::time::Duration;

use winit::event_loop::EventLoop;

use optix::app::OptixApp;
use optix::config::Config;
use optix::event::PaneEvent;

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

    // Watch ~/.config/optix/config.toml and wake the event loop whenever it
    // changes so the running app can live-reload settings (colors, fonts, ...).
    spawn_config_watcher(el_wakeup.clone());

    let mut app = OptixApp::new(config, event_tx, event_rx, el_wakeup);

    if let Err(err) = event_loop.run_app(&mut app) {
        log::error!("event loop error: {err}");
    }

    // Remove the IPC socket left behind by a clean shutdown. A stale socket
    // from a crash is unlinked automatically at the next launch.
    let _ = std::fs::remove_file(optix::ipc::socket_path());
}

/// Poll the config file's mtime and wake the event loop when it changes.
fn spawn_config_watcher(wakeup: winit::event_loop::EventLoopProxy<()>) {
    std::thread::spawn(move || {
        let path = optix::config::config_path();
        let mut last = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            if now.is_some() && now != last {
                // Ignore a broken proxy: the loop shutting down just ends the poll.
                let _ = wakeup.send_event(());
                last = now;
            }
        }
    });
}
