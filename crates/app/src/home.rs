//! The home screen: start a new lab, open a file, carry on with the last session, or try one of
//! the example labs. A child of `app` so it can reach the app's fields.

use super::*;
use crate::autosave::Autosave;
use crate::canvas::{draw_device_icon, kind_colour};
use crate::chrome::card;
use crate::icons::{self, Icon};
use egui::{Align2, FontId, RichText, Vec2};

/// One of the labs that ship with the app.
pub struct Example {
    pub title: &'static str,
    pub blurb: &'static str,
    pub text: &'static str,
    /// What is in it, for the little row of symbols on its card.
    pub kinds: &'static [LabKind],
}

pub const EXAMPLES: &[Example] = &[
    Example {
        title: "Two routers, static routes",
        blurb: "Two LANs joined by two routers that know each other's networks from static routes.",
        text: DEFAULT_LAB,
        kinds: &[LabKind::Host, LabKind::Router, LabKind::Router, LabKind::Host],
    },
    Example {
        title: "Two VLANs across a trunk",
        blurb: "Two switches carry two VLANs over one trunk; each PC reaches only its own VLAN.",
        text: include_str!("../../../labs/examples/vlans-trunk.lab.toml"),
        kinds: &[LabKind::Host, LabKind::Switch, LabKind::Switch, LabKind::Host],
    },
    Example {
        title: "Router on a stick",
        blurb: "One router cable and a subinterface per VLAN route between two VLANs.",
        text: include_str!("../../../labs/examples/router-on-a-stick.lab.toml"),
        kinds: &[LabKind::Host, LabKind::Switch, LabKind::Router],
    },
    Example {
        title: "DHCP with a relay",
        blurb: "A router gives out addresses on its own LAN and, through a relay, on a LAN further away.",
        text: include_str!("../../../labs/examples/dhcp-relay.lab.toml"),
        kinds: &[LabKind::Host, LabKind::Switch, LabKind::Router, LabKind::Router],
    },
    Example {
        title: "Three switches and spanning tree",
        blurb: "A loop of switches; rapid spanning tree blocks one port so frames cannot circle.",
        text: include_str!("../../../labs/examples/spanning-tree.lab.toml"),
        kinds: &[LabKind::Switch, LabKind::Switch, LabKind::Switch],
    },
];

/// A lab with nothing in it but its title.
pub fn blank_lab(title: &str) -> String {
    let mut lab = labfile::load("format = \"labsim-lab\"\nversion = 1\n")
        .expect("the empty lab loads")
        .lab;
    let title = title.trim();
    lab.meta.title = if title.is_empty() {
        "Untitled lab".to_string()
    } else {
        title.to_string()
    };
    labfile::save(&lab)
}

/// The title stored in a lab's text, if it has one.
fn title_of(text: &str) -> Option<String> {
    let t = labfile::load(text).ok()?.lab.meta.title;
    (!t.trim().is_empty()).then_some(t)
}

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Workspace,
}

/// What "Continue" on the home screen would carry on with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// Unsaved work from a session that ended without saving.
    Recovered { title: String, age: String },
    /// The lab that was open last time (it was saved, or never changed).
    Last { title: String, age: String },
}

impl App {
    /// Show the home screen. The lab that is open stays open behind it.
    pub fn show_home(&mut self) {
        self.screen = Screen::Home;
        self.close_device_window();
    }

    /// Start at the home screen, with nothing to go back to: the lab behind it is only the
    /// bundled one the engine starts with.
    pub fn start_at_home(&mut self) {
        self.show_home();
        self.entered = false;
    }

    pub fn on_home(&self) -> bool {
        self.screen == Screen::Home
    }

    /// Go (back) to the lab that is open.
    pub fn show_workspace(&mut self) {
        self.screen = Screen::Workspace;
        self.entered = true;
    }

