//! Tracks whether the lab has unsaved changes and when an autosave is due.
//!
//! Structural edits (add, move, cable, delete) mark the lab dirty directly. A console
//! command may or may not change a config, so after each one the caller compares the lab's
//! text with the last saved text ([`Changes::observe`]); that is cheap at human typing speed
//! and means running a `ping` does not make the lab look modified.

/// Seconds a lab must sit unchanged before it is autosaved. Short enough to lose little,
/// long enough not to write on every drag frame.
pub const AUTOSAVE_DELAY: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AutosaveState {
    /// Nothing to save: clean, or this exact revision is already saved.
    Idle,
    /// Changed recently; seconds until it is due.
    Waiting(f64),
    Due,
}

#[derive(Debug, Default)]
pub struct Changes {
    /// Lab text as of the last save or open. Empty means "never saved".
    baseline: String,
    /// Lab text as of the last [`Changes::observe`], to notice real changes between checks.
    last_seen: String,
    dirty: bool,
    revision: u64,
    last_change: f64,
    autosaved: u64,
}

impl Changes {
    /// A clean lab whose saved text is `text`.
    pub fn clean(text: String) -> Self {
        Changes {
            last_seen: text.clone(),
            baseline: text,
            ..Changes::default()
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// A structural edit happened at wall time `now`.
    pub fn mark_edited(&mut self, now: f64) {
        self.dirty = true;
        self.revision += 1;
        self.last_change = now;
    }

    /// Compare the lab's current text with what was saved. Call after anything that might
    /// have changed a config.
    pub fn observe(&mut self, text: &str, now: f64) {
        if text != self.last_seen {
            self.revision += 1;
            self.last_change = now;
            self.last_seen = text.to_string();
        }
        self.dirty = text != self.baseline;
    }

    /// The lab was saved (or opened): `text` is now the baseline and nothing is pending.
    pub fn mark_saved(&mut self, text: String) {
        self.last_seen = text.clone();
        self.baseline = text;
        self.dirty = false;
        self.autosaved = self.revision;
    }

    /// Recovered work that exists nowhere but the autosave: dirty until saved for real.
    pub fn mark_recovered(&mut self, text: String, now: f64) {
        self.baseline = String::new();
        self.last_seen = text;
        self.dirty = true;
        self.revision += 1;
        self.last_change = now;
        // The autosave already holds exactly this text.
        self.autosaved = self.revision;
    }

    pub fn autosave_state(&self, now: f64) -> AutosaveState {
        if !self.dirty || self.autosaved == self.revision {
            return AutosaveState::Idle;
        }
        let waited = now - self.last_change;
        if waited >= AUTOSAVE_DELAY {
            AutosaveState::Due
        } else {
            AutosaveState::Waiting(AUTOSAVE_DELAY - waited)
        }
    }

    /// The current revision was written to the autosave.
    pub fn autosaved(&mut self) {
        self.autosaved = self.revision;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_lab_is_idle() {
        let c = Changes::clean("a".into());
        assert!(!c.is_dirty());
        assert_eq!(c.autosave_state(100.0), AutosaveState::Idle);
    }

    #[test]
    fn edits_dirty_the_lab_and_schedule_an_autosave() {
        let mut c = Changes::clean("a".into());
        c.mark_edited(10.0);
        assert!(c.is_dirty());
        assert_eq!(
            c.autosave_state(11.0),
            AutosaveState::Waiting(AUTOSAVE_DELAY - 1.0)
        );
        assert_eq!(c.autosave_state(10.0 + AUTOSAVE_DELAY), AutosaveState::Due);
        c.autosaved();
        assert_eq!(c.autosave_state(99.0), AutosaveState::Idle);
        // A further edit restarts the wait.
        c.mark_edited(50.0);
        assert!(matches!(c.autosave_state(51.0), AutosaveState::Waiting(_)));
    }

    #[test]
    fn observe_ignores_unchanged_text_and_notices_real_changes() {
        let mut c = Changes::clean("v1".into());
        c.observe("v1", 5.0);
        assert!(
            !c.is_dirty(),
            "running a command that changes nothing is not an edit"
        );
        assert_eq!(c.autosave_state(99.0), AutosaveState::Idle);
        c.observe("v2", 6.0);
        assert!(c.is_dirty());
        assert!(matches!(c.autosave_state(7.0), AutosaveState::Waiting(_)));
        // Same text again does not restart the autosave wait.
        c.observe("v2", 8.0);
        assert_eq!(c.autosave_state(6.0 + AUTOSAVE_DELAY), AutosaveState::Due);
    }

    #[test]
    fn undoing_a_change_makes_the_lab_clean_again() {
        let mut c = Changes::clean("v1".into());
        c.observe("v2", 1.0);
        assert!(c.is_dirty());
        c.observe("v1", 2.0);
        assert!(!c.is_dirty());
    }

    #[test]
    fn saving_resets_everything() {
        let mut c = Changes::clean("v1".into());
        c.mark_edited(1.0);
        c.observe("v2", 2.0);
        c.mark_saved("v2".into());
        assert!(!c.is_dirty());
        assert_eq!(c.autosave_state(100.0), AutosaveState::Idle);
        c.observe("v2", 3.0);
        assert!(!c.is_dirty(), "the saved text is the new baseline");
    }

    #[test]
    fn recovered_work_stays_dirty_until_saved_but_is_not_rewritten() {
        let mut c = Changes::clean("fresh".into());
        c.mark_recovered("old work".into(), 1.0);
        assert!(c.is_dirty());
        assert_eq!(c.autosave_state(99.0), AutosaveState::Idle);
        c.observe("old work", 2.0);
        assert!(c.is_dirty(), "never saved to a real file");
        c.mark_saved("old work".into());
        assert!(!c.is_dirty());
    }
}
