use crate::ui_consts::FOOTER_INDENT_COLS;
use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::WidgetRef;
use std::iter;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FooterProps {
    pub(crate) mode: FooterMode,
    pub(crate) esc_backtrack_hint: bool,
    pub(crate) use_shift_enter_hint: bool,
    pub(crate) is_task_running: bool,
    pub(crate) context_window_percent: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FooterMode {
    CtrlCReminder,
    ShortcutPrompt,
    ShortcutOverlay,
    EscHint,
    Empty,
}

pub(crate) fn toggle_shortcut_mode(current: FooterMode, ctrl_c_hint: bool) -> FooterMode {
    if ctrl_c_hint && matches!(current, FooterMode::CtrlCReminder) {
        return current;
    }

    match current {
        FooterMode::ShortcutOverlay | FooterMode::CtrlCReminder => FooterMode::ShortcutPrompt,
        _ => FooterMode::ShortcutOverlay,
    }
}

pub(crate) fn esc_hint_mode(current: FooterMode, is_task_running: bool) -> FooterMode {
    if is_task_running {
        current
    } else {
        FooterMode::EscHint
    }
}

pub(crate) fn reset_mode_after_activity(current: FooterMode) -> FooterMode {
    match current {
        FooterMode::EscHint
        | FooterMode::ShortcutOverlay
        | FooterMode::CtrlCReminder
        | FooterMode::Empty => FooterMode::ShortcutPrompt,
        other => other,
    }
}

pub(crate) fn footer_height(props: FooterProps) -> u16 {
    footer_lines(props).len() as u16
}

pub(crate) fn render_footer(area: Rect, buf: &mut Buffer, props: FooterProps) {
    let lines = footer_lines(props);
    for (idx, line) in lines.into_iter().enumerate() {
        let y = area.y + idx as u16;
        if y >= area.y + area.height {
            break;
        }
        let row = Rect::new(area.x, y, area.width, 1);
        line.render_ref(row, buf);
    }
}

fn footer_lines(props: FooterProps) -> Vec<Line<'static>> {
    match props.mode {
        FooterMode::CtrlCReminder => vec![ctrl_c_reminder_line(CtrlCReminderState {
            is_task_running: props.is_task_running,
        })],
        FooterMode::ShortcutPrompt => {
            if props.is_task_running {
                vec![context_window_line(props.context_window_percent)]
            } else {
                vec![dim_line(indent_text("? for shortcuts"))]
            }
        }
        FooterMode::ShortcutOverlay => shortcut_overlay_lines(ShortcutsState {
            use_shift_enter_hint: props.use_shift_enter_hint,
            esc_backtrack_hint: props.esc_backtrack_hint,
        }),
        FooterMode::EscHint => vec![esc_hint_line(props.esc_backtrack_hint)],
        FooterMode::Empty => Vec::new(),
    }
}

#[derive(Clone, Copy, Debug)]
struct CtrlCReminderState {
    is_task_running: bool,
}

#[derive(Clone, Copy, Debug)]
struct ShortcutsState {
    use_shift_enter_hint: bool,
    esc_backtrack_hint: bool,
}

fn ctrl_c_reminder_line(state: CtrlCReminderState) -> Line<'static> {
    let action = if state.is_task_running {
        "interrupt"
    } else {
        "quit"
    };
    let text = format!("ctrl + c again to {action}");
    dim_line(indent_text(&text))
}

fn esc_hint_line(esc_backtrack_hint: bool) -> Line<'static> {
    let text = if esc_backtrack_hint {
        "esc again to edit previous message"
    } else {
        "esc esc to edit previous message"
    };
    dim_line(indent_text(text))
}

fn shortcut_overlay_lines(state: ShortcutsState) -> Vec<Line<'static>> {
    let mut commands = String::new();
    let mut submit = String::new();
    let mut newline = String::new();
    let mut file_paths = String::new();
    let mut paste_image = String::new();
    let mut edit_previous = String::new();
    let mut quit = String::new();
    let mut show_transcript = String::new();

    for descriptor in SHORTCUTS {
        if let Some(text) = descriptor.overlay_entry(state) {
            match descriptor.id {
                ShortcutId::Commands => commands = text,
                ShortcutId::Submit => submit = text,
                ShortcutId::InsertNewline => newline = text,
                ShortcutId::FilePaths => file_paths = text,
                ShortcutId::PasteImage => paste_image = text,
                ShortcutId::EditPrevious => edit_previous = text,
                ShortcutId::Quit => quit = text,
                ShortcutId::ShowTranscript => show_transcript = text,
            }
        }
    }

    let mut ordered = Vec::new();
    ordered.push(commands);
    if !submit.is_empty() {
        ordered.push(submit);
    }
    ordered.push(newline);
    ordered.push(file_paths);
    ordered.push(paste_image);
    ordered.push(edit_previous);
    ordered.push(quit);
    ordered.push(String::new());
    ordered.push(show_transcript);

    build_columns(ordered)
}

