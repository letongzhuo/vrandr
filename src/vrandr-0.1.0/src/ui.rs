//! Modal popups: help, EDID, input, quit confirmation.

use crate::app::{App, Focus, ModalKind};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

/// Maximum number of lines the preview pane can grow to when the user
/// has manually staged a long command. Above this we wrap (the wrap
/// drops the tail). The point is to always show as much of the
/// command as the terminal can fit, while leaving room for the
/// status line and the body panes.
pub const MAX_PREVIEW_LINES: usize = 5;

/// Top-level draw routine. The split is:
///
/// ```text
/// ┌────────────────┬────────────────┐
/// │ output list    │ mode list      │
/// │                │                │
/// ├────────────────┴────────────────┤
/// │ status line                     │
/// │ preview line (1..=5 rows)       │
/// └─────────────────────────────────┘
/// ```
///
/// The bottom region is `1 + preview_height` lines, where
/// `preview_height` is computed from the staged command length and
/// capped at `MAX_PREVIEW_LINES`. The preview is always visible, even on
/// small terminals — the minimum terminal height is 5 rows.
pub(crate) fn preview_height(app: &App, width: u16) -> u16 {
    if app.last_applied_profile.is_some() || app.pending.is_empty() {
        return 1;
    }
    let width = width.max(1) as usize;
    let needed = preview_text(app).chars().count().div_ceil(width).max(1);
    needed.min(MAX_PREVIEW_LINES) as u16
}

/// Compute the text rendered inside the preview pane.
pub fn preview_text(app: &App) -> String {
    if let Some(name) = &app.last_applied_profile {
        return format!("Applied profile: {name}");
    }
    crate::command::preview_command(&app.pending)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.size();
    if area.height < 5 {
        // Nothing useful to draw on a sub-5-line terminal; just show a hint.
        let p = Paragraph::new("Terminal too small — resize to at least 5 rows.");
        f.render_widget(p, area);
        return;
    }
    let preview_h = preview_height(app, area.width);
    // Reserve 1 line for the status bar and `preview_h` lines for the
    // preview pane. The body panes take whatever's left.
    let body_h = area.height.saturating_sub(1 + preview_h);
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(body_h),
            Constraint::Length(1),
            Constraint::Length(preview_h),
        ])
        .split(area);
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(v[0]);

    render_output_list(f, app, body[0]);
    render_mode_list(f, app, body[1]);
    render_status_line(f, app, v[1]);
    render_preview_line(f, app, v[2]);

    if app.modal.is_some() {
        render_modal(f, app);
    }
}

/// Render the active modal, if any. Always clears the area first.
pub fn render_modal(f: &mut Frame, app: &mut App) {
    let kind = match &app.modal {
        Some(m) => m.clone(),
        None => return,
    };
    let area = centered_rect(70, 70, f.size());
    f.render_widget(Clear, area);
    match kind {
        ModalKind::Help => render_help(f, app, area),
        ModalKind::Edid(idx) => render_edid(f, app, area, idx),
        ModalKind::EdidParsed(idx, parsed) => render_edid_parsed(f, app, area, idx, &parsed),
        ModalKind::Input(input) => render_input(f, app, area, &input),
        ModalKind::QuitConfirm => render_quit_confirm(f, area),
        ModalKind::Error(msg) => render_error(f, area, msg),
        ModalKind::ProfileList { profiles, selected } => {
            render_profile_list(f, area, &profiles, selected)
        }
    }
}

