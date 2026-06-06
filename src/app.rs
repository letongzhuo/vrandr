//! Main application state, event loop, and key bindings.

use crate::command;
use crate::config::{self, LayoutConfig, PersistedOutput};
use crate::model::{CustomMode, Output, PendingChange, PendingMap, Reflection, RelativePosition};
use crate::ui;
use crate::xrandr;
#[allow(unused_imports)]
use anyhow::{anyhow, Result};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::{Duration, Instant};

/// Which pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Output,
    Mode,
}

/// Modal dialogs. `Error` carries a one-shot error message.
#[derive(Debug, Clone)]
pub enum ModalKind {
    Help,
    /// Raw-only EDID view (kept for backwards-compat and tests).
    #[allow(dead_code)]
    Edid(usize),
    /// Variant of EDID that pre-renders both the raw hex and the parsed
    /// summary. We keep the parsed view in the variant itself so the renderer
    /// doesn't have to recompute it every frame.
    EdidParsed(usize, Vec<String>),
    Input(InputModal),
    QuitConfirm,
    Error(String),
    /// Profile selection list. `Vec<String>` are profile names; `selected`
    /// is the cursor position (0-indexed).
    ProfileList { profiles: Vec<String>, selected: usize },
}

#[derive(Debug, Clone)]
pub struct InputModal {
    pub title: &'static str,
    pub prompt: String,
    pub buffer: String,
    pub error: Option<String>,
    #[allow(dead_code)]
    pub target: InputTarget,
    /// Last blink tick. Kept for future use (we currently always show the
    /// caret) and to allow the modal struct to be cheaply re-used.
    #[allow(dead_code)]
    pub cursor_blink_at: Instant,
    pub on_submit_kind: InputKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum InputTarget {
    Scale,
    /// Reserved for a future `--scale-from` modal.
    ScaleFrom,
    Gamma,
    SameAs,
    /// Absolute `--pos <x>x<y>`.
    Position,
    /// `--panning <w>x<h>`.
    Panning,
    /// Filename under `~/.config/vrandr/profiles/` to write the current
    /// state to (the `W` key).
    ProfileName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum InputKind {
    /// Reserved for future single-f32 inputs.
    ParseF32,
    ParseScale,
    ParseGamma,
    ParseName,
    ParsePosition,
    ParsePanning,
    ParseProfileName,
}

impl InputModal {
    pub fn cursor_visible(&self, _now: Instant) -> bool {
        // The cursor is always visible inside an input modal.
        true
    }
}

/// Pending state of the TUI.
pub struct App {
    pub outputs: Vec<Output>,
    pub pending: PendingMap,
    pub selected_output: usize,
    pub selected_mode: usize,
    pub focus: Focus,
    pub modal: Option<ModalKind>,
    pub status_message: Option<(String, Instant)>,
    pub should_quit: bool,
    pub tick: u64,
    /// The last previewed command, used to show the user what will run on `a`.
    pub last_preview: String,
    /// Name of the most recently applied profile, if any. When set, the
    /// preview pane shows a single-line summary ("Applied profile: …")
    /// rather than the staged-command preview, so the user can tell the
    /// last action was a profile load.
    pub last_applied_profile: Option<String>,
    /// Scroll offset (in lines) for the help modal. Reset to 0 every time
    /// the help modal is opened.
    pub help_scroll: u16,
    /// Buffered key presses that haven't yet formed a complete key.
    pub _unused: (),
}

impl App {
    pub fn new(outputs: Vec<Output>) -> Self {
        Self {
            outputs,
            pending: PendingMap::new(),
            selected_output: 0,
            selected_mode: 0,
            focus: Focus::Output,
            modal: None,
            status_message: None,
            should_quit: false,
            tick: 0,
            last_preview: String::new(),
            last_applied_profile: None,
            help_scroll: 0,
            _unused: (),
        }
    }

    /// Refresh the list of outputs from xrandr.
    pub fn refresh(&mut self) -> Result<()> {
        let outs = xrandr::query_xrandr()?;
        self.outputs = outs;
        if self.selected_output >= self.outputs.len() {
            self.selected_output = self.outputs.len().saturating_sub(1);
        }
        if self.selected_mode
            >= self
                .outputs
                .get(self.selected_output)
                .map(|o| o.available_modes.len())
                .unwrap_or(0)
        {
            self.selected_mode = 0;
        }
        Ok(())
    }

    /// Return the output currently selected, if any.
    pub fn selected_output(&self) -> Option<&Output> {
        self.outputs.get(self.selected_output)
    }

    pub fn selected_output_mut(&mut self) -> Option<&mut Output> {
        self.outputs.get_mut(self.selected_output)
    }

    /// Push a transient status message to display in the status bar.
    pub fn flash(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), Instant::now()));
    }

    /// Returns a clone of the current status message if it is still within
    /// the display window (2 seconds).
    #[allow(dead_code)]
    pub fn current_status(&self) -> Option<&str> {
        match &self.status_message {
            Some((s, t)) if t.elapsed() < Duration::from_secs(2) => Some(s.as_str()),
            _ => None,
        }
    }

    /// True if the user has staged any user-visible changes.
    pub fn has_pending(&self) -> bool {
        self.pending.values().any(|c| c.has_visible_change())
    }

    /// Locate the neighbour in the connected list relative to the selected
    /// output. Used by H/J/K/L.
    fn neighbour(&self, dir: NeighbourDir) -> Option<usize> {
        let me = self.selected_output;
        let connected: Vec<usize> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.connected)
            .map(|(i, _)| i)
            .collect();
        let pos = connected.iter().position(|&i| i == me)?;
        let target = match dir {
            NeighbourDir::Left => pos.checked_sub(1),
            NeighbourDir::Right => {
                let p = pos + 1;
                if p < connected.len() {
                    Some(p)
                } else {
                    None
                }
            }
            NeighbourDir::Above | NeighbourDir::Below => None, // not meaningful in a 1D list
        }?;
        Some(connected[target])
    }
}

