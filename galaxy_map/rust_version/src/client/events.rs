//! Watching the vault for other people's edits.
//!
//! Without this the face is correct but stale: another operator renames a
//! system and you keep looking at the old name until you restart. The server
//! already fans PostgreSQL notifications out over SSE at `/events`; this reads
//! that stream on a thread of its own and rings a bell.
//!
//! Deliberately dumb about *what* changed. The payload is a system id, but
//! acting on it would mean a partial-update path that has to stay in step with
//! the full-load path, and a vault is small. Any change means "reload", and the
//! coalescing below keeps a burst of edits from meaning a burst of reloads.

use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Minimum gap between reloads.
///
/// Leading edge, not trailing: the first notification fires immediately and
/// further ones inside the window are dropped. A trailing-edge debounce would
/// have to wait out the window before acting, and since the loop below blocks
/// in `read_line`, a quiet stream would leave the timer unchecked and the
/// reload would never happen at all — which is exactly the bug this comment
/// replaces. Leading edge also means a single edit, the common case, is seen
/// with no added latency.
const COALESCE: Duration = Duration::from_millis(300);

/// Reconnect backoff. Starts short so a server restart is barely noticed, and
/// grows so a server that is gone for the afternoon is not hammered.
const RETRY_MIN: Duration = Duration::from_millis(500);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Handle to the listener thread. Dropping it stops the thread at its next
/// read boundary.
pub struct ChangeListener {
    stop: Arc<AtomicBool>,
}

impl ChangeListener {
    /// Watch `base_url/events`, calling `on_change` when the vault moves.
    ///
    /// `on_change` runs on the listener thread, is already coalesced, and
    /// should do something cheap — post a reload request and ask for a repaint.
    pub fn spawn(
        base_url: &str,
        token: Option<String>,
        on_change: impl Fn() + Send + 'static,
    ) -> ChangeListener {
        let stop = Arc::new(AtomicBool::new(false));
        let url = format!("{}/events", base_url.trim_end_matches('/'));
        let flag = stop.clone();

        std::thread::Builder::new()
            .name("parallax-events".into())
            .spawn(move || run(url, token, on_change, flag))
            .expect("spawn the change listener");

        ChangeListener { stop }
    }
}

impl Drop for ChangeListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(
    url: String,
    token: Option<String>,
    on_change: impl Fn(),
    stop: Arc<AtomicBool>,
) {
    let agent = ureq::AgentBuilder::new()
        // No read timeout: an SSE stream is idle by design, and a timeout here
        // would look like a disconnection every time the vault is quiet.
        .timeout_connect(Duration::from_secs(5))
        .build();

    let mut backoff = RETRY_MIN;

    while !stop.load(Ordering::Relaxed) {
        let mut req = agent.get(&url).set("Accept", "text/event-stream");
        if let Some(t) = &token {
            req = req.set("Authorization", &format!("Bearer {t}"));
        }

        match req.call() {
            Ok(response) => {
                backoff = RETRY_MIN;
                let mut reader = BufReader::new(response.into_reader());
                let mut line = String::new();
                let mut last_fired: Option<Instant> = None;

                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    line.clear();
                    match reader.read_line(&mut line) {
                        // End of stream: fall out and reconnect.
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }

                    // `event:` names the frame and `:` starts a keep-alive
                    // comment; only `data:` carries a change.
                    if !line.starts_with("data:") {
                        continue;
                    }
                    let due = last_fired.map_or(true, |t| t.elapsed() >= COALESCE);
                    if due {
                        last_fired = Some(Instant::now());
                        on_change();
                    }
                }
            }
            Err(_) => {
                // Unreachable, or a token the server will not accept. Either
                // way the face keeps working against what it already loaded;
                // it simply stops learning about other people's edits.
            }
        }

        // Sleep in slices so dropping the handle is noticed promptly rather
        // than after a full backoff.
        let deadline = Instant::now() + backoff;
        while Instant::now() < deadline {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100).min(backoff));
        }
        backoff = (backoff * 2).min(RETRY_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_handle_asks_the_thread_to_stop() {
        let listener = ChangeListener::spawn("http://127.0.0.1:1", None, || {});
        let flag = listener.stop.clone();
        assert!(!flag.load(Ordering::Relaxed));
        drop(listener);
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn the_backoff_is_bounded() {
        let mut b = RETRY_MIN;
        for _ in 0..20 {
            b = (b * 2).min(RETRY_MAX);
        }
        assert_eq!(b, RETRY_MAX, "backoff must not grow without limit");
        assert!(RETRY_MIN < RETRY_MAX);
    }

    #[test]
    fn an_unreachable_server_does_not_panic_the_thread() {
        // Port 1 refuses immediately, so this exercises the error path.
        let listener = ChangeListener::spawn("http://127.0.0.1:1", None, || {});
        std::thread::sleep(Duration::from_millis(200));
        drop(listener);
    }
}
