//! Postgres LISTEN/NOTIFY, fanned out to connected clients.
//!
//! Held on its own dedicated connection rather than borrowed from the pool: a
//! listening session must not be recycled, and taking one out of a pool of N
//! would quietly reduce the write capacity to N-1.

use futures_util::{stream, StreamExt};
use tokio::sync::broadcast;
use tokio_postgres::{AsyncMessage, NoTls};

/// Connect, `LISTEN parallax_changed`, and republish every payload.
///
/// Reconnects on failure: a dropped listener would leave clients silently stale,
/// which is worse than a brief gap because nothing signals it.
pub fn spawn(url: String, tx: broadcast::Sender<String>) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = listen_once(&url, &tx).await {
                tracing::warn!("change listener dropped: {e}; retrying in 2s");
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

async fn listen_once(
    url: &str,
    tx: &broadcast::Sender<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (client, mut connection) = tokio_postgres::connect(url, NoTls).await?;

    // Order matters here, and getting it wrong deadlocks silently.
    //
    // `poll_message` is the only way to see asynchronous notifications, and it
    // borrows the connection — so the connection cannot also be driven by the
    // usual `tokio::spawn(connection)`. But `client.batch_execute("LISTEN ...")`
    // cannot complete unless something *is* driving the connection. Pumping
    // first and issuing LISTEN second is what makes both possible.
    let sender = tx.clone();
    let pump = tokio::spawn(async move {
        let mut messages = stream::poll_fn(move |cx| connection.poll_message(cx));
        while let Some(message) = messages.next().await {
            match message {
                Ok(AsyncMessage::Notification(note)) => {
                    // A send error only means nobody is subscribed right now.
                    let _ = sender.send(note.payload().to_string());
                }
                Ok(AsyncMessage::Notice(notice)) => {
                    tracing::debug!("postgres notice: {notice}")
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("listen connection error: {e}");
                    break;
                }
            }
        }
    });

    client.batch_execute("listen parallax_changed").await?;
    tracing::info!("listening for vault changes");

    // Hold the client for as long as the pump lives; dropping it would close the
    // session and end the subscription.
    pump.await?;
    Ok(())
}
