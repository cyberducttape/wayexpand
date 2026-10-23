//! Screen layout for `wayexpand-ui`.
//!
//! `frame` turns a [`View`] into exactly `height` lines, each exactly `width`
//! terminal cells wide, so the writer can paint every row in place without
//! clearing the screen and nothing depends on the terminal's line wrapping.
//! It does no I/O, which keeps the layout testable at any terminal size.

use crossterm::style::Color;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use wayexpand_core::{Config, ExpansionConfig, MatchMode};

use crate::{DaemonState, InsertionAssurance, Tone};

/// Smallest terminal the full layout fits in.
pub(crate) const MIN_WIDTH: u16 = 40;
pub(crate) const MIN_HEIGHT: u16 = 8;
/// From this width the details sit beside the list instead of below it.
const WIDE_WIDTH: u16 = 100;
/// Width of the label column in the details pane.
const LABEL_WIDTH: usize = 14;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub fg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub reverse: bool,
}

impl Style {
    const fn plain() -> Self {
        Self {
            fg: None,
            bold: false,
            dim: false,
            italic: false,
            reverse: false,
        }
    }
    const fn fg(color: Color) -> Self {
        Self {
            fg: Some(color),
            ..Self::plain()
        }
    }
    const fn bold(self) -> Self {
        Self { bold: true, ..self }
    }
    const fn dim(self) -> Self {
        Self { dim: true, ..self }
    }
    const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }
    const fn reverse(self) -> Self {
        Self {
            reverse: true,
            ..self
        }
    }
}

const ACCENT: Color = Color::Cyan;
const KEY: Style = Style::fg(ACCENT).bold();
const LABEL: Style = Style::fg(ACCENT);
const HEADING: Style = Style::plain().bold();

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub text: String,
    pub style: Style,
}

pub(crate) type Line = Vec<Span>;

fn span(text: impl Into<String>, style: Style) -> Span {
    Span {
        text: text.into(),
        style,
    }
}

/// An open text prompt: its label, the text so far, and the caret position
/// in characters.
pub(crate) struct PromptView<'a> {
    pub label: &'a str,
    pub input: &'a str,
    pub cursor: usize,
}

/// Everything one frame shows.
pub(crate) struct View<'a> {
    pub path: &'a str,
    pub config: &'a Config,
    pub visible: &'a [usize],
    pub selected: usize,
    pub scroll: usize,
    pub query: &'a str,
    pub searching: bool,
    pub daemon: DaemonState,
    pub insertion: Option<InsertionAssurance>,
    pub message: &'a str,
    pub tone: Tone,
    pub prompt: Option<PromptView<'a>>,
    pub confirm_delete: bool,
    pub preview: &'a str,
    pub preview_app: &'a str,
    pub show_help: bool,
}

/// Where the parts of the screen go for one terminal size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub too_small: bool,
    /// Details beside the list rather than below it.
    pub wide: bool,
    /// Rows between the filter bar and the message line.
    pub body_rows: usize,
    pub list_width: usize,
    /// Rows for snippets (the list pane minus its column header).
    pub list_rows: usize,
    /// Rows of the details pane when it is below the list; 0 hides it.
    pub detail_rows: usize,
}

impl Layout {
    /// `items` is the number of snippets in the list; a short list leaves
    /// its spare rows to the details below it.
    pub(crate) fn new(width: u16, height: u16, items: usize) -> Self {
        let too_small = width < MIN_WIDTH || height < MIN_HEIGHT;
        // Header, filter bar, message line, and key bar.
        let body_rows = usize::from(height).saturating_sub(4);
        let width_cells = usize::from(width);
        let wide = width >= WIDE_WIDTH;
        let (list_width, list_rows, detail_rows) = if wide {
            (width_cells * 11 / 20, body_rows.saturating_sub(1), 0)
        } else if body_rows >= 12 {
            let detail_min = (body_rows / 2).clamp(8, 14);
            // The column header plus one row per snippet (or the empty note).
            let list_height = (items.max(1) + 1).clamp(3, body_rows - detail_min);
            (width_cells, list_height - 1, body_rows - list_height)
        } else {
            (width_cells, body_rows.saturating_sub(1), 0)
        };
        Self {
            too_small,
            wide,
            body_rows,
            list_width,
            list_rows,
            detail_rows,
        }
    }
}