#[derive(Debug, Clone, Copy)]
enum NeighbourDir {
    Left,
    Right,
    Above,
    Below,
}

/// Top-level event loop: draw, handle events, repeat until `should_quit`.
pub fn run<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    mut app: App,
    tick_rate: Duration,
) -> Result<()> {
    let mut last_tick = Instant::now();
    loop {
        app.tick = app.tick.wrapping_add(1);
        terminal.draw(|f| ui::draw(f, &mut app))?;
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = crossterm::event::read()? {
                handle_key(&mut app, key);
            }
        }
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
        if app.should_quit {
            return Ok(());
        }
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Modal keys take precedence.
    if app.modal.is_some() {
        handle_modal_key(app, key);
        return;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let code = key.code;
    match (ctrl, code) {
        (false, KeyCode::Char('q')) => {
            if app.has_pending() {
                app.modal = Some(ModalKind::QuitConfirm);
            } else {
                app.should_quit = true;
            }
        }
        (false, KeyCode::Char('?')) => {
            app.help_scroll = 0;
            app.modal = Some(ModalKind::Help);
        }
        (false, KeyCode::Char('a')) => {
            // Any new staged change supersedes the "applied profile"
            // summary in the preview pane.
            app.last_applied_profile = None;
            apply_pending(app);
        }
        (false, KeyCode::Char('r')) => {
            // In normal view: reset pending.
            // In modal view: handled by modal key handler.
            if !app.pending.is_empty() {
                app.pending.clear();
                app.flash("Pending changes cleared");
            }
        }
        (false, KeyCode::Char('W')) => {
            // Export current state (with any pending changes materialised)
            // to a named profile. The user types the name in a follow-up
            // input modal.
            open_input(
                app,
                "export",
                "Enter profile name (saved to ~/.config/vrandr/profiles/<name>.toml):",
                InputTarget::ProfileName,
                InputKind::ParseProfileName,
            );
        }
        (false, KeyCode::Char('T')) => {
            // Show the list of saved profiles. Pressing Enter on one
            // applies it directly to xrandr.
            //
            // Note: the original spec asked for `L`, but `L` is already
            // used by the relative-placement binding (place output
            // right-of neighbour). `T` ("Take profile") is unused and
            // keeps the same role. The conflict is documented in the
            // help screen.
            match config::list_profiles() {
                Ok(names) if names.is_empty() => {
                    app.flash("No saved profiles yet – press W to export one");
                }
                Ok(names) => {
                    app.modal = Some(ModalKind::ProfileList {
                        profiles: names,
                        selected: 0,
                    });
                }
                Err(e) => app.modal = Some(ModalKind::Error(format!("{e:#}"))),
            }
        }
        (false, KeyCode::Char('j')) | (false, KeyCode::Down) => move_selection(app, 1),
        (false, KeyCode::Char('k')) | (false, KeyCode::Up) => move_selection(app, -1),
        (false, KeyCode::Char('h')) | (false, KeyCode::Left) => {
            app.focus = Focus::Output;
        }
        (false, KeyCode::Char('l')) | (false, KeyCode::Right) => {
            app.focus = Focus::Mode;
        }
        (false, KeyCode::Tab) => {
            app.focus = match app.focus {
                Focus::Output => Focus::Mode,
                Focus::Mode => Focus::Output,
            };
        }
        (false, KeyCode::Enter) => stage_mode(app),
        (false, KeyCode::Char('m')) => {
            let name = match app.selected_output().map(|o| o.name.clone()) {
                Some(n) => n,
                None => return,
            };
            // Set this as primary and clear primary on all others.
            for o in &mut app.outputs {
                o.is_primary = o.name == name;
            }
            let entry = app.pending.entry(name.clone()).or_default();
            entry.primary = Some(true);
            app.flash(format!("{name} will become primary"));
        }
        (false, KeyCode::Char('d')) => {
            let name = match app.selected_output().map(|o| o.name.clone()) {
                Some(n) => n,
                None => return,
            };
            let entry = app.pending.entry(name.clone()).or_default();
            entry.off = Some(true);
            // Turning off supersedes mode.
            entry.mode = None;
            app.flash(format!("{name} will be turned off"));
        }
        (false, KeyCode::Char('p')) => {
            let name = match app.selected_output().map(|o| o.name.clone()) {
                Some(n) => n,
                None => return,
            };
            let entry = app.pending.entry(name.clone()).or_default();
            entry.off = Some(false);
            app.flash(format!("{name} will be turned on (--auto)"));
        }
        (false, KeyCode::Char('x')) => {
            // Cycle reflection
            let name = match app.selected_output().map(|o| o.name.clone()) {
                Some(n) => n,
                None => return,
            };
            let cur = app.selected_output().map(|o| o.reflection).unwrap_or_default();
            let next = cur.next();
            if let Some(o) = app.selected_output_mut() {
                o.reflection = next;
            }
            let entry = app.pending.entry(name.clone()).or_default();
            entry.reflection = Some(next);
            app.flash(format!("{name} reflection -> {}", next.as_str()));
        }
        (false, KeyCode::Char('X')) => {
            // Direct reset of reflection to normal.
            let name = match app.selected_output().map(|o| o.name.clone()) {
                Some(n) => n,
                None => return,
            };
            if let Some(o) = app.selected_output_mut() {
                o.reflection = Reflection::Normal;
            }
            let entry = app.pending.entry(name.clone()).or_default();
            entry.reflection = Some(Reflection::Normal);
            app.flash(format!("{name} reflection -> normal"));
        }
        (false, KeyCode::Char('R')) => {
            // Rotation cycle. The spec uses lowercase 'r' for rotation but
            // also for "reset pending" – we resolve the conflict by binding
            // rotation to uppercase R and reset to lowercase r, which is
            // easier to discover and matches the convention of capitalising
            // destructive actions on the modal layer.
            cycle_rotation(app);
        }
        (false, KeyCode::Char('[')) => step_scale(app, -0.1),
        (false, KeyCode::Char(']')) => step_scale(app, 0.1),
        (false, KeyCode::Char('s')) => {
            open_input(
                app,
                "scale",
                "Enter scale, e.g. 0.8 or 0.8x0.6:",
                InputTarget::Scale,
                InputKind::ParseScale,
            );
        }
        (false, KeyCode::Char('g')) => {
            open_input(
                app,
                "gamma",
                "Enter gamma R:G:B, e.g. 1.0:0.8:0.8:",
                InputTarget::Gamma,
                InputKind::ParseGamma,
            );
        }
        (false, KeyCode::Char('e')) => {
            // Open the EDID popup with both the raw hex and a parsed
            // human-readable summary. The parsed lines are computed once
            // when the modal opens; the renderer just shows them.
            let idx = app.selected_output;
            let parsed = app
                .outputs
                .get(idx)
                .and_then(|o| o.edid.as_deref())
                .and_then(crate::edid::EdidInfo::parse)
                .map(|info| info.summary_lines())
                .unwrap_or_default();
            app.modal = Some(ModalKind::EdidParsed(idx, parsed));
        }
        (false, KeyCode::Char('H')) => {
            place_relative(app, NeighbourDir::Left, RelativePosition::LeftOf);
        }
        (false, KeyCode::Char('L')) => {
            place_relative(app, NeighbourDir::Right, RelativePosition::RightOf);
        }
        (false, KeyCode::Char('K')) => {
            place_relative(app, NeighbourDir::Above, RelativePosition::Above);
        }
        (false, KeyCode::Char('J')) => {
            place_relative(app, NeighbourDir::Below, RelativePosition::Below);
        }
        (false, KeyCode::Char('P')) => {
            // Clone another output.
            open_input(
                app,
                "clone",
                "Enter output to clone (--same-as):",
                InputTarget::SameAs,
                InputKind::ParseName,
            );
        }
        (false, KeyCode::Char('O')) => {
            // Absolute position. Note: staged absolute position clears any
            // previously staged relative position (handled in submit_input).
            open_input(
                app,
                "position",
                "Enter absolute position, e.g. 1920x0:",
                InputTarget::Position,
                InputKind::ParsePosition,
            );
        }
        (false, KeyCode::Char('N')) => {
            // Panning area.
            open_input(
                app,
                "panning",
                "Enter panning area, e.g. 1920x1080:",
                InputTarget::Panning,
                InputKind::ParsePanning,
            );
        }
        _ => {}
    }
}

fn handle_modal_key(app: &mut App, key: KeyEvent) {
    let modal = app.modal.clone();
    match modal {
        Some(ModalKind::Help) => {
            // Help modal supports scrolling. j/Down/PageDown advance, k/Up/
            // PageUp go back. Esc / q / ? close.
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    app.modal = None;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    app.help_scroll = app.help_scroll.saturating_add(1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.help_scroll = app.help_scroll.saturating_sub(1);
                }
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    app.help_scroll = app.help_scroll.saturating_add(10);
                }
                KeyCode::PageUp => {
                    app.help_scroll = app.help_scroll.saturating_sub(10);
                }
                KeyCode::Home => app.help_scroll = 0,
                _ => {}
            }
        }
        Some(ModalKind::Edid(_)) | Some(ModalKind::EdidParsed(_, _)) | Some(ModalKind::Error(_)) => {
            if key.code == KeyCode::Esc || (key.code == KeyCode::Char('q')) {
                app.modal = None;
            }
        }
        Some(ModalKind::QuitConfirm) => match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                apply_pending(app);
                app.should_quit = true;
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                app.pending.clear();
                app.should_quit = true;
            }
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Esc => {
                app.modal = None;
            }
            _ => {}
        },
        Some(ModalKind::ProfileList {
            profiles,
            selected,
        }) => match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.modal = None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if !profiles.is_empty() {
                    let n = (selected + 1) % profiles.len();
                    app.modal = Some(ModalKind::ProfileList {
                        profiles: profiles.clone(),
                        selected: n,
                    });
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !profiles.is_empty() {
                    let n = (selected + profiles.len() - 1) % profiles.len();
                    app.modal = Some(ModalKind::ProfileList {
                        profiles: profiles.clone(),
                        selected: n,
                    });
                }
            }
            KeyCode::Char('d') => {
                // Delete the currently highlighted profile.
                if let Some(name) = profiles.get(selected) {
                    let path = config::profile_path(name);
                    match std::fs::remove_file(&path) {
                        Ok(_) => {
                            let mut names = profiles.clone();
                            names.remove(selected);
                            if names.is_empty() {
                                app.modal = None;
                                app.flash(format!("Deleted profile {name}"));
                            } else {
                                let n = selected.min(names.len() - 1);
                                app.modal = Some(ModalKind::ProfileList {
                                    profiles: names,
                                    selected: n,
                                });
                                app.flash(format!("Deleted profile {name}"));
                            }
                        }
                        Err(e) => {
                            app.modal = Some(ModalKind::Error(format!("{e:#}")));
                        }
                    }
                }
            }
            KeyCode::Enter => {
                if let Some(name) = profiles.get(selected).cloned() {
                    apply_profile(app, &name);
                }
            }
            _ => {}
        },
        Some(ModalKind::Input(mut input)) => match key.code {
            KeyCode::Esc => app.modal = None,
            KeyCode::Enter => {
                if let Err(e) = submit_input(app, &input) {
                    input.error = Some(e.to_string());
                    app.modal = Some(ModalKind::Input(input));
                } else {
                    app.modal = None;
                }
            }
            KeyCode::Backspace => {
                input.buffer.pop();
                app.modal = Some(ModalKind::Input(input));
            }
            KeyCode::Char(c) => {
                input.buffer.push(c);
                app.modal = Some(ModalKind::Input(input));
            }
            _ => {}
        },
        None => {}
    }
}