fn build_columns(entries: Vec<String>) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return Vec::new();
    }

    const COLUMNS: usize = 2;
    const COLUMN_PADDING: [usize; COLUMNS] = [4, 4];
    const COLUMN_GAP: usize = 4;

    let rows = entries.len().div_ceil(COLUMNS);
    let target_len = rows * COLUMNS;
    let mut entries = entries;
    if entries.len() < target_len {
        entries.extend(std::iter::repeat_n(
            String::new(),
            target_len - entries.len(),
        ));
    }

    let mut column_widths = [0usize; COLUMNS];

    for (idx, entry) in entries.iter().enumerate() {
        let column = idx % COLUMNS;
        column_widths[column] = column_widths[column].max(entry.len());
    }

    for (idx, width) in column_widths.iter_mut().enumerate() {
        *width += COLUMN_PADDING[idx];
    }

    entries
        .chunks(COLUMNS)
        .map(|chunk| {
            let mut line = String::new();
            for (col, entry) in chunk.iter().enumerate() {
                line.push_str(entry);
                if col < COLUMNS - 1 {
                    let target_width = column_widths[col];
                    let padding = target_width.saturating_sub(entry.len()) + COLUMN_GAP;
                    line.push_str(&" ".repeat(padding));
                }
            }
            let indented = indent_text(&line);
            dim_line(indented)
        })
        .collect()
}

fn indent_text(text: &str) -> String {
    let mut indented = String::with_capacity(FOOTER_INDENT_COLS + text.len());
    indented.extend(iter::repeat_n(' ', FOOTER_INDENT_COLS));
    indented.push_str(text);
    indented
}

fn dim_line(text: String) -> Line<'static> {
    Line::from(text).dim()
}

