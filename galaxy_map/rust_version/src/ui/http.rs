//! Fetching from the archive.
//!
//! Isolated here because it is the one place native and wasm genuinely differ.
//! `ehttp` papers over that, so all this module owns is the retry chain and the
//! shared slot the callback writes into.

use std::sync::{Arc, Mutex};

use crate::core::model::System;
use crate::core::nasa::{parse_rows, query_url, NasaError};

/// Where a background fetch deposits its answer.
pub type Slot = Arc<Mutex<Option<Result<Vec<System>, String>>>>;

pub fn slot() -> Slot {
    Arc::new(Mutex::new(None))
}

/// Browsers may refuse a cross-origin request to the archive. Two public
/// read-only mirrors are tried before giving up and offering manual paste.
fn routes(url: &str) -> Vec<String> {
    vec![
        url.to_string(),
        format!("https://corsproxy.io/?url={}", crate::core::nasa::url_encode(url)),
        format!("https://api.allorigins.win/raw?url={}", crate::core::nasa::url_encode(url)),
    ]
}

/// Start a fetch. Returns immediately; the answer lands in `slot`.
///
/// `repaint` is called when the result arrives so the UI wakes up even if the
/// window is idle.
pub fn fetch(adql: &str, slot: Slot, repaint: impl Fn() + Send + Sync + 'static) {
    let urls = routes(&query_url(adql));
    try_route(urls, 0, slot, Arc::new(repaint));
}

fn try_route(
    urls: Vec<String>,
    index: usize,
    slot: Slot,
    repaint: Arc<dyn Fn() + Send + Sync>,
) {
    // `Fn() + Send` is wrapped in an Arc so each retry can hold its own handle.
    let Some(url) = urls.get(index).cloned() else {
        *slot.lock().unwrap() = Some(Err(NasaError::Http("no route succeeded".into()).to_string()));
        repaint();
        return;
    };

    let request = ehttp::Request::get(&url);
    ehttp::fetch(request, move |result| {
        let body = match result {
            Ok(resp) if resp.ok => resp.text().map(|s| s.to_owned()),
            Ok(resp) => {
                retry_or_fail(urls, index, slot, repaint, format!("status {}", resp.status));
                return;
            }
            Err(e) => {
                retry_or_fail(urls, index, slot, repaint, e);
                return;
            }
        };

        match body.as_deref().map(parse_rows) {
            Some(Ok(systems)) => {
                *slot.lock().unwrap() = Some(Ok(systems));
                repaint();
            }
            // An empty result is a real answer, not a transport failure, so it
            // stops the retry chain rather than trying the next mirror.
            Some(Err(NasaError::Empty)) => {
                *slot.lock().unwrap() = Some(Err(NasaError::Empty.to_string()));
                repaint();
            }
            Some(Err(e)) => retry_or_fail(urls, index, slot, repaint, e.to_string()),
            None => retry_or_fail(urls, index, slot, repaint, "empty body".into()),
        }
    });
}

fn retry_or_fail(
    urls: Vec<String>,
    index: usize,
    slot: Slot,
    repaint: Arc<dyn Fn() + Send + Sync>,
    reason: String,
) {
    if index + 1 < urls.len() {
        try_route(urls, index + 1, slot, repaint);
    } else {
        *slot.lock().unwrap() = Some(Err(format!("could not reach the archive ({reason})")));
        repaint();
    }
}