    /// Titles of the example labs, in the order the home screen shows them.
    pub fn example_titles() -> Vec<&'static str> {
        EXAMPLES.iter().map(|e| e.title).collect()
    }

    /// Start an empty lab called `title` (asks first if the open one has unsaved changes).
    pub fn new_lab(&mut self, title: &str) {
        let name = if title.trim().is_empty() {
            "a new lab".to_string()
        } else {
            format!("\"{}\"", title.trim())
        };
        self.replace_lab(blank_lab(title), name);
    }

    /// Open example `index` (asks first if the open lab has unsaved changes).
    pub fn open_example(&mut self, index: usize) {
        if let Some(e) = EXAMPLES.get(index) {
            self.replace_lab(e.text.to_string(), format!("the example \"{}\"", e.title));
        }
    }

    fn replace_lab(&mut self, text: String, source: String) {
        self.replace_with = Some((text, source));
        if self.is_dirty() {
            self.confirm = Some(Pending::Replace);
        } else {
            self.do_replace();
        }
    }

    pub(super) fn do_replace(&mut self) {
        if let Some((text, source)) = self.replace_with.take() {
            self.open_lab(&text, &source, Then::Fresh);
        }
    }

    /// What "Continue" would open, if anything.
    pub fn resume(&self) -> Option<Resume> {
        let now = now_unix_secs();
        let title = |a: &Autosave| title_of(&a.text).unwrap_or_else(|| "Untitled lab".into());
        if let Some(r) = &self.recovery {
            return Some(Resume::Recovered {
                title: title(r),
                age: age_text(r.unix_secs, now),
            });
        }
        self.last_session.as_ref().map(|a| Resume::Last {
            title: title(a),
            age: age_text(a.unix_secs, now),
        })
    }

    /// Carry on with the unsaved work, or else the lab that was open last time.
    pub fn continue_session(&mut self) {
        if self.recovery.is_some() {
            self.restore_recovered();
        } else if let Some(a) = self.last_session.clone() {
            self.replace_lab(a.text, "your last session".into());
        }
    }

    /// Note the lab as it is now as the one to continue with next time.
    pub(super) fn remember_session(&mut self) {
        let a = Autosave {
            text: self.export_lab(),
            unix_secs: now_unix_secs(),
        };
        if self.store.remember_session(&a).is_ok() {
            self.last_session = Some(a);
        }
    }

    pub(super) fn home_screen(&mut self, ui: &mut Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let width = ui.available_width().min(980.0);
                    let pad = ((ui.available_width() - width) / 2.0).max(0.0);
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            self.home_contents(ui, width);
                        });
                    });
                });
        });
    }

    fn home_contents(&mut self, ui: &mut Ui, width: f32) {
        ui.add_space(28.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(56.0, 44.0), egui::Sense::hover());
            draw_device_icon(
                ui.painter(),
                r,
                LabKind::Router,
                kind_colour(LabKind::Router),
                egui::Stroke::NONE,
            );
            ui.vertical(|ui| {
                ui.label(RichText::new("wirebench").size(34.0).strong());
                ui.label(
                    RichText::new(
                        "Build a network, cable it up, and configure every device from its console.",
                    )
                    .size(15.0)
                    .weak(),
                );
            });
        });
        ui.add_space(22.0);
        if let Some(n) = &self.notice {
            if n.starts_with("could not") {
                ui.colored_label(ui.visuals().warn_fg_color, n);
                ui.add_space(8.0);
            }
        }
        if self.entered {
            let back = format!("Back to \"{}\"", self.title);
            if ui
                .add(
                    egui::Button::new(RichText::new(back).size(15.0))
                        .min_size(Vec2::new(0.0, 32.0)),
                )
                .clicked()
            {
                self.show_workspace();
            }
            ui.add_space(14.0);
        }

        // Start: new, open, continue.
        let gap = 14.0;
        let w = (width - 2.0 * gap) / 3.0;
        let size = Vec2::new(w, 150.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let mut create = false;
            let (_, resp) = card(ui, egui::Id::unique("home-new"), size, true, |ui| {
                heading(ui, Icon::New, "New lab");
                ui.label("Start from an empty canvas and add devices yourself.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.new_title)
                            .hint_text("Name (optional)")
                            .desired_width(w - 110.0),
                    );
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    if ui.button("Create").clicked() || enter {
                        create = true;
                    }
                });
            });
            if create || resp.clicked() {
                let title = std::mem::take(&mut self.new_title);
                self.new_lab(&title);
            }

            let (_, resp) = card(ui, egui::Id::unique("home-open"), size, true, |ui| {
                heading(ui, Icon::Open, "Open a file");
                ui.label("Open a lab you saved before (a .lab.toml file).");
                #[cfg(not(target_arch = "wasm32"))]
                {
                    ui.add_space(6.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.file_path)
                            .hint_text("path/to/lab.toml")
                            .desired_width(w - 40.0),
                    );
                }
            });
            if resp.clicked() {
                self.request_open();
            }

            let resume = self.resume();
            let (_, resp) = card(
                ui,
                egui::Id::unique("home-continue"),
                size,
                resume.is_some(),
                |ui| {
                    heading(ui, Icon::Continue, "Continue");
                    match &resume {
                        Some(Resume::Recovered { title, age }) => {
                            ui.label(RichText::new(title).strong());
                            ui.label(format!("Unsaved work, autosaved {age}."));
                        }
                        Some(Resume::Last { title, age }) => {
                            ui.label(RichText::new(title).strong());
                            ui.label(format!("Last open {age}."));
                        }
                        None => {
                            ui.weak("Nothing to continue yet. Your last lab will appear here.");
                        }
                    }
                },
            );
            if resp.clicked() {
                self.continue_session();
            }
        });

        ui.add_space(28.0);
        ui.label(RichText::new("Examples").size(20.0).strong());
        ui.weak(
            "Ready-made labs that work as soon as they open. Each has a note saying what to try.",
        );
        ui.add_space(8.0);
        let cols = if width > 760.0 { 3 } else { 2 };
        let w = (width - gap * (cols as f32 - 1.0)) / cols as f32;
        let mut chosen = None;
        for row in EXAMPLES.chunks(cols).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (k, e) in row.1.iter().enumerate() {
                    let i = row.0 * cols + k;
                    let (_, resp) = card(
                        ui,
                        egui::Id::unique(("home-example", i)),
                        Vec2::new(w, 138.0),
                        true,
                        |ui| {
                            ui.horizontal(|ui| {
                                for kind in e.kinds {
                                    let (r, _) = ui.allocate_exact_size(
                                        Vec2::new(30.0, 22.0),
                                        egui::Sense::hover(),
                                    );
                                    draw_device_icon(
                                        ui.painter(),
                                        r,
                                        *kind,
                                        kind_colour(*kind),
                                        egui::Stroke::NONE,
                                    );
                                }
                            });
                            ui.add_space(4.0);
                            ui.label(RichText::new(e.title).size(15.0).strong());
                            ui.label(e.blurb);
                        },
                    );
                    if resp.clicked() {
                        chosen = Some(i);
                    }
                }
            });
            ui.add_space(gap);
        }
        if let Some(i) = chosen {
            self.open_example(i);
        }

        ui.add_space(14.0);
        ui.label(RichText::new("How it works").size(20.0).strong());
        ui.add_space(6.0);
        let steps = [
            (
                "1",
                "Add devices",
                "Drag a router, switch or PC from the bar at the bottom onto the canvas.",
            ),
            (
                "2",
                "Cable them",
                "Drag the cable onto one device, then click the device at the other end.",
            ),
            (
                "3",
                "Configure",
                "Click a device to open its console and settings. Type ? at any prompt for help.",
            ),
        ];
        let w = (width - 2.0 * gap) / 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (n, title, text) in steps {
                ui.allocate_ui(Vec2::new(w, 80.0), |ui| {
                    ui.horizontal_top(|ui| {
                        let (r, _) =
                            ui.allocate_exact_size(Vec2::splat(30.0), egui::Sense::hover());
                        ui.painter().circle_filled(
                            r.center(),
                            14.0,
                            ui.visuals().selection.bg_fill,
                        );
                        ui.painter().text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            n,
                            FontId::proportional(15.0),
                            ui.visuals().strong_text_color(),
                        );
                        ui.vertical(|ui| {
                            ui.label(RichText::new(title).strong());
                            ui.label(text);
                        });
                    });
                });
            }
        });
        ui.add_space(30.0);
    }
}

fn heading(ui: &mut Ui, icon: Icon, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::hover());
        icons::paint(ui.painter(), r, icon, ui.visuals().strong_text_color());
        ui.label(RichText::new(text).size(18.0).strong());
    });
    ui.add_space(4.0);
}