/// The first visible list row after moving the selection: unchanged while the
/// selection stays on screen, otherwise the nearest offset that shows it.
pub(crate) fn keep_visible(scroll: usize, selected: usize, rows: usize, len: usize) -> usize {
    if rows == 0 || len == 0 {
        return 0;
    }
    let scroll = if selected < scroll {
        selected
    } else if selected >= scroll + rows {
        selected + 1 - rows
    } else {
        scroll
    };
    scroll.min(len.saturating_sub(rows))
}

pub(crate) fn frame(view: &View<'_>, width: u16, height: u16) -> Vec<Line> {
    let layout = Layout::new(width, height, view.visible.len());
    let width = usize::from(width);
    let height = usize::from(height);
    if layout.too_small {
        let mut lines = vec![finish(
            vec![span(
                format!("Enlarge the terminal to at least {MIN_WIDTH}×{MIN_HEIGHT}"),
                Style::plain().bold(),
            )],
            width,
            Style::plain(),
        )];
        lines.resize_with(height, || blank(width));
        return lines;
    }

    let mut lines = Vec::with_capacity(height);
    lines.push(header(view, width));
    lines.push(filter_bar(view, width));
    if view.show_help {
        lines.extend(help(width, layout.body_rows));
    } else if layout.wide {
        let detail_width = width - layout.list_width - 1;
        let list = list_pane(view, layout.list_width, layout.body_rows);
        let details = details(view, detail_width, layout.body_rows);
        for (mut row, detail) in list.into_iter().zip(details) {
            row.push(span("│", Style::plain().dim()));
            row.extend(detail);
            lines.push(row);
        }
    } else {
        let list_height = layout.body_rows - layout.detail_rows;
        lines.extend(list_pane(view, width, list_height));
        if layout.detail_rows > 0 {
            lines.extend(details(view, width, layout.detail_rows));
        }
    }
    lines.push(message_line(view, width));
    lines.push(key_bar(view, width));
    lines
}

fn header(view: &View<'_>, width: usize) -> Line {
    let mut right = Vec::new();
    let (daemon, daemon_style) = match view.daemon {
        DaemonState::Checking => ("… checking daemon", Style::plain().dim()),
        DaemonState::Running => ("● Running", Style::fg(Color::Green).bold()),
        DaemonState::Paused => ("‖ Paused", Style::fg(Color::Yellow).bold()),
        DaemonState::Stopped => ("○ Daemon not running", Style::plain()),
    };
    right.push(span(daemon, daemon_style));
    if let Some(insertion) = view.insertion {
        let (text, color) = match insertion {
            InsertionAssurance::Verified => ("Verified insertion", Color::Green),
            InsertionAssurance::Compatibility => ("Compatibility insertion", Color::Yellow),
            InsertionAssurance::Blocked => ("Typed expansions blocked", Color::Red),
        };
        right.push(span("  ", Style::plain()));
        right.push(span(text, Style::fg(color)));
    }
    right.push(span(" ", Style::plain()));
    let left = vec![
        span(" WayExpand Settings", Style::fg(ACCENT).bold()),
        span("  ", Style::plain()),
        span(view.path, Style::plain().dim()),
    ];
    justify(left, right, width, Style::plain())
}

fn filter_bar(view: &View<'_>, width: usize) -> Line {
    let total = view.config.expansion.len();
    let shown = view.visible.len();
    let mut left = vec![span(" / ", KEY)];
    if view.searching {
        left.push(span(view.query, Style::plain().bold()));
        left.push(span(" ", Style::plain().reverse()));
    } else if view.query.is_empty() {
        left.push(span("search snippets", Style::plain().dim()));
    } else {
        left.push(span(view.query, Style::plain().bold()));
        left.push(span("   Esc clears", Style::plain().dim()));
    }
    let count = match (view.query.is_empty(), shown) {
        (true, 0) => "no snippets ".to_owned(),
        (true, _) => format!("{} of {total} ", view.selected + 1),
        (false, 0) => format!("no matches · {total} in all "),
        (false, _) => format!(
            "{} of {shown} {} · {total} in all ",
            view.selected + 1,
            plural(shown, "match", "matches")
        ),
    };
    justify(
        left,
        vec![span(count, Style::plain())],
        width,
        Style::plain(),
    )
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 {
        one
    } else {
        many
    }
}

