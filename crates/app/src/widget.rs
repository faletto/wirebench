//! The egui terminal widget: scrollback, an input line with a cursor, and key handling.

use crate::term::TermModel;
use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Color32, Event, EventFilter, FontId, IMEPurpose, Id, Key, Modifiers, Role, ScrollArea, Sense,
    TextStyle, Ui, WidgetInfo,
};

/// What the widget wants the host app to do. Actions come out in the order typed, so a
/// multi-line paste becomes a sequence of `Submit`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermAction {
    /// Enter pressed: run this line.
    Submit(String),
    /// `?` typed: show help for the input so far (the `?` itself is not inserted).
    Help(String),
    /// Tab pressed: complete this line.
    Tab(String),
}

/// Translate input events into model edits and actions. Separate from drawing so it can be
/// tested with synthetic events.
pub fn handle_events(model: &mut TermModel, events: &[Event]) -> Vec<TermAction> {
    let mut actions = Vec::new();
    for ev in events {
        match ev {
            Event::Text(s) => {
                for ch in s.chars() {
                    if ch == '?' {
                        actions.push(TermAction::Help(model.input()));
                    } else {
                        model.insert(ch.encode_utf8(&mut [0; 4]));
                    }
                }
            }
            Event::Paste(s) => {
                for line in model.paste(s) {
                    actions.push(TermAction::Submit(line));
                }
            }
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) => model.preedit = text.clone(),
            Event::Ime(egui::ImeEvent::Commit(s)) => {
                model.preedit.clear();
                model.insert(s);
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if let Some(a) = handle_key(model, *key, *modifiers) {
                    actions.push(a);
                }
            }
            _ => {}
        }
    }
    actions
}

fn handle_key(model: &mut TermModel, key: Key, m: Modifiers) -> Option<TermAction> {
    match (key, m.ctrl || m.command) {
        (Key::Enter, _) => return Some(TermAction::Submit(model.take_line())),
        (Key::Tab, _) => return Some(TermAction::Tab(model.input())),
        (Key::Backspace, _) => model.backspace(),
        (Key::Delete, _) => model.delete(),
        (Key::ArrowLeft, _) => model.left(),
        (Key::ArrowRight, _) => model.right(),
        (Key::ArrowUp, _) => model.history_prev(),
        (Key::ArrowDown, _) => model.history_next(),
        (Key::Home, _) | (Key::A, true) => model.home(),
        (Key::End, _) | (Key::E, true) => model.end(),
        (Key::U, true) => model.kill_to_start(),
        (Key::L, true) => model.scrollback.clear(),
        (Key::Escape, _) => model.clear_line(),
        _ => {}
    }
    None
}

/// Draw the terminal and return actions for the host to run. `prompt` is shown before the
/// input line. Pass `request_focus` when the terminal becomes the active one.
pub fn show(
    ui: &mut Ui,
    id: Id,
    model: &mut TermModel,
    prompt: &str,
    request_focus: bool,
) -> Vec<TermAction> {
    let font = TextStyle::Monospace.resolve(ui.style());
    let row_h = ui.fonts_mut(|f| f.row_height(&font));
    let fg = ui.visuals().text_color();
    let dim = ui.visuals().weak_text_color();
    let rows = model.scrollback.len() + 1;

    let area = ScrollArea::vertical()
        .id_salt(id.with("scroll"))
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, rows, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut ime = None;
            for i in range {
                if let Some(line) = model.scrollback.get(i) {
                    ui.label(egui::RichText::new(line).font(font.clone()).color(fg));
                } else {
                    ime = Some(draw_input_row(ui, model, prompt, &font, fg, dim));
                }
            }
            ime
        });

    // The whole area is the focus target, so clicking anywhere in the terminal types into it.
    let resp = ui.interact(area.inner_rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, "Terminal input"));
    if resp.clicked() || request_focus {
        resp.request_focus();
    }
    if !ui.memory(|m| m.has_focus(id)) {
        return Vec::new();
    }
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        )
    });
    if let Some((rect, cursor_rect)) = area.inner {
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: IMEPurpose::Terminal,
                rect,
                cursor_rect,
                should_interrupt_composition: false,
            })
        });
    }
    let events = ui.input(|i| i.events.clone());
    handle_events(model, &events)
}