fn move_selection(app: &mut App, delta: isize) {
    match app.focus {
        Focus::Output => {
            let len = app.outputs.len();
            if len == 0 {
                return;
            }
            let cur = app.selected_output as isize;
            let n = (cur + delta).rem_euclid(len as isize) as usize;
            app.selected_output = n;
            app.selected_mode = 0;
        }
        Focus::Mode => {
            let len = app
                .outputs
                .get(app.selected_output)
                .map(|o| o.available_modes.len())
                .unwrap_or(0);
            if len == 0 {
                return;
            }
            let cur = app.selected_mode as isize;
            let n = (cur + delta).rem_euclid(len as isize) as usize;
            app.selected_mode = n;
        }
    }
}

fn stage_mode(app: &mut App) {
    // Only meaningful when focus is on the mode list, but we accept it from
    // anywhere for convenience.
    let name = match app.selected_output().map(|o| o.name.clone()) {
        Some(n) => n,
        None => return,
    };
    let mode = match app
        .selected_output()
        .and_then(|o| o.available_modes.get(app.selected_mode))
        .cloned()
    {
        Some(m) => m,
        None => return,
    };
    let entry = app.pending.entry(name.clone()).or_default();
    entry.mode = Some(mode.clone());
    entry.off = None; // staging a mode implies turning the output on
    app.flash(format!(
        "Staged {name} -> {} @ {:.2}Hz",
        mode.name, mode.refresh_rate
    ));
}

