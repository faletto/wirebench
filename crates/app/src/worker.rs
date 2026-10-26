//! The engine in a Web Worker (web builds only).
//!
//! The page's own wasm module is started a second time inside a module worker; there it does not
//! draw anything but answers requests: [`worker_start`] makes an [`Engine`] and returns the
//! function the worker's script calls with each message. [`WorkerTransport`] is the page's end.
//! Both sides use the same bundle, so the only thing to find is where it lives; the page's
//! `<link rel="modulepreload">` and `<link rel="preload">` (written by trunk) say.

use crate::link::Transport;
use engine::protocol::Engine;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Blob, BlobPropertyBag, MessageEvent, Url, Worker, WorkerOptions, WorkerType};

/// Who to tell when an answer arrives.
type Waker = Rc<RefCell<Option<Box<dyn Fn()>>>>;

/// True when this code is running inside a worker rather than in the page.
pub fn in_worker() -> bool {
    js_sys::global()
        .dyn_into::<web_sys::WorkerGlobalScope>()
        .is_ok()
}

/// The page's JavaScript glue and wasm module, as trunk linked them.
fn bundle_urls() -> Option<(String, String)> {
    let doc = web_sys::window()?.document()?;
    let js = doc
        .query_selector("link[rel=modulepreload]")
        .ok()??
        .get_attribute("href")?;
    let wasm = doc
        .query_selector("link[rel=preload][as=fetch]")
        .ok()??
        .get_attribute("href")?;
    let base = web_sys::window()?.location().href().ok()?;
    let abs = |p: &str| Url::new_with_base(p, &base).ok().map(|u| u.href());
    Some((abs(&js)?, abs(&wasm)?))
}

/// The worker's script. Messages that arrive while the wasm is still loading are kept and
/// handled once it is up.
fn bootstrap(js: &str, wasm: &str) -> String {
    format!(
        r#"const queue = []; let handler = null;
self.onmessage = (e) => {{ if (handler) handler(e.data); else queue.push(e.data); }};
const mod = await import({js:?});
await mod.default({{ module_or_path: {wasm:?} }});
handler = mod.worker_start((b) => self.postMessage(b, [b.buffer]));
for (const m of queue) handler(m);
"#
    )
}

/// The worker's side: an engine, and the function to give it each message. `post` sends a reply
/// back to the page.
#[wasm_bindgen]
pub fn worker_start(post: js_sys::Function) -> js_sys::Function {
    let mut engine = Engine::new(engine::World::new(kernel::Trace::new()));
    let handler = Closure::<dyn FnMut(JsValue)>::new(move |data: JsValue| {
        let request = js_sys::Uint8Array::new(&data).to_vec();
        let reply = engine.serve(&request);
        let _ = post.call1(&JsValue::NULL, &js_sys::Uint8Array::from(reply.as_slice()));
    });
    handler.into_js_value().unchecked_into()
}

/// The page's side: requests go to the worker, replies wait here until polled.
pub struct WorkerTransport {
    worker: Worker,
    inbox: Rc<RefCell<VecDeque<Vec<u8>>>>,
    wake: Waker,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
}

impl WorkerTransport {
    /// Start a worker, or say why not (no bundle to load, workers blocked): the caller can fall
    /// back to an engine in the page.
    pub fn spawn() -> Result<WorkerTransport, String> {
        let (js, wasm) = bundle_urls().ok_or("the page does not say where its bundle is")?;
        let parts = js_sys::Array::of1(&JsValue::from_str(&bootstrap(&js, &wasm)));
        let bag = BlobPropertyBag::new();
        bag.set_type("text/javascript");
        let blob =
            Blob::new_with_str_sequence_and_options(&parts, &bag).map_err(|e| format!("{e:?}"))?;
        let url = Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
        let opts = WorkerOptions::new();
        opts.set_type(WorkerType::Module);
        let worker = Worker::new_with_options(&url, &opts).map_err(|e| format!("{e:?}"))?;
        let inbox: Rc<RefCell<VecDeque<Vec<u8>>>> = Rc::default();
        let wake: Waker = Rc::default();
        let (i, w) = (inbox.clone(), wake.clone());
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            i.borrow_mut()
                .push_back(js_sys::Uint8Array::new(&e.data()).to_vec());
            if let Some(f) = &*w.borrow() {
                f();
            }
        });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        Ok(WorkerTransport {
            worker,
            inbox,
            wake,
            _on_message: on_message,
        })
    }
}

impl Transport for WorkerTransport {
    fn post(&mut self, request: Vec<u8>) {
        let bytes = js_sys::Uint8Array::from(request.as_slice());
        let _ = self.worker.post_message(&bytes);
    }

    fn poll(&mut self) -> Vec<Vec<u8>> {
        self.inbox.borrow_mut().drain(..).collect()
    }

    fn set_waker(&mut self, wake: Box<dyn Fn()>) {
        *self.wake.borrow_mut() = Some(wake);
    }
}
