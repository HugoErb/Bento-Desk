//! Bento Desk: desktop fences for Windows 10 and 11.
//!
//! The resident process never loads code into another process: no shell
//! extension, no DLL, no hook other than the low-level mouse hook while the
//! desktop has the focus. See `docs/architecture.md`.

#![windows_subsystem = "windows"]
#![forbid(unsafe_code)]

mod app;
mod fences;
mod logging;
mod menus;
mod peek;
mod popup;
mod portals;
mod sync;
mod worker;

use bento_core::ipc::{COPYDATA_KIND, DAEMON_CLASS, IpcCmd};
use bento_win::com::ComApartment;
use bento_win::system::{SingleInstance, send_copydata};
use bento_win::window;

const MUTEX: &str = "Local\\BentoDesk.SingleInstance";

fn main() {
    if let Err(e) = run() {
        tracing::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}

/// `bento-desk.exe --command <json>` sends an [`IpcCmd`] to the running
/// instance and exits (used by scripts and the installer).
fn send_command(json: &str) -> anyhow::Result<()> {
    let cmd: IpcCmd = serde_json::from_str(json)?;
    let w = window::find_window(DAEMON_CLASS).ok_or_else(|| anyhow::anyhow!("not running"))?;
    if !send_copydata(w, COPYDATA_KIND, &cmd.to_bytes()) {
        anyhow::bail!("the running instance did not answer");
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--command")
        && let Some(json) = args.get(i + 1)
    {
        return send_command(json);
    }
    let paths = bento_config::Paths::resolve()?;
    paths.ensure_dirs()?;
    let config = bento_config::load_config(&paths);
    let _log = logging::init(&paths.log_dir, config.general.log_level);

    let Some(_instance) = SingleInstance::acquire(MUTEX)? else {
        // Second launch: ask the running instance to open the settings.
        if let Some(w) = window::find_window(DAEMON_CLASS) {
            send_copydata(w, COPYDATA_KIND, &IpcCmd::OpenSettings.to_bytes());
        }
        return Ok(());
    };
    tracing::info!("Bento Desk {} starting", env!("CARGO_PKG_VERSION"));

    let _com = ComApartment::ole_sta()?;
    let app = app::App::new(paths)?;
    app::install(app);
    let code = window::run_message_loop();
    app::uninstall();
    tracing::info!("Bento Desk stopped ({code})");
    Ok(())
}