fn cycle_rotation(app: &mut App) {
    let name = match app.selected_output().map(|o| o.name.clone()) {
        Some(n) => n,
        None => return,
    };
    let cur = app.selected_output().map(|o| o.rotation).unwrap_or_default();
    let next = cur.next();
    if let Some(o) = app.selected_output_mut() {
        o.rotation = next;
    }
    let entry = app.pending.entry(name.clone()).or_default();
    entry.rotation = Some(next);
    app.flash(format!("{name} rotation -> {}", next.as_str()));
}

fn step_scale(app: &mut App, delta: f32) {
    let name = match app.selected_output().map(|o| o.name.clone()) {
        Some(n) => n,
        None => return,
    };
    let cur = app
        .selected_output()
        .and_then(|o| o.scale)
        .unwrap_or((1.0, 1.0));
    let sx = (cur.0 + delta).clamp(0.1, 2.0);
    let sy = (cur.1 + delta).clamp(0.1, 2.0);
    if let Some(o) = app.selected_output_mut() {
        o.scale = Some((sx, sy));
    }
    let entry = app.pending.entry(name.clone()).or_default();
    entry.scale = Some((sx, sy));
    app.flash(format!("{name} scale -> {sx:.2}x{sy:.2}"));
}

fn place_relative(app: &mut App, dir: NeighbourDir, pos: RelativePosition) {
    let target = match app.neighbour(dir) {
        Some(i) => i,
        None => {
            app.flash("No neighbour in that direction");
            return;
        }
    };
    let me = match app.selected_output().map(|o| o.name.clone()) {
        Some(n) => n,
        None => return,
    };
    let other = app.outputs[target].name.clone();
    let entry = app.pending.entry(me.clone()).or_default();
    entry.relative_to = Some(Some((other.clone(), pos)));
    app.flash(format!("{me} will be placed {} {}", pos.as_str(), other));
}

