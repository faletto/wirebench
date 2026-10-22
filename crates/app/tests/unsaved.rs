//! Unsaved-changes tracking, autosave, recovery and Ctrl+S, headless with explicit clocks.

use app::autosave::{now_unix_secs, Autosave, FileStore, MemoryStore, Store};
use app::widget::TermAction;
use app::{App, Choice, Pending};
use egui::{
    Context, Event, Key, Modifiers, Pos2, RawInput, ViewportCommand, ViewportEvent, ViewportId,
    ViewportInfo,
};
use engine::lab::LabKind;
use std::path::PathBuf;
use std::time::Duration;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("wirebench-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_string()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Out {
    repaint: Duration,
    commands: Vec<ViewportCommand>,
}

fn frame_with(ctx: &Context, app: &mut App, t: f64, events: Vec<Event>, close: bool) -> Out {
    let mut raw = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1100.0, 700.0),
        )),
        time: Some(t),
        events,
        ..Default::default()
    };
    if close {
        let info = ViewportInfo {
            events: vec![ViewportEvent::Close],
            ..Default::default()
        };
        raw.viewports.insert(ViewportId::ROOT, info);
    }
    let mut out = ctx.run_ui(raw, |ui| app.ui(ui));
    out.textures_delta.clear();
    let v = &out.viewport_output[&ViewportId::ROOT];
    Out {
        repaint: v.repaint_delay,
        commands: v.commands.clone(),
    }
}

fn frame(ctx: &Context, app: &mut App, t: f64) -> Out {
    frame_with(ctx, app, t, vec![], false)
}

fn ctrl(key: Key) -> Vec<Event> {
    let k = |pressed| Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    };
    vec![k(true), k(false)]
}

fn run(app: &mut App, lines: &[&str]) {
    for l in lines {
        app.apply(0, TermAction::Submit((*l).to_string())); // tab 0 is r1
    }
}

fn ready() -> (App, Context) {
    let mut app = App::new();
    let ctx = Context::default();
    frame(&ctx, &mut app, 0.0);
    frame(&ctx, &mut app, 0.01);
    (app, ctx)
}