/// Draws `prompt + input` with the cursor as an inverted block; returns the input row's rect
/// and the cursor rect (for positioning the IME popup).
fn draw_input_row(
    ui: &mut Ui,
    model: &TermModel,
    prompt: &str,
    font: &FontId,
    fg: Color32,
    dim: Color32,
) -> (egui::Rect, egui::Rect) {
    let input: Vec<char> = model.input().chars().collect();
    let cursor = model.cursor().min(input.len());
    let before: String = input[..cursor].iter().collect();
    let under = input.get(cursor).copied().unwrap_or(' ');
    let after: String = input.iter().skip(cursor + 1).collect();

    let plain = |c| TextFormat {
        font_id: font.clone(),
        color: c,
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    job.append(prompt, 0.0, plain(dim));
    job.append(&before, 0.0, plain(fg));
    if !model.preedit.is_empty() {
        let mut f = plain(fg);
        f.underline = egui::Stroke::new(1.0, fg);
        job.append(&model.preedit, 0.0, f);
    }
    let mut block = plain(ui.visuals().extreme_bg_color);
    block.background = fg;
    job.append(&under.to_string(), 0.0, block);
    job.append(&after, 0.0, plain(fg));

    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    let pos = prompt.chars().count() + cursor + model.preedit.chars().count();
    let cursor_rect = galley
        .pos_from_cursor(CCursor::new(pos))
        .translate(rect.min.to_vec2());
    ui.painter().galley(rect.min, galley, fg);
    (rect, cursor_rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key) -> Event {
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn ctrl(k: Key) -> Event {
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }
    }

    #[test]
    fn typing_and_enter_submit_the_line() {
        let mut m = TermModel::default();
        let a = handle_events(&mut m, &[Event::Text("conf t".into()), key(Key::Enter)]);
        assert_eq!(a, vec![TermAction::Submit("conf t".into())]);
        assert_eq!(m.input(), "");
    }

    #[test]
    fn question_mark_asks_for_help_without_inserting() {
        let mut m = TermModel::default();
        let a = handle_events(&mut m, &[Event::Text("show ip?".into())]);
        assert_eq!(a, vec![TermAction::Help("show ip".into())]);
        assert_eq!(m.input(), "show ip");
    }

    #[test]
    fn tab_requests_completion_of_the_current_input() {
        let mut m = TermModel::default();
        let a = handle_events(&mut m, &[Event::Text("sh".into()), key(Key::Tab)]);
        assert_eq!(a, vec![TermAction::Tab("sh".into())]);
        assert_eq!(
            m.input(),
            "sh",
            "model is updated by the host from the engine's answer"
        );
    }

    #[test]
    fn multi_line_paste_becomes_ordered_submits() {
        let mut m = TermModel::default();
        let a = handle_events(&mut m, &[Event::Paste("enable\nconf t\nhost".into())]);
        assert_eq!(
            a,
            vec![
                TermAction::Submit("enable".into()),
                TermAction::Submit("conf t".into())
            ]
        );
        assert_eq!(m.input(), "host");
    }

    #[test]
    fn ime_commit_inserts_and_clears_preedit() {
        let mut m = TermModel::default();
        handle_events(
            &mut m,
            &[Event::Ime(egui::ImeEvent::Preedit {
                text: "にほ".into(),
                active_range_chars: None,
            })],
        );
        assert_eq!(m.preedit, "にほ");
        handle_events(&mut m, &[Event::Ime(egui::ImeEvent::Commit("日本".into()))]);
        assert_eq!((m.preedit.as_str(), m.input().as_str()), ("", "日本"));
    }

    #[test]
    fn key_repeat_events_edit_like_presses() {
        let mut m = TermModel::default();
        m.insert("abcd");
        let rep = Event::Key {
            key: Key::Backspace,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: Modifiers::NONE,
        };
        handle_events(&mut m, &[rep.clone(), rep]);
        assert_eq!(m.input(), "ab");
    }

    #[test]
    fn readline_shortcuts() {
        let mut m = TermModel::default();
        m.insert("hello");
        handle_events(
            &mut m,
            &[
                ctrl(Key::A),
                Event::Text("X".into()),
                ctrl(Key::E),
                Event::Text("Y".into()),
            ],
        );
        assert_eq!(m.input(), "XhelloY");
        handle_events(&mut m, &[key(Key::Escape)]);
        assert_eq!(m.input(), "");
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut m = TermModel::default();
        m.insert("a");
        let up = Event::Key {
            key: Key::Backspace,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        handle_events(&mut m, &[up]);
        assert_eq!(m.input(), "a");
    }
}