fn open_input(
    app: &mut App,
    title: &'static str,
    prompt: impl Into<String>,
    target: InputTarget,
    kind: InputKind,
) {
    app.modal = Some(ModalKind::Input(InputModal {
        title,
        prompt: prompt.into(),
        buffer: String::new(),
        error: None,
        target,
        cursor_blink_at: Instant::now(),
        on_submit_kind: kind,
    }));
}

fn submit_input(app: &mut App, input: &InputModal) -> Result<()> {
    let name = app
        .selected_output()
        .map(|o| o.name.clone())
        .ok_or_else(|| anyhow::anyhow!("no output selected"))?;
    let entry = app.pending.entry(name.clone()).or_default();
    let buf = input.buffer.trim();
    if buf.is_empty() {
        anyhow::bail!("empty input");
    }
    match input.on_submit_kind {
        InputKind::ParseF32 => {
            // No f32 inputs are currently exposed; keep the variant reserved
            // for future fields (e.g. CSD backlight).
            let _v: f32 = buf.parse().map_err(|_| anyhow::anyhow!("not a number"))?;
        }
        InputKind::ParseScale => {
            // Accept "0.8" (uniform) or "0.8x0.6".
            if let Some((a, b)) = buf.split_once('x') {
                let sx: f32 = a
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid x factor"))?;
                let sy: f32 = b
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid y factor"))?;
                if !(0.1..=8.0).contains(&sx) || !(0.1..=8.0).contains(&sy) {
                    anyhow::bail!("scale must be in 0.1..=8.0");
                }
                entry.scale = Some((sx, sy));
            } else {
                let s: f32 = buf
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid scale"))?;
                if !(0.1..=8.0).contains(&s) {
                    anyhow::bail!("scale must be in 0.1..=8.0");
                }
                entry.scale = Some((s, s));
            }
        }
        InputKind::ParseGamma => {
            let parts: Vec<&str> = buf.split(':').collect();
            if parts.len() != 3 {
                anyhow::bail!("gamma must be R:G:B");
            }
            let r: f32 = parts[0].parse()?;
            let g: f32 = parts[1].parse()?;
            let b: f32 = parts[2].parse()?;
            if !(0.1..=10.0).contains(&r) || !(0.1..=10.0).contains(&g) || !(0.1..=10.0).contains(&b)
            {
                anyhow::bail!("gamma components must be in 0.1..=10.0");
            }
            entry.gamma = Some((r, g, b));
        }
        InputKind::ParseName => {
            // Same-as: pick a connected output with this name.
            if !app.outputs.iter().any(|o| o.name == buf) {
                anyhow::bail!("unknown output: {buf}");
            }
            entry.relative_to = Some(Some((buf.to_string(), RelativePosition::SameAs)));
        }
        InputKind::ParsePosition => {
            // Accept "<x>x<y>". Setting an absolute position clears any
            // previously staged relative position.
            let (xs, ys) = buf
                .split_once('x')
                .ok_or_else(|| anyhow::anyhow!("expected <x>x<y>"))?;
            let x: i32 = xs
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid x"))?;
            let y: i32 = ys
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid y"))?;
            entry.position = Some((x, y));
            entry.relative_to = Some(None);
        }
        InputKind::ParsePanning => {
            // Accept "<w>x<h>".
            let (ws, hs) = buf
                .split_once('x')
                .ok_or_else(|| anyhow::anyhow!("expected <w>x<h>"))?;
            let w: u32 = ws
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid width"))?;
            let h: u32 = hs
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid height"))?;
            if w == 0 || h == 0 {
                anyhow::bail!("panning dimensions must be > 0");
            }
            entry.panning = Some((w, h));
        }
        InputKind::ParseProfileName => {
            // Always capture the *live* xrandr state before persisting.
            // `app.outputs` is the in-memory snapshot from the last
            // refresh and may be stale if the user adjusted anything
            // outside vrandr (e.g. via a second monitor plugged in
            // between launches, or by running xrandr in another shell).
            // The bug report was that two different profile names ended
            // up with the same TOML: that is consistent with us
            // materialising against a stale snapshot. Re-querying here
            // removes the staleness as a class.
            match xrandr::query_xrandr() {
                Ok(fresh) => {
                    app.outputs = fresh;
                }
                Err(e) => {
                    app.modal = Some(ModalKind::Error(format!(
                        "refreshing xrandr state before save failed: {e:#}"
                    )));
                    return Ok(());
                }
            }
            let _ = config::validate_profile_name(buf)?;
            // Build the snapshot (current state with pending materialised)
            // and write it to disk. The trailing "X updated" flash below
            // doesn't apply to a profile export, so we return early.
            let snapshot: Vec<Output> = app
                .outputs
                .iter()
                .map(|o| {
                    let change = app.pending.get(&o.name).cloned().unwrap_or_default();
                    command::materialize(o, &change)
                })
                .collect();
            let cfg = LayoutConfig::from_outputs(&snapshot);
            // Debug: print the TOML to stderr so the user can verify the
            // saved content actually reflects their changes (helpful
            // when two profile names accidentally share the same body).
            // Toggle the `VRANDR_DEBUG_SAVE` env var on/off if the log
            // becomes noisy.
            if std::env::var_os("VRANDR_DEBUG_SAVE").is_some() {
                eprintln!(
                    "[vrandr] saving profile {buf:?} ({} outputs):\n{}",
                    cfg.outputs.len(),
                    cfg.to_toml().unwrap_or_else(|e| format!("<serialise error: {e:#}>"))
                );
            }
            match config::save_profile(buf, &cfg) {
                Ok(path) => {
                    app.flash(format!("Profile saved to {}", path.display()));
                }
                Err(e) => {
                    app.modal = Some(ModalKind::Error(format!("{e:#}")));
                }
            }
            return Ok(());
        }
    }
    app.flash(format!("{name} updated"));
    Ok(())
}

