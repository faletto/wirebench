//! Getting labs in and out of the app.
//!
//! Native: a path typed into the toolbar; Save writes it, Open reads it. Web: Save downloads
//! a file and Open shows the browser's file picker; the chosen file arrives asynchronously
//! through an [`Inbox`] that the app drains once per frame.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Largest lab file we will read. Real labs are tens of kilobytes; this stops a wrong file
/// (a video, a core dump) from stalling the UI.
pub const MAX_LAB_BYTES: usize = 5_000_000;

/// Something that arrived from outside the app.
// Only the web build produces these; native keeps them so tests and shared code compile.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Lab {
        name: String,
        text: String,
    },
    /// The file could not be read at all.
    Failed(String),
}

/// A queue shared between the UI and asynchronous browser callbacks (single-threaded).
#[derive(Clone, Default)]
pub struct Inbox(Rc<RefCell<VecDeque<Incoming>>>);

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Inbox({} waiting)", self.0.borrow().len())
    }
}

impl Inbox {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn push(&self, item: Incoming) {
        self.0.borrow_mut().push_back(item);
    }

    /// Everything waiting, oldest first.
    pub fn take(&self) -> Vec<Incoming> {
        self.0.borrow_mut().drain(..).collect()
    }
}

/// `Two routers, static routes` -> `two-routers-static-routes.lab.toml`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn file_name_for(title: &str) -> String {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    format!(
        "{}.lab.toml",
        if slug.is_empty() { "wirebench" } else { slug }
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write_file(path: &str, text: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("type a file name first".into());
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &str) -> Result<String, String> {
    if path.trim().is_empty() {
        return Err("type a file name first".into());
    }
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_LAB_BYTES as u64 {
        return Err(format!("{path} is too large to be a lab ({len} bytes)"));
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Hand `text` to the browser as a download called `name`.
#[cfg(target_arch = "wasm32")]
pub fn download(name: &str, text: &str) -> Result<(), String> {
    use eframe::wasm_bindgen::JsCast as _;
    let js = |e: eframe::wasm_bindgen::JsValue| format!("{e:?}");
    let doc = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let parts = js_sys::Array::of1(&text.into());
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("text/plain;charset=utf-8");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = doc
        .create_element("a")
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(name);
    let body = doc.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

/// Show the browser's file picker; the chosen file lands in `inbox`.
#[cfg(target_arch = "wasm32")]
pub fn pick(inbox: Inbox, ctx: egui::Context) -> Result<(), String> {
    use eframe::wasm_bindgen::closure::Closure;
    use eframe::wasm_bindgen::JsCast as _;
    let js = |e: eframe::wasm_bindgen::JsValue| format!("{e:?}");
    let doc = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("no document")?;
    let input: web_sys::HtmlInputElement = doc
        .create_element("input")
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "not an input")?;
    input.set_type("file");
    input.set_accept(".toml,text/plain");
    let chosen = input.clone();
    let onchange = Closure::once_into_js(move || {
        let Some(file) = chosen.files().and_then(|f| f.get(0)) else {
            return;
        };
        if file.size() > MAX_LAB_BYTES as f64 {
            inbox.push(Incoming::Failed(format!(
                "{} is too large to be a lab",
                file.name()
            )));
            ctx.request_repaint();
            return;
        }
        let name = file.name();
        let Ok(reader) = web_sys::FileReader::new() else {
            return;
        };
        let done = reader.clone();
        let onload = Closure::once_into_js(move || {
            match done.result().ok().and_then(|v| v.as_string()) {
                Some(text) => inbox.push(Incoming::Lab { name, text }),
                None => inbox.push(Incoming::Failed("could not read the file as text".into())),
            }
            ctx.request_repaint();
        });
        reader.set_onload(Some(onload.unchecked_ref()));
        let _ = reader.read_as_text(&file);
    });
    input.set_onchange(Some(onchange.unchecked_ref()));
    input.click();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe_slugs() {
        assert_eq!(
            file_name_for("Two routers, static routes"),
            "two-routers-static-routes.lab.toml"
        );
        assert_eq!(file_name_for("  ../../etc/passwd  "), "etc-passwd.lab.toml");
        assert_eq!(file_name_for(""), "wirebench.lab.toml");
        assert_eq!(file_name_for("!!!"), "wirebench.lab.toml");
        assert_eq!(file_name_for("R1 & R2 (OSPF)"), "r1-r2-ospf.lab.toml");
    }

    #[test]
    fn inbox_is_fifo_and_drains() {
        let a = Inbox::default();
        let b = a.clone();
        a.push(Incoming::Failed("one".into()));
        b.push(Incoming::Lab {
            name: "n".into(),
            text: "t".into(),
        });
        assert_eq!(
            a.take(),
            vec![
                Incoming::Failed("one".into()),
                Incoming::Lab {
                    name: "n".into(),
                    text: "t".into()
                }
            ]
        );
        assert!(b.take().is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_files_round_trip_and_reject_bad_paths() {
        let dir = std::env::temp_dir().join(format!("wirebench-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.lab.toml");
        let p = path.to_str().unwrap();
        write_file(p, "hello").unwrap();
        assert_eq!(read_file(p).unwrap(), "hello");
        assert!(read_file(dir.join("missing.toml").to_str().unwrap()).is_err());
        assert!(write_file("  ", "x").is_err());
        assert!(read_file("").is_err());
        let big = dir.join("big.toml");
        std::fs::write(&big, vec![b'a'; MAX_LAB_BYTES + 1]).unwrap();
        assert!(read_file(big.to_str().unwrap())
            .unwrap_err()
            .contains("too large"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