/// The column header and snippet rows, `rows` lines in all.
fn list_pane(view: &View<'_>, width: usize, rows: usize) -> Vec<Line> {
    let mut lines = Vec::with_capacity(rows);
    let trigger_width = view
        .visible
        .iter()
        .map(|&index| view.config.expansion[index].trigger.width())
        .max()
        .unwrap_or(7)
        .clamp(7, (width / 3).clamp(7, 24));
    // Marker and state: " ❯ ● ".
    let lead = 5;
    lines.push(finish(
        vec![
            span(" ".repeat(lead), Style::plain()),
            span(pad("TRIGGER", trigger_width), HEADING.dim()),
            span("  DESCRIPTION", HEADING.dim()),
        ],
        width,
        Style::plain(),
    ));
    if view.visible.is_empty() {
        let text = if view.config.expansion.is_empty() {
            "   No snippets yet. Press n to create one.".to_owned()
        } else {
            format!(
                "   Nothing matches “{}”. Esc clears the search.",
                view.query
            )
        };
        lines.push(finish(
            vec![span(text, Style::plain().italic())],
            width,
            Style::plain(),
        ));
    }
    for (row, &index) in view
        .visible
        .iter()
        .enumerate()
        .skip(view.scroll)
        .take(rows.saturating_sub(1))
    {
        lines.push(list_row(
            &view.config.expansion[index],
            row == view.selected,
            trigger_width,
            width,
        ));
    }
    lines.resize_with(rows, || blank(width));
    lines.truncate(rows);
    lines
}

fn list_row(
    expansion: &ExpansionConfig,
    selected: bool,
    trigger_width: usize,
    width: usize,
) -> Line {
    let base = if selected {
        Style::plain().reverse().bold()
    } else if expansion.enabled {
        Style::plain()
    } else {
        Style::plain().dim()
    };
    let colored = |color: Color| {
        if selected {
            base
        } else {
            Style {
                fg: Some(color),
                ..base
            }
        }
    };
    let mut badges = Vec::new();
    if matches!(expansion.match_mode, MatchMode::WordBoundary) {
        badges.push("word");
    }
    if expansion.command.is_some() {
        badges.push("cmd");
    }
    if !expansion.app_filter.is_empty() {
        badges.push("app");
    }
    if expansion.output_file.is_some() {
        badges.push("file");
    }
    if !expansion.enabled {
        badges.push("off");
    }
    let badges = if badges.is_empty() {
        String::new()
    } else {
        format!(" {} ", badges.join(" "))
    };
    let (description, description_style) = if expansion.description.trim().is_empty() {
        (
            first_line(&expansion.replacement),
            if selected { base } else { base.italic().dim() },
        )
    } else {
        (expansion.description.clone(), base)
    };
    let left = vec![
        span(if selected { " ❯ " } else { "   " }, colored(ACCENT)),
        span(
            if expansion.enabled { "● " } else { "○ " },
            colored(if expansion.enabled {
                Color::Green
            } else {
                Color::Yellow
            }),
        ),
        span(pad(&expansion.trigger, trigger_width), base.bold()),
        span("  ", base),
        span(description, description_style),
    ];
    let right = vec![span(badges, colored(Color::Magenta))];
    justify(left, right, width, base)
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let more = text.lines().filter(|line| !line.trim().is_empty()).count() > 1;
    let mut shown = printable(line.trim());
    if more {
        shown.push_str(" …");
    }
    shown
}

