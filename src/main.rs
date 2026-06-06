//! Entry point for vrandr.
//!
//! Bootstraps the terminal, queries xrandr, and runs the main event loop.
//! All real logic lives in the other modules; this file is intentionally
//! thin.

mod app;
mod command;
mod config;
mod edid;
mod model;
mod ui;
mod xrandr;

use anyhow::{Context, Result};
use crossterm::{
    event::DisableMouseCapture,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use std::time::Duration;

fn main() -> Result<()> {
    // If invoked with --version / --help, do that without touching the TUI.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vrandr {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "vrandr {} – Vim-style TUI for xrandr\n\
             \n\
             USAGE:\n  vrandr\n\
             \n\
             See the on-screen help (press '?') for the full key map.\n\
             Configuration is stored at $XDG_CONFIG_HOME/vrandr/layout.toml\n",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    let outputs = xrandr::query_xrandr().context("failed to read xrandr state")?;
    let app = app::build_app(outputs);

    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, DisableMouseCapture)
        .context("failed to switch to alternate screen")?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).context("failed to init terminal")?;

    let tick_rate = Duration::from_millis(250);
    let res = app::run(&mut terminal, app, tick_rate);

    // Always restore the terminal, even on error.
    disable_raw_mode().ok();
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .ok();
    terminal.show_cursor().ok();

    res
}