fn context_window_line(percent: Option<u8>) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(indent_text("").into());
    match percent {
        Some(percent) => {
            spans.push(format!("{percent}%").bold());
            spans.push(" context left".dim());
        }
        None => {
            spans.push("? for shortcuts".dim());
        }
    }
    Line::from(spans)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShortcutId {
    Commands,
    Submit,
    InsertNewline,
    FilePaths,
    PasteImage,
    EditPrevious,
    Quit,
    ShowTranscript,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShortcutBinding {
    code: KeyCode,
    modifiers: KeyModifiers,
    overlay_text: &'static str,
    condition: DisplayCondition,
}

impl ShortcutBinding {
    fn matches(&self, state: ShortcutsState) -> bool {
        self.condition.matches(state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayCondition {
    Always,
    WhenShiftEnterHint,
    WhenNotShiftEnterHint,
}

impl DisplayCondition {
    fn matches(self, state: ShortcutsState) -> bool {
        match self {
            DisplayCondition::Always => true,
            DisplayCondition::WhenShiftEnterHint => state.use_shift_enter_hint,
            DisplayCondition::WhenNotShiftEnterHint => !state.use_shift_enter_hint,
        }
    }
}

struct ShortcutDescriptor {
    id: ShortcutId,
    bindings: &'static [ShortcutBinding],
    prefix: &'static str,
    label: &'static str,
}

impl ShortcutDescriptor {
    fn binding_for(&self, state: ShortcutsState) -> Option<&'static ShortcutBinding> {
        self.bindings.iter().find(|binding| binding.matches(state))
    }

    fn overlay_entry(&self, state: ShortcutsState) -> Option<String> {
        // Keep legacy snapshots stable: only show the explicit "send" (Enter)
        // hint when glyphs are enabled (runtime or opted-in tests).
        if matches!(self.id, ShortcutId::Submit) && !glyphs_enabled() {
            return None;
        }
        let binding = self.binding_for(state)?;
        let label = match self.id {
            ShortcutId::EditPrevious => {
                if state.esc_backtrack_hint {
                    " again to edit previous message"
                } else {
                    " esc to edit previous message"
                }
            }
            _ => self.label,
        };
        // Prefer compact, glyph-based key hints at runtime, while keeping
        // existing text-only strings in tests to preserve snapshots.
        let key = binding_overlay_string(self.id, binding);
        let text = format!("{}{}{}", self.prefix, key, label);
        Some(text)
    }
}

// Render friendly overlay key text. In tests, keep the original strings to
// avoid churn in insta snapshots; at runtime use compact glyphs.
fn binding_overlay_string(id: ShortcutId, binding: &ShortcutBinding) -> String {
    if !glyphs_enabled() {
        return binding.overlay_text.to_string();
    }
    use crossterm::event::KeyCode::*;
    use crossterm::event::KeyModifiers as KM;
    match (id, binding.modifiers, binding.code) {
        // Send/Submit
        (ShortcutId::Submit, KM::NONE, Enter) => "⏎".to_string(),
        // Newline variants
        (ShortcutId::InsertNewline, KM::SHIFT, Enter) => "⇧⏎".to_string(),
        (ShortcutId::InsertNewline, KM::CONTROL, Char('j')) => "⌃J".to_string(),
        // Control shortcuts
        (ShortcutId::PasteImage, KM::CONTROL, Char('v')) => "⌃V".to_string(),
        (ShortcutId::Quit, KM::CONTROL, Char('c')) => "⌃C".to_string(),
        (ShortcutId::ShowTranscript, KM::CONTROL, Char('t')) => "⌃T".to_string(),
        // Pass-through for simple literal keys
        (ShortcutId::Commands, KM::NONE, Char('/')) => "/".to_string(),
        (ShortcutId::FilePaths, KM::NONE, Char('@')) => "@".to_string(),
        // Fallback to provided text
        _ => binding.overlay_text.to_string(),
    }
}

#[inline]
fn glyphs_enabled() -> bool {
    #[cfg(test)]
    {
        return std::env::var("CODEX_TUI_TEST_FORCE_GLYPHS").ok().as_deref() == Some("1");
    }
    #[cfg(not(test))]
    {
        true
    }
}

const SHORTCUTS: &[ShortcutDescriptor] = &[
    ShortcutDescriptor {
        id: ShortcutId::Commands,
        bindings: &[ShortcutBinding {
            code: KeyCode::Char('/'),
            modifiers: KeyModifiers::NONE,
            overlay_text: "/",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " for commands",
    },
    ShortcutDescriptor {
        id: ShortcutId::Submit,
        bindings: &[ShortcutBinding {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            overlay_text: "enter",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " send",
    },
    ShortcutDescriptor {
        id: ShortcutId::InsertNewline,
        bindings: &[
            ShortcutBinding {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::SHIFT,
                overlay_text: "shift + enter",
                condition: DisplayCondition::WhenShiftEnterHint,
            },
            ShortcutBinding {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::CONTROL,
                overlay_text: "ctrl + j",
                condition: DisplayCondition::WhenNotShiftEnterHint,
            },
        ],
        prefix: "",
        label: " for newline",
    },
    ShortcutDescriptor {
        id: ShortcutId::FilePaths,
        bindings: &[ShortcutBinding {
            code: KeyCode::Char('@'),
            modifiers: KeyModifiers::NONE,
            overlay_text: "@",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " for file paths",
    },
    ShortcutDescriptor {
        id: ShortcutId::PasteImage,
        bindings: &[ShortcutBinding {
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::CONTROL,
            overlay_text: "ctrl + v",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to paste images",
    },
    ShortcutDescriptor {
        id: ShortcutId::EditPrevious,
        bindings: &[ShortcutBinding {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            overlay_text: "esc",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: "",
    },
    ShortcutDescriptor {
        id: ShortcutId::Quit,
        bindings: &[ShortcutBinding {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            overlay_text: "ctrl + c",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to exit",
    },
    ShortcutDescriptor {
        id: ShortcutId::ShowTranscript,
        bindings: &[ShortcutBinding {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::CONTROL,
            overlay_text: "ctrl + t",
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to view transcript",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn snapshot_footer(name: &str, props: FooterProps) {
        let height = footer_height(props).max(1);
        let mut terminal = Terminal::new(TestBackend::new(80, height)).unwrap();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, f.area().width, height);
                render_footer(area, f.buffer_mut(), props);
            })
            .unwrap();
        assert_snapshot!(name, terminal.backend());
    }

    #[test]
    fn footer_snapshots() {
        snapshot_footer(
            "footer_shortcuts_default",
            FooterProps {
                mode: FooterMode::ShortcutPrompt,
                esc_backtrack_hint: false,
                use_shift_enter_hint: false,
                is_task_running: false,
                context_window_percent: None,
            },
        );

        snapshot_footer(
            "footer_shortcuts_shift_and_esc",
            FooterProps {
                mode: FooterMode::ShortcutOverlay,
                esc_backtrack_hint: true,
                use_shift_enter_hint: true,
                is_task_running: false,
                context_window_percent: None,
            },
        );

        snapshot_footer(
            "footer_ctrl_c_quit_idle",
            FooterProps {
                mode: FooterMode::CtrlCReminder,
                esc_backtrack_hint: false,
                use_shift_enter_hint: false,
                is_task_running: false,
                context_window_percent: None,
            },
        );

        snapshot_footer(
            "footer_ctrl_c_quit_running",
            FooterProps {
                mode: FooterMode::CtrlCReminder,
                esc_backtrack_hint: false,
                use_shift_enter_hint: false,
                is_task_running: true,
                context_window_percent: None,
            },
        );

        snapshot_footer(
            "footer_esc_hint_idle",
            FooterProps {
                mode: FooterMode::EscHint,
                esc_backtrack_hint: false,
                use_shift_enter_hint: false,
                is_task_running: false,
                context_window_percent: None,
            },
        );

        snapshot_footer(
            "footer_esc_hint_primed",
            FooterProps {
                mode: FooterMode::EscHint,
                esc_backtrack_hint: true,
                use_shift_enter_hint: false,
                is_task_running: false,
                context_window_percent: None,
            },
        );

        snapshot_footer(
            "footer_shortcuts_context_running",
            FooterProps {
                mode: FooterMode::ShortcutPrompt,
                esc_backtrack_hint: false,
                use_shift_enter_hint: false,
                is_task_running: true,
                context_window_percent: Some(72),
            },
        );
    }
}