fn apply_pending(app: &mut App) {
    if !app.has_pending() {
        app.flash("No pending changes to apply");
        return;
    }
    match command::apply(&app.pending) {
        Ok(preview) => {
            app.last_preview = preview;
            app.pending.clear();
            app.flash("Applied successfully");
            if let Err(e) = app.refresh() {
                app.modal = Some(ModalKind::Error(format!("{e:#}")));
            }
        }
        Err(e) => {
            app.modal = Some(ModalKind::Error(format!("{e:#}")));
        }
    }
}

#[allow(dead_code)]
fn save_layout(app: &mut App) {
    // The previous "S" key binding used to call this. The new design
    // routes saving through the `W` -> profile-name modal in
    // `submit_input` (InputKind::ParseProfileName). This function is
    // kept around in case a future key needs to dump the layout to the
    // legacy `layout.toml` file, and for the unit-style fallback.
    let snapshot: Vec<Output> = app
        .outputs
        .iter()
        .map(|o| {
            let change = app.pending.get(&o.name).cloned().unwrap_or_default();
            command::materialize(o, &change)
        })
        .collect();
    let cfg = LayoutConfig::from_outputs(&snapshot);
    match config::save(&cfg) {
        Ok(path) => app.flash(format!("Saved to {}", path.display())),
        Err(e) => app.modal = Some(ModalKind::Error(format!("{e:#}"))),
    }
}

