//! The HTTP API.
//!
//! Every route resolves a [`Caller`] from the share link presented, and the
//! grant behind that link decides two things: which systems the response may
//! contain, and whether it may change anything.
//!
//! The vault itself is shared — that is the point — but the *view* onto it is
//! not: selection, comparison and the open planet are per grant.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::core::model::System;
use crate::core::store::Settings;

use crate::core::grant::{share_link, Capability, Grant, MintedGrant, Scope};
use crate::core::patch::{PlanetRecordPatch, RecordPatch};
use super::auth::{AuthError, Caller};
use super::repo::{Repo, RepoError, VaultSnapshot};

/// One error type for handlers, so a route can fail on authorisation, on the
/// database, or on a missing system without three separate return types.
#[derive(Debug)]
pub enum Failure {
    Auth(AuthError),
    Repo(RepoError),
    /// Also returned for a system the caller may not see. Distinguishing the
    /// two would let a link enumerate the vault by probing ids.
    NotFound,
}

impl From<AuthError> for Failure {
    fn from(e: AuthError) -> Self {
        Failure::Auth(e)
    }
}

impl From<RepoError> for Failure {
    fn from(e: RepoError) -> Self {
        Failure::Repo(e)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        match self {
            Failure::Auth(e) => e.into_response(),
            Failure::Repo(e) => e.into_response(),
            Failure::NotFound => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "no such system in this stage" })),
            )
                .into_response(),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<Repo>,
    /// Broadcast of changed system ids, fed by Postgres LISTEN/NOTIFY.
    pub changes: tokio::sync::broadcast::Sender<String>,
    /// Public base URL, used to render share links. Without it a minted link
    /// would be a bare token the host has to assemble by hand.
    pub base_url: String,
}

/// Lets the `Caller` extractor reach the pool without knowing about the rest
/// of the state.
impl axum::extract::FromRef<AppState> for Arc<Repo> {
    fn from_ref(s: &AppState) -> Arc<Repo> {
        s.repo.clone()
    }
}

/// A system a grant cannot see must be indistinguishable from one that does
/// not exist, otherwise the error message itself leaks the vault's contents.
async fn require_visible(
    s: &AppState,
    caller: &Caller,
    system_id: &str,
) -> Result<(), Failure> {
    if s.repo.grant_can_see(caller.id(), system_id).await? {
        Ok(())
    } else {
        Err(Failure::NotFound)
    }
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
        .route("/grants", get(list_grants).post(mint_grant))
        .route("/grants/{id}", axum::routing::delete(revoke_grant))
        .route("/whoami", get(whoami))
        .with_state(state)
}

async fn health(State(s): State<AppState>) -> Result<Json<serde_json::Value>, RepoError> {
    s.repo.health().await?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

/// The vault, as this link may see it. Filtering happens in SQL, so what is
/// withheld never reaches the process, let alone the wire.
async fn vault(
    State(s): State<AppState>,
    caller: Caller,
) -> Result<Json<VaultSnapshot>, Failure> {
    caller.require(Capability::Read)?;
    Ok(Json(s.repo.snapshot(caller.settings_key()).await?))
}

/// What this link is and what it can do, so the face can say so plainly rather
/// than leaving the viewer to guess why an edit box is missing.
async fn whoami(caller: Caller) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "grant": caller.grant.id,
        "label": caller.grant.label,
        "capability": caller.grant.capability.as_str(),
        "scope": caller.grant.scope,
        "anonymous": caller.grant.is_anonymous(),
        "can_write": caller.can(Capability::Write),
        "can_administer": caller.can(Capability::Admin),
        "expires_at": caller.grant.expires_at,
    }))
}

/* ----------------------------------------------------------------- grants */

#[derive(Deserialize)]
pub struct MintRequest {
    #[serde(default)]
    pub label: String,
    #[serde(default = "default_capability")]
    pub capability: Capability,
    pub scope: Scope,
    /// Days until the link stops working. Absent means it never does, which is
    /// convenient and is the option to think twice about.
    #[serde(default)]
    pub expires_in_days: Option<i32>,
}

fn default_capability() -> Capability {
    Capability::Read
}

#[derive(Serialize)]
pub struct MintResponse {
    #[serde(flatten)]
    pub minted: MintedGrant,
    /// The link to hand out. Shown once; only a digest is stored.
    pub link: String,
}

async fn list_grants(
    State(s): State<AppState>,
    caller: Caller,
) -> Result<Json<Vec<Grant>>, Failure> {
    caller.require(Capability::Admin)?;
    Ok(Json(s.repo.list_grants().await?))
}

async fn mint_grant(
    State(s): State<AppState>,
    caller: Caller,
    Json(req): Json<MintRequest>,
) -> Result<Json<MintResponse>, Failure> {
    caller.require(Capability::Admin)?;
    let minted = s
        .repo
        .mint_grant(&req.label, req.capability, &req.scope, req.expires_in_days)
        .await?;
    let link = share_link(&s.base_url, &minted.token);
    Ok(Json(MintResponse { minted, link }))
}