/// The selected snippet's fields and its rendered preview, `rows` lines in
/// all. The first line is a title rule.
fn details(view: &View<'_>, width: usize, rows: usize) -> Vec<Line> {
    let mut lines = Vec::with_capacity(rows);
    let expansion = view
        .visible
        .get(view.selected)
        .map(|&index| &view.config.expansion[index]);
    let title = expansion.map_or_else(|| "Details".to_owned(), |e| e.trigger.clone());
    lines.push(rule(&title, width));
    let Some(expansion) = expansion else {
        lines.push(finish(
            vec![span(
                " Select a snippet to see it here.",
                Style::plain().italic(),
            )],
            width,
            Style::plain(),
        ));
        lines.resize_with(rows, || blank(width));
        lines.truncate(rows);
        return lines;
    };

    let mut field = |label: &str, value: Vec<Span>| {
        let mut line = vec![span(format!(" {}", pad(label, LABEL_WIDTH - 1)), LABEL)];
        line.extend(value);
        lines.push(finish(line, width, Style::plain()));
    };
    let plain = |text: String| vec![span(text, Style::plain())];
    if !expansion.description.trim().is_empty() {
        field("Description", plain(expansion.description.clone()));
    }
    let mode = match expansion.match_mode {
        MatchMode::Immediate => "expands immediately",
        MatchMode::WordBoundary => "waits for a word boundary",
    };
    field(
        "Status",
        vec![
            if expansion.enabled {
                span("● Enabled", Style::fg(Color::Green))
            } else {
                span("○ Disabled", Style::fg(Color::Yellow))
            },
            span(format!(" · {mode}"), Style::plain()),
        ],
    );
    if !expansion.aliases.is_empty() {
        field("Aliases", plain(expansion.aliases.join(", ")));
    }
    if !expansion.tags.is_empty() {
        field("Tags", plain(expansion.tags.join(", ")));
    }
    if !expansion.category.is_empty() {
        field("Category", plain(expansion.category.clone()));
    }
    if !expansion.app_filter.is_empty() {
        field("Only in", plain(expansion.app_filter.join(", ")));
    }
    if let Some(command) = &expansion.command {
        let program = command.action.as_deref().map_or_else(
            || command.program.clone(),
            |action| format!("action {action}"),
        );
        field("Runs", plain(program));
    }
    if let Some(output_file) = &expansion.output_file {
        field("Writes file", plain(output_file.clone()));
    }

    let mut preview_title = vec![span(" Preview", HEADING)];
    if !view.preview_app.is_empty() {
        preview_title.push(span(
            format!(" as {}", view.preview_app),
            Style::plain().dim(),
        ));
    }
    lines.push(finish(preview_title, width, Style::plain()));
    let room = rows.saturating_sub(lines.len());
    let text_width = width.saturating_sub(3).max(1);
    let wrapped: Vec<String> = view
        .preview
        .split('\n')
        .flat_map(|line| wrap(&printable(line), text_width))
        .collect();
    let shown = if wrapped.len() > room {
        room.saturating_sub(1)
    } else {
        wrapped.len()
    };
    for line in &wrapped[..shown] {
        lines.push(finish(
            vec![
                span(" │ ", Style::fg(ACCENT).dim()),
                span(line.clone(), Style::fg(Color::Green)),
            ],
            width,
            Style::plain(),
        ));
    }
    if shown < wrapped.len() && room > 0 {
        let hidden = wrapped.len() - shown;
        lines.push(finish(
            vec![span(
                format!(" │ … {hidden} more {}", plural(hidden, "line", "lines")),
                Style::plain().dim(),
            )],
            width,
            Style::plain(),
        ));
    }
    lines.resize_with(rows, || blank(width));
    lines.truncate(rows);
    lines
}

fn rule(title: &str, width: usize) -> Line {
    let title = format!(" {} ", fit(title, width.saturating_sub(6)));
    let used = 2 + title.width();
    finish(
        vec![
            span("──", Style::plain().dim()),
            span(title, Style::fg(ACCENT).bold()),
            span("─".repeat(width.saturating_sub(used)), Style::plain().dim()),
        ],
        width,
        Style::plain(),
    )
}