#[allow(dead_code)]
fn load_layout_into_pending(app: &mut App) -> Result<()> {
    // Retained for the same reason as `save_layout`: the previous
    // on-startup "load saved layout?" prompt has been removed, but this
    // helper is useful as a one-shot CLI hook or future "import"
    // binding.
    let (_, cfg) = config::load()?.ok_or_else(|| anyhow::anyhow!("no saved layout"))?;
    for p in &cfg.outputs {
        let pending = persisted_output_to_pending(p);
        app.pending.insert(p.name.clone(), pending);
    }
    for cm in cfg.custom_modes {
        let target = app
            .outputs
            .iter()
            .position(|o| o.custom_modes.iter().any(|c| c.name == cm.name))
            .or_else(|| app.outputs.iter().position(|o| o.connected))
            .unwrap_or(0);
        if let Some(o) = app.outputs.get_mut(target) {
            o.custom_modes.push(CustomMode {
                name: cm.name.clone(),
                modeline: cm.modeline.clone(),
            });
        }
        let entry = app
            .pending
            .entry(app.outputs[target].name.clone())
            .or_default();
        entry.custom_modes.push(CustomMode {
            name: cm.name.clone(),
            modeline: cm.modeline.clone(),
        });
    }
    app.flash("Loaded saved layout into pending");
    Ok(())
}