async fn revoke_grant(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, Failure> {
    caller.require(Capability::Admin)?;
    let revoked = s.repo.revoke_grant(&id).await?;
    Ok(Json(serde_json::json!({ "revoked": revoked })))
}

/// Refresh a system's archive fields. Idempotent, and never touches a dossier.
async fn put_system(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    Json(mut system): Json<System>,
) -> Result<Json<VersionResponse>, Failure> {
    caller.require(Capability::Write)?;
    // Adding a system that is not yet in the vault is only meaningful for a
    // link that can see the whole thing; a curated stage would otherwise be
    // able to grow itself.
    if !caller.can(Capability::Admin) {
        require_visible(&s, &caller, &id).await?;
    }
    // The path is authoritative: a body claiming a different id would otherwise
    // let a client write to a system it did not address.
    system.id = id;
    let version = s.repo.upsert_system(&system).await?;
    Ok(Json(VersionResponse { version }))
}

async fn patch_record(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
    Json(patch): Json<RecordPatch>,
) -> Result<Json<VersionResponse>, Failure> {
    caller.require(Capability::Write)?;
    require_visible(&s, &caller, &id).await?;
    let version = s.repo.patch_record(&id, &patch).await?;
    Ok(Json(VersionResponse { version }))
}

async fn patch_planet_record(
    State(s): State<AppState>,
    caller: Caller,
    Path((id, planet)): Path<(String, String)>,
    Json(patch): Json<PlanetRecordPatch>,
) -> Result<Json<VersionResponse>, Failure> {
    caller.require(Capability::Write)?;
    require_visible(&s, &caller, &id).await?;
    let version = s.repo.patch_planet_record(&id, &planet, &patch).await?;
    Ok(Json(VersionResponse { version }))
}

/// Deleting is deliberately admin-only. A shared write link is meant for
/// annotating a stage, and destroying rows for everyone else who holds it is a
/// larger power than that implies.
async fn delete_system(
    State(s): State<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    caller.require(Capability::Admin)?;
    s.repo.delete_system(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Settings are per grant, so everyone sharing a link shares a cursor and two
/// separate links do not fight over one. Read-only links may still set them:
/// where you are looking is not a change to the vault.
async fn put_settings(
    State(s): State<AppState>,
    caller: Caller,
    Json(settings): Json<Settings>,
) -> Result<StatusCode, Failure> {
    caller.require(Capability::Read)?;
    s.repo.save_settings(caller.settings_key(), &settings).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Populate an empty vault with the shipped neighbourhood.
///
/// Server-side rather than client-side: with several clients starting at once,
/// each seeding independently would race. Here it is one guarded operation, and
/// by default it refuses to run against a vault that already has content.
async fn seed(
    State(s): State<AppState>,
    caller: Caller,
    Json(req): Json<SeedRequest>,
) -> Result<Json<serde_json::Value>, Failure> {
    caller.require(Capability::Admin)?;
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

    // Identity used to come from an `X-Parallax-User` header, which any client
    // could set to anything. It has been replaced by capability links, so the
    // header tests that lived here are gone rather than adapted: there is no
    // longer a caller-supplied identity to test.
    //
    // Enforcement is exercised end to end in `tests/auth.rs`, over real HTTP
    // against real PostgreSQL, because filtering that only holds in a unit test
    // is not access control.

    #[test]
    fn the_mint_request_defaults_to_the_least_privilege() {
        // A body that omits `capability` must not quietly produce a writable
        // link.
        let req: MintRequest =
            serde_json::from_str(r#"{"scope":{"kind":"all"}}"#).expect("parse");
        assert_eq!(req.capability, Capability::Read);
        assert!(req.expires_in_days.is_none());
        assert!(req.label.is_empty());
    }

    #[test]
    fn a_scope_round_trips_through_the_wire_form() {
        let req: MintRequest = serde_json::from_str(
            r#"{"capability":"write","scope":{"kind":"systems","ids":["sol","gj-1061"]}}"#,
        )
        .expect("parse");
        assert_eq!(req.capability, Capability::Write);
        assert_eq!(req.scope.system_ids(), ["sol".to_string(), "gj-1061".to_string()]);

        let tag: MintRequest =
            serde_json::from_str(r#"{"scope":{"kind":"tag","tag":"habitable-zone"}}"#)
                .expect("parse");
        assert_eq!(tag.scope.tag(), Some("habitable-zone"));
    }

    #[test]
    fn an_unknown_capability_is_refused_rather_than_defaulted() {
        assert!(serde_json::from_str::<MintRequest>(
            r#"{"capability":"root","scope":{"kind":"all"}}"#
        )
        .is_err());
    }
}
