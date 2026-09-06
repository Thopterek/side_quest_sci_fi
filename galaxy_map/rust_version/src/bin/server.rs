//! The Parallax backend: `parallax-server`.
//!
//! Owns the database. Clients speak HTTP to it and never hold credentials.

use std::sync::Arc;

use parallax::core::grant::{share_link, Capability, Scope};
use parallax::server::{listen, router, AppState, Repo};

/// `--health-check`: probe our own `/health` and exit 0 or 1.
///
/// Container healthchecks need something inside the image to run, and this
/// image ships no shell and no curl on purpose — adding either to carry a probe
/// would be a wider attack surface than the probe is worth. So the binary
/// probes itself, over plain TCP, with no extra dependency.
fn health_check() -> std::process::ExitCode {
    use std::io::{Read, Write};

    let bind = std::env::var("PARALLAX_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    // 0.0.0.0 is a bind address, not a destination.
    let target = bind.replace("0.0.0.0", "127.0.0.1").replace("[::]", "[::1]");
    let timeout = std::time::Duration::from_secs(3);

    let Ok(addr) = target.parse::<std::net::SocketAddr>() else {
        eprintln!("health: cannot parse {target}");
        return std::process::ExitCode::FAILURE;
    };
    let Ok(mut sock) = std::net::TcpStream::connect_timeout(&addr, timeout) else {
        eprintln!("health: no listener on {target}");
        return std::process::ExitCode::FAILURE;
    };
    let _ = sock.set_read_timeout(Some(timeout));
    if sock
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return std::process::ExitCode::FAILURE;
    }

    let mut head = [0u8; 15];
    if sock.read_exact(&mut head).is_err() {
        eprintln!("health: no response");
        return std::process::ExitCode::FAILURE;
    }
    if head.starts_with(b"HTTP/1.1 200") {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("health: {}", String::from_utf8_lossy(&head));
        std::process::ExitCode::FAILURE
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--health-check") {
        // Not an error path, so bypass the normal startup entirely rather than
        // building a pool just to throw it away.
        let code = health_check();
        std::process::exit(if code == std::process::ExitCode::SUCCESS { 0 } else { 1 });
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "parallax=info,tower_http=warn".into()),
        )
        .init();

    let url = std::env::var("PARALLAX_DATABASE_URL")
        .unwrap_or_else(|_| "host=localhost user=postgres dbname=parallax".into());
    let bind = std::env::var("PARALLAX_BIND").unwrap_or_else(|_| "0.0.0.0:8080".into());
    // Used to render share links, so they point at wherever this is reachable
    // rather than at the address it happens to bind.
    let base_url = std::env::var("PARALLAX_BASE_URL")
        .unwrap_or_else(|_| format!("http://{}", bind.replace("0.0.0.0", "localhost")));
    let pool_size: usize = std::env::var("PARALLAX_POOL_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);

    let repo = Repo::connect(&url, pool_size)?;

    // Migrate before serving: a client that connects to an unmigrated database
    // fails in ways that are tedious to diagnose.
    repo.migrate().await?;
    tracing::info!("schema up to date; pool size {pool_size}");

    let (changes, _) = tokio::sync::broadcast::channel(256);
    listen::spawn(url.clone(), changes.clone());

    let repo = Arc::new(repo);

    // On an unclaimed vault, mint an admin link and print it once. Without this
    // there is no way in: every route needs a grant, and only an admin can mint
    // one. Printed rather than stored anywhere retrievable, so it behaves like
    // the token it is.
    match repo.list_grants().await {
        Ok(grants) if !grants.iter().any(|g| g.capability == Capability::Admin) => {
            match repo
                .mint_grant("Host", Capability::Admin, &Scope::All, None)
                .await
            {
                Ok(minted) => {
                    let link = share_link(&base_url, &minted.token);
                    tracing::warn!("no admin grant existed; minted one for this vault");
                    // Deliberately on stdout, not through the log filter: this
                    // is shown exactly once and must not be lost to RUST_LOG.
                    //
                    // The label and the URL share one line on purpose. Split
                    // across two, the obvious `grep "admin link"` matches the
                    // header and hides the very thing it was run to find —
                    // which is exactly what happened to the first person who
                    // followed the documented command.
                    println!();
                    println!("  PARALLAX ADMIN LINK (shown once, store it now): {link}");
                    println!();
                }
                Err(e) => tracing::error!("could not mint the first admin grant: {e}"),
            }
        }
        Ok(_) => {}
        Err(e) => tracing::error!("could not check for an admin grant: {e}"),
    }

    let app = router(AppState { repo, changes, base_url })
        .layer(tower_http::trace::TraceLayer::new_for_http())
        // The face is a native client, but the wasm build is served from a
        // different origin, so it needs this.
        .layer(tower_http::cors::CorsLayer::permissive());

    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("parallax-server listening on {bind}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// Drain in-flight requests on SIGTERM so a container restart does not abort a
/// write mid-transaction.
async fn shutdown() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok(); };
    #[cfg(unix)]
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
    tracing::info!("shutting down");
}