/// Apply a named profile to the live xrandr state and refresh the UI.
fn apply_profile(app: &mut App, name: &str) {
    // Build the pending map from the saved config. Outputs that are no
    // longer connected are silently skipped (we still load the entry
    // because `command::apply` will ignore unknown outputs, but we
    // surface a hint if *nothing* matched).
    let (path, cfg) = match config::load_profile_by_name(name) {
        Ok(Some(v)) => v,
        Ok(None) => {
            app.modal = Some(ModalKind::Error(format!(
                "profile {name:?} disappeared (was: {})",
                config::profile_path(name).display()
            )));
            return;
        }
        Err(e) => {
            app.modal = Some(ModalKind::Error(format!("{e:#}")));
            return;
        }
    };
    let mut pending: PendingMap = PendingMap::new();
    for p in &cfg.outputs {
        pending.insert(p.name.clone(), persisted_output_to_pending(p));
    }
    let cmd_preview = match command::apply(&pending) {
        Ok(s) => s,
        Err(e) => {
            app.modal = Some(ModalKind::Error(format!("{e:#}")));
            return;
        }
    };
    app.pending.clear();
    app.last_preview = cmd_preview;
    // Mark the latest action as "applied a profile" so the preview pane
    // shows a single-line summary instead of the staged command.
    app.last_applied_profile = Some(name.to_string());
    app.flash(format!("Applied profile {name} ({})", path.display()));
    if let Err(e) = app.refresh() {
        app.modal = Some(ModalKind::Error(format!("{e:#}")));
    } else {
        app.modal = None;
    }
}

fn persisted_output_to_pending(p: &PersistedOutput) -> PendingChange {
    config::to_pending(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Mode, Reflection, Rotation};

    fn sample_output(name: &str) -> Output {
        Output {
            name: name.into(),
            connected: true,
            current_mode: Some(Mode::new("1920x1080".into(), 60.0, true, false)),
            available_modes: vec![Mode::new("1920x1080".into(), 60.0, true, false)],
            is_primary: false,
            rotation: Rotation::Normal,
            reflection: Reflection::Normal,
            scale: None,
            scale_from: None,
            gamma: None,
            position: Some((0, 0)),
            relative_to: None,
            off: false,
            edid: None,
            custom_modes: vec![],
        }
    }

    #[test]
    fn save_profile_uses_staged_rotation_in_toml() {
        // The save flow must materialise the *pending* change into the
        // snapshot before serialising — otherwise the saved TOML would
        // record the (possibly stale) `app.outputs` value. This is the
        // user-visible half of the "save always writes the same content"
        // bug: even if `app.outputs` is fresh, ignoring the pending
        // change would make the file look identical to one saved before
        // the user staged a rotation.
        let mut app = App::new(vec![sample_output("eDP-1")]);
        app.pending.insert(
            "eDP-1".into(),
            PendingChange {
                rotation: Some(Rotation::Left),
                ..Default::default()
            },
        );
        let snapshot: Vec<Output> = app
            .outputs
            .iter()
            .map(|o| {
                let change = app.pending.get(&o.name).cloned().unwrap_or_default();
                command::materialize(o, &change)
            })
            .collect();
        let cfg = LayoutConfig::from_outputs(&snapshot);
        let toml = cfg.to_toml().unwrap();
        assert!(
            toml.contains("left"),
            "expected the staged rotation to be saved, got:\n{toml}"
        );
        assert!(
            !toml.contains("rotation = \"normal\""),
            "rotation = normal should not appear when the user staged Left"
        );
    }

    #[test]
    fn preview_height_collapses_for_profile_summary() {
        let mut app = App::new(vec![sample_output("eDP-1")]);
        app.last_applied_profile = Some("home".into());
        let h = ui::preview_height(&app, 80);
        assert_eq!(h, 1, "profile summary should always be 1 line");
    }

    #[test]
    fn preview_height_grows_with_command_length() {
        let mut app = App::new(vec![sample_output("eDP-1")]);
        // Stage a change that produces a multi-arg command so the
        // preview has something to wrap.
        let mode = Mode::new("1920x1080".into(), 60.0, true, false);
        app.pending.insert(
            "eDP-1".into(),
            PendingChange {
                off: Some(false),
                mode: Some(mode),
                position: Some((0, 0)),
                panning: Some((9999, 9999)),
                primary: Some(true),
                rotation: Some(Rotation::Left),
                reflection: Some(Reflection::X),
                ..Default::default()
            },
        );
        let h = ui::preview_height(&app, 80);
        assert!(
            h >= 1 && h <= ui::MAX_PREVIEW_LINES as u16,
            "preview height {h} out of bounds"
        );
    }
}


/// Public constructor used by `main` after querying xrandr.
///
/// On startup we do *not* auto-load any saved profile. The user invokes
/// `L` to pick one. The previous "found a saved layout, load it?"
/// prompt has been removed deliberately: the new design treats startup
/// as a clean slate, and saved profiles are an explicit opt-in.
pub fn build_app(outputs: Vec<Output>) -> App {
    App::new(outputs)
}
