//! Where autosaves live. The app talks to a [`Store`]; `App::new()` uses [`NullStore`] so
//! tests and embedders never write to a real data directory unless they ask to.
//!
//! Native: one file in the user's data directory. Web: `localStorage` (a lab is tens of
//! kilobytes, well under its limit, and it needs no async API).

use std::cell::RefCell;
use std::fmt::Debug;
use std::rc::Rc;

/// The most recent unsaved state of a lab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autosave {
    pub text: String,
    /// Seconds since the Unix epoch when it was written.
    pub unix_secs: u64,
}

pub trait Store: Debug {
    fn load(&self) -> Option<Autosave>;
    fn save(&mut self, a: &Autosave) -> Result<(), String>;
    fn clear(&mut self);
}

/// Discards everything. The default, so nothing touches disk by accident.
#[derive(Debug, Default)]
pub struct NullStore;

impl Store for NullStore {
    fn load(&self) -> Option<Autosave> {
        None
    }
    fn save(&mut self, _: &Autosave) -> Result<(), String> {
        Ok(())
    }
    fn clear(&mut self) {}
}

/// In-memory store whose contents the creator can inspect (clones share the cell). For tests.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore(pub Rc<RefCell<Option<Autosave>>>);

impl Store for MemoryStore {
    fn load(&self) -> Option<Autosave> {
        self.0.borrow().clone()
    }
    fn save(&mut self, a: &Autosave) -> Result<(), String> {
        *self.0.borrow_mut() = Some(a.clone());
        Ok(())
    }
    fn clear(&mut self) {
        *self.0.borrow_mut() = None;
    }
}

/// Seconds since the Unix epoch.
#[cfg(not(target_arch = "wasm32"))]
pub fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(target_arch = "wasm32")]
pub fn now_unix_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// `# wirebench autosave <secs>` then the lab text. The header is a TOML comment, so the
/// file is itself a valid lab you can open by hand.
pub fn encode(a: &Autosave) -> String {
    format!("# wirebench autosave {}\n{}", a.unix_secs, a.text)
}

pub fn decode(raw: &str) -> Option<Autosave> {
    let (head, text) = raw.split_once('\n')?;
    let unix_secs = head
        .strip_prefix("# wirebench autosave ")?
        .trim()
        .parse()
        .ok()?;
    Some(Autosave {
        text: text.to_string(),
        unix_secs,
    })
}

/// "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn age_text(saved: u64, now: u64) -> String {
    let secs = now.saturating_sub(saved);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use native::{default_dir, FileStore};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::path::PathBuf;

    /// `WIREBENCH_DATA_DIR` if set, else the platform's per-user data directory.
    pub fn default_dir() -> PathBuf {
        if let Some(d) = std::env::var_os("WIREBENCH_DATA_DIR") {
            return d.into();
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let dir = if cfg!(target_os = "macos") {
            home.map(|h| h.join("Library/Application Support"))
        } else if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| home.map(|h| h.join(".local/share")))
        };
        dir.unwrap_or_else(std::env::temp_dir).join("wirebench")
    }

    /// One file, `autosave.lab.toml`, in `dir`.
    #[derive(Debug)]
    pub struct FileStore {
        path: PathBuf,
    }

    impl FileStore {
        pub fn new(dir: PathBuf) -> Self {
            FileStore {
                path: dir.join("autosave.lab.toml"),
            }
        }
    }

    impl Store for FileStore {
        fn load(&self) -> Option<Autosave> {
            decode(&std::fs::read_to_string(&self.path).ok()?)
        }

        fn save(&mut self, a: &Autosave) -> Result<(), String> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            // Write beside it and rename, so a crash mid-write never leaves half a file.
            let tmp = self.path.with_extension("tmp");
            std::fs::write(&tmp, encode(a)).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
        }

        fn clear(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::LocalStorageStore;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;

    const KEY: &str = "wirebench.autosave";

    #[derive(Debug, Default)]
    pub struct LocalStorageStore;

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    impl Store for LocalStorageStore {
        fn load(&self) -> Option<Autosave> {
            decode(&storage()?.get_item(KEY).ok().flatten()?)
        }

        fn save(&mut self, a: &Autosave) -> Result<(), String> {
            storage()
                .ok_or("local storage is unavailable")?
                .set_item(KEY, &encode(a))
                .map_err(|e| format!("{e:?}"))
        }

        fn clear(&mut self) {
            if let Some(s) = storage() {
                let _ = s.remove_item(KEY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip_and_the_file_is_still_a_lab() {
        let a = Autosave {
            text: "format = \"labsim-lab\"\nversion = 1\n".into(),
            unix_secs: 1_700_000_000,
        };
        let raw = encode(&a);
        assert_eq!(decode(&raw), Some(a));
        assert!(labfile::load(&raw).is_ok(), "the header is a TOML comment");
    }

    #[test]
    fn decode_rejects_anything_that_is_not_ours() {
        for bad in [
            "",
            "no newline",
            "# something else\nbody",
            "# wirebench autosave x\nbody",
        ] {
            assert_eq!(decode(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn age_text_picks_sensible_units() {
        assert_eq!(age_text(100, 100), "just now");
        assert_eq!(age_text(100, 100 + 59), "just now");
        assert_eq!(age_text(0, 300), "5 min ago");
        assert_eq!(age_text(0, 3 * 3600 + 5), "3 h ago");
        assert_eq!(age_text(0, 2 * 86_400), "2 days ago");
        assert_eq!(
            age_text(500, 100),
            "just now",
            "a clock that went backwards"
        );
    }

    #[test]
    fn memory_store_is_shared_between_clones() {
        let a = MemoryStore::default();
        let mut b = a.clone();
        assert_eq!(a.load(), None);
        b.save(&Autosave {
            text: "x".into(),
            unix_secs: 1,
        })
        .unwrap();
        assert_eq!(a.load().unwrap().text, "x");
        b.clear();
        assert_eq!(a.load(), None);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn file_store_round_trips_replaces_and_clears() {
        let dir = std::env::temp_dir().join(format!("wirebench-autosave-{}", std::process::id()));
        let mut s = FileStore::new(dir.join("nested"));
        assert_eq!(s.load(), None, "nothing yet, and no directory yet");
        s.save(&Autosave {
            text: "one".into(),
            unix_secs: 1,
        })
        .unwrap();
        s.save(&Autosave {
            text: "two".into(),
            unix_secs: 2,
        })
        .unwrap();
        assert_eq!(
            s.load(),
            Some(Autosave {
                text: "two".into(),
                unix_secs: 2
            })
        );
        assert!(
            !dir.join("nested/autosave.tmp").exists(),
            "no temp file left behind"
        );
        s.clear();
        assert_eq!(s.load(), None);
        s.clear(); // clearing nothing is fine
        let _ = std::fs::remove_dir_all(&dir);
    }
}