fn help(width: usize, rows: usize) -> Vec<Line> {
    const GROUPS: &[(&str, &[(&str, &str)])] = &[
        (
            "Move",
            &[
                ("↑ ↓  k j", "previous / next snippet"),
                ("PgUp PgDn", "a page up / down"),
                ("Home End  g G", "first / last snippet"),
            ],
        ),
        (
            "Search",
            &[
                ("/", "search triggers, descriptions and tags"),
                ("Enter", "keep the search and go back to the list"),
                ("Esc", "clear the search"),
            ],
        ),
        (
            "Edit",
            &[
                ("n", "new snippet"),
                ("e   E", "edit the replacement here / in $EDITOR"),
                ("D   t", "edit the description / tags"),
                ("m", "switch immediate / word-boundary matching"),
                ("Space", "enable or disable"),
                ("d d", "delete (press d twice)"),
                ("u", "undo the last change"),
            ],
        ),
        (
            "Other",
            &[
                ("a", "preview as if typed in an app"),
                ("p", "pause or resume the daemon"),
                ("r", "reload the file from disk"),
                ("q  Ctrl-C", "quit"),
            ],
        ),
    ];
    let mut lines = vec![rule("Keys", width)];
    for (group, keys) in GROUPS {
        lines.push(finish(
            vec![span(format!(" {group}"), HEADING)],
            width,
            Style::plain(),
        ));
        for (key, action) in *keys {
            lines.push(finish(
                vec![
                    span(format!("   {}", pad(key, 16)), KEY),
                    span(*action, Style::plain()),
                ],
                width,
                Style::plain(),
            ));
        }
    }
    lines.push(finish(
        vec![span(
            " Press any key to close this help.",
            Style::plain().italic(),
        )],
        width,
        Style::plain(),
    ));
    lines.resize_with(rows, || blank(width));
    lines.truncate(rows);
    lines
}

fn message_line(view: &View<'_>, width: usize) -> Line {
    let style = match view.tone {
        Tone::Info => Style::plain(),
        Tone::Success => Style::fg(Color::Green),
        Tone::Warning => Style::fg(Color::Yellow),
        Tone::Error => Style::fg(Color::Red).bold(),
    };
    let right = if view.prompt.is_some() {
        hints(&[("Enter", "save"), ("Esc", "cancel"), ("Ctrl-U", "clear")])
    } else {
        Vec::new()
    };
    justify(
        vec![span(format!(" {}", printable(view.message)), style)],
        right,
        width,
        Style::plain(),
    )
}

fn key_bar(view: &View<'_>, width: usize) -> Line {
    if let Some(prompt) = &view.prompt {
        return prompt_line(prompt, width);
    }
    let keys: &[(&str, &str)] = if view.confirm_delete {
        &[("d", "delete"), ("Esc", "keep it")]
    } else if view.searching {
        &[("Enter", "keep search"), ("Esc", "clear"), ("↑↓", "move")]
    } else if view.show_help {
        &[("Esc", "close help")]
    } else {
        &[
            ("↑↓", "move"),
            ("/", "search"),
            ("n", "new"),
            ("e", "edit"),
            ("E", "$EDITOR"),
            ("Space", "on/off"),
            ("d", "delete"),
            ("u", "undo"),
            ("p", "pause"),
        ]
    };
    let tail: &[(&str, &str)] = if view.confirm_delete || view.searching || view.show_help {
        &[]
    } else {
        &[("?", "help"), ("q", "quit")]
    };
    let tail_width = line_width(&hints(tail));
    let mut chosen = Vec::new();
    for key in keys {
        let candidate: Vec<_> = chosen.iter().chain(std::iter::once(key)).copied().collect();
        if line_width(&hints(&candidate)) + tail_width + 1 > width {
            break;
        }
        chosen.push(*key);
    }
    chosen.extend_from_slice(tail);
    let mut line = vec![span(" ", Style::plain())];
    line.extend(hints(&chosen));
    finish(line, width, Style::plain())
}

fn hints(keys: &[(&str, &str)]) -> Line {
    let mut line = Vec::new();
    for (position, (key, action)) in keys.iter().enumerate() {
        if position > 0 {
            line.push(span("  ", Style::plain()));
        }
        line.push(span(*key, KEY));
        line.push(span(format!(" {action}"), Style::plain()));
    }
    line.push(span(" ", Style::plain()));
    line
}