fn titles(out: &Out) -> Vec<String> {
    out.commands
        .iter()
        .filter_map(|c| match c {
            ViewportCommand::Title(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

// ---- unsaved changes ----

#[test]
fn only_real_changes_make_the_lab_dirty() {
    let (mut app, _ctx) = ready();
    assert!(!app.is_dirty());
    // Running commands that change nothing is not an edit.
    run(
        &mut app,
        &["enable", "show running-config", "show ip route"],
    );
    app.sim_queue_ping("pc1", [10, 0, 1, 1]);
    app.sim_run();
    assert!(
        !app.is_dirty(),
        "show commands and traffic leave the lab clean"
    );
    // A config change is.
    run(&mut app, &["configure terminal", "hostname edge"]);
    assert!(app.is_dirty());
    // Changing it back is clean again.
    run(&mut app, &["hostname r1"]);
    assert!(!app.is_dirty());
    // Structural edits are.
    app.add_device(LabKind::Host, [10, 10]);
    assert!(app.is_dirty());
}

#[test]
fn every_kind_of_edit_dirties_the_lab() {
    for edit in 0..4 {
        let (mut app, _ctx) = ready();
        match edit {
            0 => app.add_device(LabKind::Switch, [0, 0]),
            1 => app.move_node(0, [999, 999]),
            2 => {
                app.add_device(LabKind::Switch, [0, 0]);
                app.add_device(LabKind::Host, [0, 0]);
                app.connect(4, 5);
            }
            _ => app.delete_device(3),
        }
        assert!(app.is_dirty(), "edit {edit}");
    }
}

#[test]
fn saving_cleans_the_lab_and_the_title_follows() {
    let dir = Dir::new("title");
    let mut app = App::new();
    let ctx = Context::default();
    let first = frame(&ctx, &mut app, 1.0);
    assert!(
        titles(&first)
            .iter()
            .any(|t| t == "Two routers, static routes — wirebench"),
        "{:?}",
        titles(&first)
    );
    app.add_device(LabKind::Host, [0, 0]);
    let dirty = frame(&ctx, &mut app, 2.0);
    assert_eq!(titles(&dirty), ["● Two routers, static routes — wirebench"]);
    assert!(app.title_text().starts_with("● "));
    assert!(app.save_to_path(&dir.file("a.lab.toml")));
    assert!(!app.is_dirty());
    let clean = frame(&ctx, &mut app, 3.0);
    assert_eq!(titles(&clean), ["Two routers, static routes — wirebench"]);
    // An unchanged title is not re-sent every frame.
    assert!(titles(&frame(&ctx, &mut app, 4.0)).is_empty());
}

// ---- Ctrl+S / Ctrl+O ----

#[test]
fn ctrl_s_saves_to_the_current_file() {
    let dir = Dir::new("ctrls");
    let path = dir.file("mine.lab.toml");
    let (mut app, ctx) = ready();
    app.set_file_path(&path);
    app.add_device(LabKind::Host, [5, 5]);
    assert!(app.is_dirty());
    frame_with(&ctx, &mut app, 1.0, ctrl(Key::S), false);
    assert!(!app.is_dirty());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("name = \"pc3\""), "{text}");
    assert_eq!(app.status_notice(), Some(format!("saved {path}").as_str()));
}

#[test]
fn ctrl_s_does_not_reach_the_terminal() {
    let dir = Dir::new("ctrls2");
    let (mut app, ctx) = ready();
    app.set_file_path(&dir.file("x.lab.toml"));
    frame_with(&ctx, &mut app, 1.0, ctrl(Key::S), false);
    // The key combination saved; it did not type or run anything in the console.
    assert!(app
        .active_scrollback()
        .iter()
        .all(|l| !l.contains('s') || l.contains("type ? for help")));
}

#[test]
fn ctrl_o_asks_first_when_dirty_and_opens_when_clean() {
    let dir = Dir::new("ctrlo");
    let (mut app, ctx) = ready();
    // Clean: opens straight away (and fails politely, since no file name is set).
    frame_with(&ctx, &mut app, 1.0, ctrl(Key::O), false);
    assert_eq!(app.pending_confirm(), None);
    assert!(
        app.status_notice().unwrap().contains("file name"),
        "{:?}",
        app.status_notice()
    );
    // Dirty: asks.
    app.set_file_path(&dir.file("other.lab.toml"));
    app.add_device(LabKind::Host, [0, 0]);
    frame_with(&ctx, &mut app, 2.0, ctrl(Key::O), false);
    assert_eq!(app.pending_confirm(), Some(Pending::Open));
}

#[test]
fn shortcuts_are_ignored_while_a_question_is_open() {
    let dir = Dir::new("ctrlq");
    let path = dir.file("q.lab.toml");
    let (mut app, ctx) = ready();
    app.set_file_path(&path);
    app.add_device(LabKind::Host, [0, 0]);
    app.request_open();
    assert_eq!(app.pending_confirm(), Some(Pending::Open));
    frame_with(&ctx, &mut app, 1.0, ctrl(Key::S), false);
    assert!(
        !std::path::Path::new(&path).exists(),
        "no save behind the dialog"
    );
}

// ---- the question ----

fn two_labs(dir: &Dir) -> (String, String) {
    let (a, b) = (dir.file("a.lab.toml"), dir.file("b.lab.toml"));
    let mut seed = App::new();
    assert!(seed.save_to_path(&a));
    let small = "format = \"labsim-lab\"\nversion = 1\n[meta]\ntitle = \"other\"\n[[device]]\nname = \"solo\"\ntemplate = \"host-pc\"\npos = [0, 0]\n";
    std::fs::write(&b, small).unwrap();
    (a, b)
}

#[test]
fn open_with_unsaved_changes_cancel_discard_and_save() {
    let dir = Dir::new("openq");
    let (a, b) = two_labs(&dir);

    // Cancel: nothing happens.
    let (mut app, _ctx) = ready();
    app.set_file_path(&b);
    app.add_device(LabKind::Host, [0, 0]);
    app.request_open();
    app.confirm(Choice::Cancel);
    assert_eq!(app.pending_confirm(), None);
    assert!(app.is_dirty());
    assert_eq!(app.device_names().len(), 5);

    // Discard: the other lab opens and the lab is clean.
    app.request_open();
    app.confirm(Choice::Discard);
    assert_eq!(app.device_names(), ["solo"]);
    assert!(!app.is_dirty());

    // Save: current lab is written to its file, then the other opens.
    let (mut app, _ctx) = ready();
    app.set_file_path(&a);
    app.add_device(LabKind::Host, [0, 0]);
    app.request_open();
    app.confirm(Choice::Save);
    assert!(
        std::fs::read_to_string(&a).unwrap().contains("pc3"),
        "saved before leaving"
    );
    // (The file just saved is the one reopened, so the lab is back, clean.)
    assert_eq!(app.device_names().len(), 5);
    assert!(!app.is_dirty());
}

#[test]
fn a_failed_open_after_discard_does_not_lose_the_changes() {
    let dir = Dir::new("openfail");
    let (mut app, _ctx) = ready();
    app.set_file_path(&dir.file("missing.lab.toml"));
    app.add_device(LabKind::Host, [0, 0]);
    app.request_open();
    app.confirm(Choice::Discard);
    assert!(
        app.is_dirty(),
        "the open failed, so nothing was actually discarded"
    );
    assert_eq!(app.device_names().len(), 5);
}

#[test]
fn a_failed_save_does_not_continue_to_open() {
    let (mut app, _ctx) = ready();
    app.set_file_path("/definitely/not/a/dir/x.lab.toml");
    app.add_device(LabKind::Host, [0, 0]);
    app.request_open();
    app.confirm(Choice::Save);
    assert!(app.is_dirty());
    assert!(app.status_notice().unwrap().starts_with("could not save"));
}

#[test]
fn closing_the_window_asks_when_dirty_and_just_closes_when_clean() {
    let (mut app, ctx) = ready();
    let clean = frame_with(&ctx, &mut app, 1.0, vec![], true);
    assert!(
        !clean.commands.contains(&ViewportCommand::CancelClose),
        "clean: let it close"
    );
    assert_eq!(app.pending_confirm(), None);

    app.add_device(LabKind::Host, [0, 0]);
    let dirty = frame_with(&ctx, &mut app, 2.0, vec![], true);
    assert!(dirty.commands.contains(&ViewportCommand::CancelClose));
    assert_eq!(app.pending_confirm(), Some(Pending::Close));

    // Cancel keeps the window; asking again asks again.
    app.confirm(Choice::Cancel);
    frame_with(&ctx, &mut app, 3.0, vec![], true);
    assert_eq!(app.pending_confirm(), Some(Pending::Close));

    // Discard closes for real, and the next close request is not cancelled.
    app.confirm(Choice::Discard);
    let next = frame(&ctx, &mut app, 4.0);
    assert!(
        next.commands.contains(&ViewportCommand::Close),
        "{:?}",
        next.commands
    );
    let again = frame_with(&ctx, &mut app, 5.0, vec![], true);
    assert!(!again.commands.contains(&ViewportCommand::CancelClose));
}

#[test]
fn save_then_close_writes_the_file_first() {
    let dir = Dir::new("closesave");
    let path = dir.file("c.lab.toml");
    let (mut app, ctx) = ready();
    app.set_file_path(&path);
    app.add_device(LabKind::Host, [0, 0]);
    frame_with(&ctx, &mut app, 1.0, vec![], true);
    app.confirm(Choice::Save);
    assert!(std::fs::read_to_string(&path).unwrap().contains("pc3"));
    assert!(frame(&ctx, &mut app, 2.0)
        .commands
        .contains(&ViewportCommand::Close));
}

#[test]
fn typing_is_not_run_as_commands_while_the_question_is_open() {
    let (mut app, ctx) = ready();
    app.add_device(LabKind::Host, [0, 0]);
    app.request_open();
    let before = app.active_scrollback().len();
    let enter = |pressed| Event::Key {
        key: Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    frame_with(
        &ctx,
        &mut app,
        1.0,
        vec![Event::Text("enable".into())],
        false,
    );
    frame_with(&ctx, &mut app, 1.1, vec![enter(true), enter(false)], false);
    assert_eq!(
        app.active_scrollback().len(),
        before,
        "nothing ran behind the dialog"
    );
}

// ---- autosave ----

fn with_store() -> (App, Context, MemoryStore) {
    let (mut app, ctx) = ready();
    let store = MemoryStore::default();
    app.set_store(Box::new(store.clone()));
    (app, ctx, store)
}

#[test]
fn autosave_fires_after_the_delay_and_only_once_per_change() {
    let (mut app, ctx, store) = with_store();
    frame(&ctx, &mut app, 10.0);
    app.add_device(LabKind::Host, [0, 0]);
    frame(&ctx, &mut app, 11.0); // this frame also sends the new title, which repaints once
    frame(&ctx, &mut app, 11.05); // and egui reports a request one frame late
    let w = frame(&ctx, &mut app, 11.1);
    assert!(store.load().is_none(), "too soon");
    assert!(
        w.repaint > Duration::from_millis(1500) && w.repaint < Duration::from_millis(2500),
        "wakes for the deadline: {:?}",
        w.repaint
    );
    frame(&ctx, &mut app, 13.2);
    let saved = store.load().expect("autosaved");
    assert!(saved.text.contains("name = \"pc3\""));
    assert!(saved.unix_secs > 0);
    // Nothing changed since: no rewrite, and the app goes back to idle.
    *store.0.borrow_mut() = Some(Autosave {
        text: "sentinel".into(),
        unix_secs: 1,
    });
    frame(&ctx, &mut app, 20.0);
    assert_eq!(
        store.load().unwrap().text,
        "sentinel",
        "not rewritten without a change"
    );
    frame(&ctx, &mut app, 21.0);
    assert_eq!(
        frame(&ctx, &mut app, 22.0).repaint,
        Duration::MAX,
        "idle again"
    );
}

#[test]
fn a_flurry_of_edits_waits_for_a_pause() {
    let (mut app, ctx, store) = with_store();
    frame(&ctx, &mut app, 10.0);
    for i in 0..10 {
        app.move_node(0, [i, i]);
        frame(&ctx, &mut app, 10.0 + f64::from(i) * 0.5); // keeps editing every half second
    }
    assert!(store.load().is_none(), "still being edited");
    frame(&ctx, &mut app, 20.0);
    assert!(store.load().is_some(), "paused for 3 s+");
}

#[test]
fn saving_removes_the_autosave() {
    let dir = Dir::new("saveclear");
    let (mut app, ctx, store) = with_store();
    frame(&ctx, &mut app, 1.0);
    app.add_device(LabKind::Host, [0, 0]);
    frame(&ctx, &mut app, 5.0);
    assert!(store.load().is_some());
    assert!(app.save_to_path(&dir.file("s.lab.toml")));
    assert!(
        store.load().is_none(),
        "the saved file supersedes the autosave"
    );
    // And nothing is autosaved for a clean lab.
    frame(&ctx, &mut app, 30.0);
    assert!(store.load().is_none());
}

#[test]
fn a_clean_lab_never_autosaves() {
    let (mut app, ctx, store) = with_store();
    run(&mut app, &["enable", "show ip route"]);
    for t in [1.0, 10.0, 60.0] {
        frame(&ctx, &mut app, t);
    }
    assert!(store.load().is_none());
}

// ---- recovery ----

fn modified_lab_text() -> String {
    let mut other = App::new();
    other.add_device(LabKind::Host, [1, 2]);
    other.export_lab()
}

#[test]
fn recovery_is_offered_and_restoring_keeps_it_unsaved() {
    let dir = Dir::new("restore");
    let store = MemoryStore::default();
    store.0.replace(Some(Autosave {
        text: modified_lab_text(),
        unix_secs: now_unix_secs() - 300,
    }));
    let (mut app, ctx) = ready();
    app.set_store(Box::new(store.clone()));
    assert_eq!(app.recovery_age().as_deref(), Some("5 min ago"));
    frame(&ctx, &mut app, 1.0); // the banner renders
    app.restore_recovered();
    assert_eq!(app.device_names().len(), 5, "pc3 is back");
    assert!(app.is_dirty(), "recovered work exists nowhere else yet");
    assert_eq!(app.recovery_age(), None, "banner gone");
    assert!(
        store.load().is_some(),
        "the autosave is still the only copy"
    );
    // Saving for real finally releases it.
    assert!(app.save_to_path(&dir.file("r.lab.toml")));
    assert!(!app.is_dirty());
    assert!(store.load().is_none());
}

#[test]
fn discarding_recovery_deletes_the_autosave() {
    let store = MemoryStore::default();
    store.0.replace(Some(Autosave {
        text: modified_lab_text(),
        unix_secs: now_unix_secs(),
    }));
    let (mut app, _ctx) = ready();
    app.set_store(Box::new(store.clone()));
    assert!(app.recovery_age().is_some());
    app.discard_recovered();
    assert_eq!(app.recovery_age(), None);
    assert!(store.load().is_none());
    assert_eq!(app.device_names().len(), 4, "the lab is untouched");
}

#[test]
fn useless_autosaves_are_not_offered() {
    let (mut app, _ctx) = ready();
    // Garbage that is not a lab.
    let junk = MemoryStore::default();
    junk.0.replace(Some(Autosave {
        text: "not [a lab".into(),
        unix_secs: 1,
    }));
    app.set_store(Box::new(junk));
    assert_eq!(app.recovery_age(), None);
    // Identical to what is already open.
    let same = MemoryStore::default();
    same.0.replace(Some(Autosave {
        text: app.export_lab(),
        unix_secs: 1,
    }));
    app.set_store(Box::new(same));
    assert_eq!(app.recovery_age(), None);
}

#[test]
fn restoring_over_unsaved_changes_asks_first() {
    let store = MemoryStore::default();
    store.0.replace(Some(Autosave {
        text: modified_lab_text(),
        unix_secs: now_unix_secs(),
    }));
    let (mut app, _ctx) = ready();
    app.set_store(Box::new(store));
    app.move_node(0, [7, 7]);
    app.restore_recovered();
    assert_eq!(app.pending_confirm(), Some(Pending::Restore));
    app.confirm(Choice::Cancel);
    assert_eq!(app.device_names().len(), 4, "nothing replaced");
    app.restore_recovered();
    app.confirm(Choice::Discard);
    assert_eq!(app.device_names().len(), 5);
}

#[test]
fn waiting_recovery_is_never_overwritten_by_new_autosaves() {
    let store = MemoryStore::default();
    let old = Autosave {
        text: modified_lab_text(),
        unix_secs: now_unix_secs() - 60,
    };
    store.0.replace(Some(old.clone()));
    let (mut app, ctx) = ready();
    app.set_store(Box::new(store.clone()));
    frame(&ctx, &mut app, 10.0);
    app.move_node(0, [3, 3]);
    frame(&ctx, &mut app, 30.0);
    assert_eq!(
        store.load(),
        Some(old),
        "the recoverable work is still there to choose"
    );
}

#[test]
fn autosave_and_recovery_work_end_to_end_through_real_files() {
    let dir = Dir::new("e2e");
    let store_dir = PathBuf::from(dir.file("data"));
    {
        let (mut app, ctx) = ready();
        app.set_store(Box::new(FileStore::new(store_dir.clone())));
        frame(&ctx, &mut app, 1.0);
        run(
            &mut app,
            &[
                "enable",
                "configure terminal",
                "ip route 172.16.0.0 255.255.0.0 10.0.12.2",
            ],
        );
        frame(&ctx, &mut app, 2.0);
        frame(&ctx, &mut app, 6.0);
        // The app "crashes" here: dropped with unsaved work and an autosave on disk.
    }
    assert!(store_dir.join("autosave.lab.toml").exists());
    let (mut app2, _ctx) = ready();
    app2.set_store(Box::new(FileStore::new(store_dir)));
    assert!(app2.recovery_age().is_some(), "the next run offers it back");
    app2.restore_recovered();
    assert!(app2
        .export_lab()
        .contains("ip route 172.16.0.0 255.255.0.0 10.0.12.2"));
    assert!(app2.is_dirty());
}

#[test]
fn the_default_app_never_touches_a_real_data_directory() {
    // App::new() uses a null store: unsaved work vanishes rather than being written anywhere.
    let (mut app, ctx) = ready();
    app.add_device(LabKind::Host, [0, 0]);
    frame(&ctx, &mut app, 50.0);
    assert_eq!(app.recovery_age(), None);
}
