//! Undo and redo for canvas edits.
//!
//! Each structural edit (add, move, cable, delete) pushes the lab as it was just before. Undo
//! swaps the lab for the latest entry and keeps the current lab for redo. Console commands are
//! not edits of their own; instead [`carry_configs`] keeps whatever was typed since an entry was
//! recorded, so stepping back through a cabling change never throws configuration away.

use labfile::LabFile;

/// Entries kept per direction. A lab is small text, so this is about memory only in theory.
const LIMIT: usize = 100;

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

#[derive(Debug, Clone)]
struct Entry {
    /// What the edit was, for the status line ("add pc1").
    label: String,
    /// The lab on the other side of the edit.
    lab: LabFile,
}

impl History {
    /// An edit is about to happen (or just did): `before` is the lab as it was.
    pub fn record(&mut self, label: impl Into<String>, before: LabFile) {
        self.redo.clear();
        self.undo.push(Entry {
            label: label.into(),
            lab: before,
        });
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// What undo would undo.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| e.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| e.label.as_str())
    }

    /// Step back: returns the lab to switch to (with typed configs carried over) and its label,
    /// and keeps `current` for redo.
    pub fn step_back(&mut self, current: &LabFile) -> Option<(String, LabFile)> {
        let entry = self.undo.pop()?;
        self.redo.push(Entry {
            label: entry.label.clone(),
            lab: current.clone(),
        });
        let mut lab = entry.lab;
        carry_configs(&mut lab, current);
        Some((entry.label, lab))
    }

    /// Step forward again after an undo.
    pub fn step_forward(&mut self, current: &LabFile) -> Option<(String, LabFile)> {
        let entry = self.redo.pop()?;
        self.undo.push(Entry {
            label: entry.label.clone(),
            lab: current.clone(),
        });
        let mut lab = entry.lab;
        carry_configs(&mut lab, current);
        Some((entry.label, lab))
    }

    /// A step failed to apply: put the stacks back as they were.
    pub fn cancel_back(&mut self, label: String, lab: LabFile) {
        self.redo.pop();
        self.undo.push(Entry { label, lab });
    }

    pub fn cancel_forward(&mut self, label: String, lab: LabFile) {
        self.undo.pop();
        self.redo.push(Entry { label, lab });
    }
}

/// For every device present in both labs, take the configuration from `current`: whatever was
/// typed at a console since `target` was recorded stays. A device that only exists in `target`
/// (undoing a delete) keeps the config it had when it was deleted.
pub fn carry_configs(target: &mut LabFile, current: &LabFile) {
    for d in &mut target.devices {
        if let Some(now) = current.devices.iter().find(|c| c.name == d.name) {
            d.startup_config.clone_from(&now.startup_config);
            d.running_config.clone_from(&now.running_config);
            d.host.clone_from(&now.host);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab(names: &[&str]) -> LabFile {
        let text = {
            let devices: String = names
                .iter()
                .map(|n| {
                    format!("[[device]]\nname = \"{n}\"\ntemplate = \"host-pc\"\npos = [0, 0]\n")
                })
                .collect();
            format!("format = \"labsim-lab\"\nversion = 1\n{devices}")
        };
        labfile::load(&text).expect("test lab parses").lab
    }

    #[test]
    fn undo_then_redo_round_trips_and_a_new_edit_clears_redo() {
        let mut h = History::default();
        let (a, ab) = (lab(&["a"]), lab(&["a", "b"]));
        h.record("add b", a.clone());
        let (label, back) = h.step_back(&ab).expect("one entry");
        assert_eq!(label, "add b");
        assert_eq!(back.devices.len(), 1);
        assert!(h.can_redo() && !h.can_undo());
        let (_, fwd) = h.step_forward(&a).expect("redo");
        assert_eq!(fwd.devices.len(), 2);
        h.step_back(&ab);
        h.record("add c", lab(&["a"]));
        assert!(!h.can_redo(), "a new edit ends the redo branch");
    }

    #[test]
    fn the_stack_is_capped() {
        let mut h = History::default();
        for i in 0..LIMIT + 10 {
            h.record(format!("edit {i}"), lab(&["a"]));
        }
        assert_eq!(h.undo.len(), LIMIT);
        assert_eq!(h.undo_label(), Some(&*format!("edit {}", LIMIT + 9)));
    }

    #[test]
    fn configs_typed_since_the_edit_survive_an_undo() {
        let mut recorded = lab(&["a", "b"]);
        let mut now = lab(&["a"]);
        now.devices[0].running_config = Some("hostname typed-later".into());
        recorded.devices[1].running_config = Some("hostname kept-from-delete".into());
        carry_configs(&mut recorded, &now);
        assert_eq!(
            recorded.devices[0].running_config.as_deref(),
            Some("hostname typed-later")
        );
        assert_eq!(
            recorded.devices[1].running_config.as_deref(),
            Some("hostname kept-from-delete")
        );
    }
}