fn render_help(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Help (j/k or PgUp/PgDn to scroll, Esc to close) ")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));
    f.render_widget(block, area);

    let inner = inner_rect(area);
    let help_text: Vec<Line> = vec![
        Line::from(Span::styled(
            "Movement",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from("  j / Down   move down in current list"),
        Line::from("  k / Up     move up in current list"),
        Line::from("  h / Left   focus the output list"),
        Line::from("  l / Right  focus the mode list"),
        Line::from("  Tab        toggle focus between lists"),
        Line::from(""),
        Line::from(Span::styled(
            "Mode & state",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from("  Enter      stage the highlighted mode (name + rate)"),
        Line::from("  m          set as primary"),
        Line::from("  d          turn output off (--off)"),
        Line::from("  p          turn output on (--auto)"),
        Line::from("  R (upper)  rotate normal -> left -> right -> inverted"),
        Line::from("  x          reflect normal -> x -> y -> xy"),
        Line::from("  X (upper)  reset reflection directly to normal"),
        Line::from(""),
        Line::from(Span::styled(
            "Positioning",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from("  h/j/k/l    place current output left/below/above/right of neighbour"),
        Line::from("  O (upper)  set absolute position (--pos <x>x<y>)"),
        Line::from("  N (upper)  set panning area (--panning <w>x<h>)"),
        Line::from("  P (upper)  clone (--same-as) another output"),
        Line::from(""),
        Line::from(Span::styled(
            "Adjustments",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from("  [ / ]      step scale by -0.1 / +0.1"),
        Line::from("  s          enter exact scale factor"),
        Line::from("  g          enter gamma (R:G:B)"),
        Line::from("  e          show EDID (raw + parsed)"),
        Line::from(""),
        Line::from(Span::styled(
            "Global",
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from("  a          apply all staged changes"),
        Line::from("  r          discard all staged changes"),
        Line::from("  W (upper)  export current state as a named profile"),
        Line::from("              (~/.config/vrandr/profiles/<name>.toml)"),
        Line::from("  T (upper)  load a saved profile (Enter to apply, d to delete)"),
        Line::from("  ?          this help"),
        Line::from("  q          quit (asks to apply staged changes first)"),
        Line::from(""),
    ];

    // Clamp the scroll offset so the help never goes past its end.
    let max_offset = help_text.len().saturating_sub(inner.height as usize) as u16;
    let offset = app.help_scroll.min(max_offset);
    let p = Paragraph::new(help_text)
        .scroll((offset, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

fn render_edid(f: &mut Frame, app: &App, area: Rect, idx: usize) {
    let title = if let Some(out) = app.outputs.get(idx) {
        format!(" EDID: {} (Esc to close) ", out.name)
    } else {
        " EDID (Esc to close) ".to_string()
    };
    let block = Block::default().title(title).borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_rect(area);

    let text = match app.outputs.get(idx).and_then(|o| o.edid.as_ref()) {
        Some(bytes) if !bytes.is_empty() => {
            // Format as 16-byte rows of hex.
            let mut lines: Vec<Line> = Vec::new();
            for (i, chunk) in bytes.chunks(16).enumerate() {
                let mut spans: Vec<Span> = Vec::new();
                spans.push(Span::styled(
                    format!("{:04x}: ", i * 16),
                    Style::default().fg(Color::DarkGray),
                ));
                for b in chunk {
                    spans.push(Span::raw(format!("{:02x} ", b)));
                }
                lines.push(Line::from(spans));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("(total {} bytes)", bytes.len()),
                Style::default().fg(Color::DarkGray),
            )));
            lines
        }
        _ => vec![Line::from("(no EDID available)")],
    };
    let p = Paragraph::new(text).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

/// EDID popup that shows *both* the raw hex dump and a parsed summary
/// (manufacturer, product, serial, size, gamma).
fn render_edid_parsed(f: &mut Frame, app: &App, area: Rect, idx: usize, parsed: &[String]) {
    let title = if let Some(out) = app.outputs.get(idx) {
        format!(" EDID: {} (Esc to close) ", out.name)
    } else {
        " EDID (Esc to close) ".to_string()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));
    f.render_widget(block, area);
    let inner = inner_rect(area);

    let mut lines: Vec<Line> = Vec::new();
    // Section 1: parsed summary.
    lines.push(Line::from(Span::styled(
        " Summary ",
        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    if parsed.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (could not parse EDID)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for l in parsed {
            lines.push(Line::from(format!("  {l}")));
        }
    }
    lines.push(Line::from(""));

    // Section 2: raw hex dump.
    if let Some(bytes) = app.outputs.get(idx).and_then(|o| o.edid.as_ref()) {
        if !bytes.is_empty() {
            lines.push(Line::from(Span::styled(
                " Raw ",
                Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            for (i, chunk) in bytes.chunks(16).enumerate() {
                let mut spans: Vec<Span> = Vec::new();
                spans.push(Span::styled(
                    format!("  {:04x}: ", i * 16),
                    Style::default().fg(Color::DarkGray),
                ));
                for b in chunk {
                    spans.push(Span::raw(format!("{:02x} ", b)));
                }
                lines.push(Line::from(spans));
            }
            lines.push(Line::from(Span::styled(
                format!("  ({} bytes)", bytes.len()),
                Style::default().fg(Color::DarkGray),
            )));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  (no EDID available for this output)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

fn render_input(f: &mut Frame, _app: &App, area: Rect, input: &crate::app::InputModal) {
    let title = format!(" {} (Enter to confirm, Esc to cancel) ", input.title);
    let block = Block::default().title(title).borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_rect(area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(inner);

    let prompt = Paragraph::new(Line::from(Span::styled(
        input.prompt.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    f.render_widget(prompt, layout[0]);

    let buf = if input.cursor_visible(std::time::Instant::now()) {
        format!("{}_", input.buffer)
    } else {
        input.buffer.clone()
    };
    let p = Paragraph::new(buf).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, layout[1]);

    if let Some(err) = &input.error {
        let p = Paragraph::new(err.as_str())
            .style(Style::default().fg(Color::LightRed))
            .alignment(Alignment::Left);
        f.render_widget(p, layout[2]);
    }
}

fn render_quit_confirm(f: &mut Frame, area: Rect) {
    let block = Block::default().title(" Quit? ").borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_rect(area);
    let text = vec![
        Line::from("There are staged changes that have not been applied."),
        Line::from(""),
        Line::from(Span::styled(
            "  [y]  apply and quit",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  [n]  discard changes and quit",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  [c]  cancel",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    let p = Paragraph::new(text);
    f.render_widget(p, inner);
}

fn render_error(f: &mut Frame, area: Rect, msg: String) {
    let block = Block::default()
        .title(" Error (Esc to close) ")
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::LightRed));
    f.render_widget(block, area);
    let inner = inner_rect(area);
    let p = Paragraph::new(msg).wrap(Wrap { trim: false });
    f.render_widget(p, inner);
}

fn render_profile_list(f: &mut Frame, area: Rect, profiles: &[String], selected: usize) {
    let block = Block::default()
        .title(" Load profile (j/k, Enter to apply, d to delete, Esc to close) ")
        .borders(Borders::ALL);
    f.render_widget(block, area);
    let inner = inner_rect(area);

    if profiles.is_empty() {
        let p = Paragraph::new(Line::from("No saved profiles."));
        f.render_widget(p, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(profiles.len() + 2);
    lines.push(Line::from("Saved profiles:"));
    lines.push(Line::from(""));
    for (i, name) in profiles.iter().enumerate() {
        let marker = if i == selected { "> " } else { "  " };
        let style = if i == selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{name}"), style)));
    }
    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

pub(crate) fn render_output_list(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Output;
    let block = Block::default()
        .title(" Outputs ")
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });
    let items: Vec<ListItem> = app
        .outputs
        .iter()
        .enumerate()
        .map(|(i, out)| {
            let marker = if i == app.selected_output { ">" } else { " " };
            let primary = if out.is_primary { "*" } else { " " };
            let pending = if app.pending.contains_key(&out.name) {
                "+"
            } else {
                " "
            };
            let line = format!(
                "{} {}{}  {:<10}  {}",
                marker, primary, pending, out.name, out.status_label()
            );
            let style = if i == app.selected_output {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else if !out.connected {
                Style::default().fg(Color::DarkGray)
            } else if app.pending.contains_key(&out.name) {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

pub(crate) fn render_mode_list(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Mode;
    let block = Block::default()
        .title(" Modes ")
        .borders(Borders::ALL)
        .border_style(if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        });

    let items: Vec<ListItem> = if let Some(out) = app.outputs.get(app.selected_output) {
        out.available_modes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let marker = if i == app.selected_mode { ">" } else { " " };
                let flags = match (m.is_current, m.is_preferred) {
                    (true, true) => " *+",
                    (true, false) => " * ",
                    (false, true) => "  +",
                    (false, false) => "   ",
                };
                let line = format!("{} {}  {}  {:>6.2}Hz", marker, flags, m.name, m.refresh_rate);
                let style = if i == app.selected_mode {
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD)
                } else if m.is_current {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                };
                ListItem::new(line).style(style)
            })
            .collect()
    } else {
        vec![ListItem::new("(no output selected)")]
    };
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

/// Top status line: a row of shortcut hints on the left, the currently
/// selected output name and pending-change count in the middle, and any
/// transient flash message (e.g. "Staged HDMI-1 -> 1920x1080 @ 60Hz") on
/// the right.
pub(crate) fn render_status_line(f: &mut Frame, app: &App, area: Rect) {
    let pending_count = app
        .pending
        .values()
        .filter(|c| c.has_visible_change())
        .count();
    let selected = app
        .outputs
        .get(app.selected_output)
        .map(|o| o.name.as_str())
        .unwrap_or("-");

    let mut spans: Vec<Span> = vec![
        Span::styled("[?]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" help  "),
        Span::styled("[a]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" apply  "),
        Span::styled("[W]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" save profile  "),
        Span::styled("[T]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" load profile  "),
        Span::styled("[r]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" reset  "),
        Span::styled("[e]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" EDID  "),
        Span::styled("[q]", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" quit   "),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::raw(" sel: "),
        Span::styled(
            format!("{selected:<10}"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" pending: "),
        Span::styled(
            format!("{pending_count}"),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];

    // Right-aligned flash message, if any.
    if let Some(msg) = app.current_status() {
        let width = area.width as usize;
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if width > used + msg.chars().count() + 1 {
            spans.push(Span::raw(" ".repeat(width - used - msg.chars().count() - 1)));
            spans.push(Span::styled(
                msg,
                Style::default().fg(Color::Yellow),
            ));
        } else {
            // Not enough room; just append.
            spans.push(Span::raw("  "));
            spans.push(Span::styled(msg, Style::default().fg(Color::Yellow)));
        }
    }

    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, area);
}

/// Render the preview pane. The pane may be 1..=MAX_PREVIEW_LINES rows
/// tall. For manually-staged changes the full command is shown (wrapped
/// across the available rows). For a profile apply the pane collapses
/// to a single line that names the profile. The placeholder
/// "xrandr  (no pending changes)" is shown when there's nothing staged
/// and no profile was just applied.
pub(crate) fn render_preview_line(f: &mut Frame, app: &App, area: Rect) {
    let full = preview_text(app);
    let width = area.width as usize;
    let height = area.height as usize;

    // Pick a style: cyan when the user has staged a manual change, dim
    // italic when nothing is staged, dark green when a profile was just
    // applied. These are consistent with the rest of the UI.
    let style = if app.last_applied_profile.is_some() {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::ITALIC)
    } else if app.pending.is_empty() {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(Color::Cyan)
    };

    if width == 0 || height == 0 {
        return;
    }

    // Wrap the text across `height` rows of `width` columns. The user
    // spec says manual staging should never be truncated; we use the
    // full text and let ratatui wrap. If the wrapped height exceeds the
    // available rows, the tail is clipped (no ellipsis – the full
    // command remains in the model's preview_command() for later).
    let visible = if app.last_applied_profile.is_some() {
        // Single-line summary: truncate with an ellipsis if it doesn't
        // fit on one row.
        if full.chars().count() <= width {
            full
        } else {
            truncate(&full, width)
        }
    } else if app.pending.is_empty() {
        // Placeholder fits in one line.
        full
    } else {
        // Manual staging: always show as much as the pane can fit, by
        // character count, never inserting an ellipsis (the user expects
        // the full command to be visible). The Paragraph's Wrap will
        // break on the next available whitespace, mirroring what
        // xrandr's argv would look like.
        let max_chars = width * height;
        if full.chars().count() <= max_chars {
            full
        } else {
            // Truncate by characters (rounding down to a char boundary)
            // so the wrap doesn't split mid-codepoint.
            full.chars().take(max_chars).collect()
        }
    };
    let p = Paragraph::new(visible)
        .style(style)
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

/// Truncate `s` so the result has at most `width` characters. If truncation
/// is needed, the last character is replaced with `…`.
fn truncate(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if s.chars().count() <= width {
        return s.to_string();
    }
    let mut out: String = s.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn inner_rect(area: Rect) -> Rect {
    if area.width < 2 || area.height < 2 {
        return area;
    }
    Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width - 2,
        height: area.height - 2,
    }
}
