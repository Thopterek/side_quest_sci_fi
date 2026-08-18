//! The HTTP API.
//!
//! Every route is scoped to a caller identified by the `X-Parallax-User` header.
//! The vault itself is shared — that is the point — but the *view* onto it is
//! not: selection, comparison and the open planet are per user.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::core::model::System;
use crate::core::store::Settings;

use crate::core::patch::{PlanetRecordPatch, RecordPatch};
use super::repo::{Repo, RepoError, VaultSnapshot};

pub const USER_HEADER: &str = "x-parallax-user";

#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<Repo>,
    /// Broadcast of changed system ids, fed by Postgres LISTEN/NOTIFY.
    pub changes: tokio::sync::broadcast::Sender<String>,
}

/// Identify the caller. Anonymous callers share the `local` view, which keeps
/// single-user operation working with no configuration.
fn user_of(headers: &HeaderMap) -> String {
    headers
        .get(USER_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("local")
        .to_string()
}

#[derive(Serialize)]
struct ApiError {
    error: String,
    /// Present on a version conflict, so the client knows what to re-read.
    #[serde(skip_serializing_if = "Option::is_none")]
    current_version: Option<i32>,
}

impl IntoResponse for RepoError {
    fn into_response(self) -> Response {
        let (status, current) = match &self {
            RepoError::NotFound => (StatusCode::NOT_FOUND, None),
            RepoError::Conflict { current } => (StatusCode::CONFLICT, Some(*current)),
            RepoError::Pool(_) => (StatusCode::SERVICE_UNAVAILABLE, None),
            RepoError::Query(_) => (StatusCode::INTERNAL_SERVER_ERROR, None),
        };
        let body = ApiError { error: self.to_string(), current_version: current };
        (status, Json(body)).into_response()
    }
}

#[derive(Serialize)]
pub struct VersionResponse {
    pub version: i32,
}

#[derive(Deserialize)]
pub struct SeedRequest {
    #[serde(default)]
    pub force: bool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/vault", get(vault))
        .route("/systems/{id}", put(put_system).delete(delete_system))
        .route("/systems/{id}/record", patch(patch_record))
        .route("/systems/{id}/planets/{planet}/record", patch(patch_planet_record))
        .route("/settings", put(put_settings))
        .route("/seed", post(seed))
        .route("/events", get(events))
        .with_state(state)
}

async fn health(State(s): State<AppState>) -> Result<Json<serde_json::Value>, RepoError> {
    s.repo.health().await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

async fn vault(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<VaultSnapshot>, RepoError> {
    Ok(Json(s.repo.snapshot(&user_of(&headers)).await?))
}

/// Refresh a system's archive fields. Idempotent, and never touches a dossier.
async fn put_system(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(mut system): Json<System>,
) -> Result<Json<VersionResponse>, RepoError> {
    // The path is authoritative: a body claiming a different id would otherwise
    // let a client write to a system it did not address.
    system.id = id;
    let version = s.repo.upsert_system(&system).await?;
    Ok(Json(VersionResponse { version }))
}

async fn patch_record(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<RecordPatch>,
) -> Result<Json<VersionResponse>, RepoError> {
    let version = s.repo.patch_record(&id, &patch).await?;
    Ok(Json(VersionResponse { version }))
}

async fn patch_planet_record(
    State(s): State<AppState>,
    Path((id, planet)): Path<(String, String)>,
    Json(patch): Json<PlanetRecordPatch>,
) -> Result<Json<VersionResponse>, RepoError> {
    let version = s.repo.patch_planet_record(&id, &planet, &patch).await?;
    Ok(Json(VersionResponse { version }))
}

async fn delete_system(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, RepoError> {
    s.repo.delete_system(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn put_settings(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, RepoError> {
    s.repo.save_settings(&user_of(&headers), &settings).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Populate an empty vault with the shipped neighbourhood.
///
/// Server-side rather than client-side: with several clients starting at once,
/// each seeding independently would race. Here it is one guarded operation, and
/// by default it refuses to run against a vault that already has content.
async fn seed(
    State(s): State<AppState>,
    Json(req): Json<SeedRequest>,
) -> Result<Json<serde_json::Value>, RepoError> {
    match s.repo.seed(req.force).await? {
        Some(count) => Ok(Json(serde_json::json!({ "seeded": true, "systems": count }))),
        None => Ok(Json(serde_json::json!({
            "seeded": false,
            "reason": "vault is not empty",
        }))),
    }
}

/// Server-sent events carrying the id of every changed system.
///
/// Clients re-read what they care about rather than receiving rows: NOTIFY has
/// an 8 kB payload limit that a system with its planets can exceed.
async fn events(State(s): State<AppState>) -> impl IntoResponse {
    use axum::response::sse::{Event, Sse};
    use futures_util::stream::StreamExt;

    let rx = s.changes.subscribe();
    let stream = tokio_stream_from(rx).map(|id| Ok::<_, std::convert::Infallible>(
        Event::default().event("changed").data(id),
    ));
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

/// Adapt a broadcast receiver into a stream without pulling in tokio-stream.
fn tokio_stream_from(
    mut rx: tokio::sync::broadcast::Receiver<String>,
) -> impl futures_util::Stream<Item = String> {
    futures_util::stream::unfold(rx_holder(&mut rx), |mut holder| async move {
        loop {
            match holder.rx.recv().await {
                Ok(id) => return Some((id, holder)),
                // A slow client misses intermediate ids; the next one it does
                // receive still prompts a re-read, so nothing is lost.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

struct RxHolder {
    rx: tokio::sync::broadcast::Receiver<String>,
}

fn rx_holder(rx: &mut tokio::sync::broadcast::Receiver<String>) -> RxHolder {
    RxHolder { rx: rx.resubscribe() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn an_anonymous_caller_gets_the_local_view() {
        assert_eq!(user_of(&HeaderMap::new()), "local");
    }

    #[test]
    fn a_named_caller_gets_their_own_view() {
        let mut h = HeaderMap::new();
        h.insert(USER_HEADER, HeaderValue::from_static("alice"));
        assert_eq!(user_of(&h), "alice");
    }

    #[test]
    fn a_blank_user_header_falls_back_rather_than_creating_an_empty_identity() {
        // user_settings.user_id has a not-blank constraint; sending "  " must
        // not reach it.
        let mut h = HeaderMap::new();
        h.insert(USER_HEADER, HeaderValue::from_static("   "));
        assert_eq!(user_of(&h), "local");
    }
}