/// The prompt label and its text, scrolled so the caret stays in view.
fn prompt_line(prompt: &PromptView<'_>, width: usize) -> Line {
    let label = format!(" {} › ", prompt.label);
    let room = width.saturating_sub(label.width() + 1).max(1);
    // Newlines and tabs show as one visible character each, so the caret
    // index still lines up with the input's characters.
    let characters: Vec<char> = prompt
        .input
        .chars()
        .map(|character| match character {
            '\n' => '↵',
            '\t' => '→',
            other if other.is_control() => '·',
            other => other,
        })
        .collect();
    let cursor = prompt.cursor.min(characters.len());
    // Start late enough that everything up to and including the caret fits:
    // walk back from the caret, keeping a cell for it and one for the `…`
    // shown when the start is cut. This visits only what fits on the line,
    // so a replacement of any size stays cheap to redraw on every key.
    let mut start = cursor;
    let mut used = 0;
    while start > 0 {
        let width = characters[start - 1].width().unwrap_or(0);
        let ellipsis = usize::from(start > 1);
        if used + width + 1 + ellipsis > room {
            break;
        }
        used += width;
        start -= 1;
    }
    let mut line = vec![span(label, Style::fg(ACCENT).bold())];
    if start > 0 {
        line.push(span("…", Style::plain().dim()));
    }
    line.push(span(
        characters[start..cursor].iter().collect::<String>(),
        Style::plain(),
    ));
    let at_caret = characters.get(cursor).copied().unwrap_or(' ');
    line.push(span(at_caret.to_string(), Style::plain().reverse()));
    if cursor < characters.len() {
        line.push(span(
            characters[cursor + 1..].iter().collect::<String>(),
            Style::plain(),
        ));
    }
    finish(line, width, Style::plain())
}

/// `left` then `right` pushed to the right edge, all exactly `width` wide.
/// When both do not fit, `left` is cut short so `right` stays readable.
fn justify(left: Line, right: Line, width: usize, fill: Style) -> Line {
    let right_width = line_width(&right);
    if right_width >= width {
        return finish(right, width, fill);
    }
    let room = width - right_width;
    let mut line = if line_width(&left) > room.saturating_sub(1) {
        // Keep a gap so the cut-off text and `right` do not run together.
        let mut line = finish(left, room.saturating_sub(1), fill);
        line.push(span(" ", fill));
        line
    } else {
        finish(left, room, fill)
    };
    line.extend(right);
    line
}

/// Cuts `line` to `width` cells (ending in `…` when cut) and pads it with
/// spaces in `fill` to exactly `width`.
fn finish(line: Line, width: usize, fill: Style) -> Line {
    let total = line_width(&line);
    let mut out = Vec::with_capacity(line.len() + 1);
    if total > width {
        let mut used = 0;
        for piece in line {
            let piece_width = piece.text.width();
            if used + piece_width < width {
                used += piece_width;
                out.push(piece);
                continue;
            }
            let text = fit(&piece.text, width - used);
            used += text.width();
            out.push(span(text, piece.style));
            break;
        }
        if used < width {
            out.push(span(" ".repeat(width - used), fill));
        }
        return out;
    }
    out.extend(line);
    if total < width {
        out.push(span(" ".repeat(width - total), fill));
    }
    out
}

fn blank(width: usize) -> Line {
    vec![span(" ".repeat(width), Style::plain())]
}

pub(crate) fn line_width(line: &Line) -> usize {
    line.iter().map(|piece| piece.text.width()).sum()
}

/// `text` cut to at most `width` cells, ending in `…` when cut.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width - 1 {
            break;
        }
        used += character_width;
        out.push(character);
    }
    out.push('…');
    out
}

/// `text` fitted and padded with spaces to exactly `width` cells.
fn pad(text: &str, width: usize) -> String {
    let mut out = fit(text, width);
    let used = out.width();
    out.extend(std::iter::repeat_n(' ', width.saturating_sub(used)));
    out
}

/// One line of text with tabs expanded and other control characters, which
/// would move the terminal cursor, shown as `·`.
fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\t' => out.push_str("    "),
            '\r' => {}
            other if other.is_control() => out.push('·'),
            other => out.push(other),
        }
    }
    out
}

/// Hard-wraps `text` into lines of at most `width` cells.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut used = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width > width {
            lines.push(String::new());
            used = 0;
        }
        used += character_width;
        lines.last_mut().expect("at least one line").push(character);
    }
    lines
}

/// The frame as plain text, one string per row, for tests.
#[cfg(test)]
pub(crate) fn plain_text(lines: &[Line]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line.iter().map(|piece| piece.text.as_str()).collect())
        .collect()
}
