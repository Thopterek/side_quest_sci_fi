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
        // The front door. Someone who opens the base URL in a browser should
        // learn that the server is up and what to do next, rather than meeting
        // a bare 404 that is indistinguishable from a dead container.
        .route("/", get(index))
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
        // The share link itself. `share_link` has always produced /v/<token>,
        // but nothing served it, so every link the host handed out answered 404.
        .route("/v/{token}", get(open_link))
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

/* ----------------------------------------------------------------- index */

/// What `GET /` says.
///
/// Deliberately reachable without a token, so it must disclose nothing beyond
/// the anonymous stage. The count it shows is that stage's, which is public by
/// definition.
async fn index(State(s): State<AppState>) -> axum::response::Html<String> {
    let public = match s.repo.anonymous_grant().await {
        Ok(g) => s.repo.snapshot(&g.id).await.map(|v| v.systems.len()).unwrap_or(0),
        Err(_) => 0,
    };
    let base = escape(&s.base_url);
    axum::response::Html(page(
        "The server is running",
        &format!(
            "<p>This is the Parallax vault server. It serves an API and share \
             links; the map itself is a desktop application.</p>\
             <p class=m>The public stage holds <strong>{public}</strong> \
             system(s). A share link widens that.</p>\
             <h2>If you have a share link</h2>\
             <p>Open it. It looks like <code>{base}/v/pxv_&hellip;</code> and \
             will say what it grants.</p>\
             <h2>If you are the host</h2>\
             <p>An admin link was printed once, the first time this server \
             started against an empty vault:</p>\
             <pre>docker compose logs server | grep \"ADMIN LINK\"</pre>\
             <p class=m>PowerShell: <code>| Select-String \"pxv_\"</code></p>\
             <h2>Endpoints</h2>\
             <p><a href=\"/health\">/health</a> &middot; \
             <a href=\"/vault\">/vault</a> &mdash; the public stage, as JSON</p>"
        ),
    ))
}

/* ------------------------------------------------------------ share link */

/// Minimal HTML escaping for the few values interpolated below.
///
/// The label is written by an admin rather than a stranger, but it reaches this
/// page from the database and a stage may be shared widely, so it is escaped
/// rather than trusted.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// What opening a share link in a browser does.
///
/// The face is a native application, so this cannot simply *be* the vault. It
/// is the landing page for a link: it confirms the link is live, says plainly
/// what stage it grants and whether it can edit, and hands over the token and
/// the two ways to use it. Ending at a 404 — which is what happened before this
/// route existed — gives the recipient no way to tell a revoked link from a
/// server that was never listening.
async fn open_link(
    State(s): State<AppState>,
    Path(token): Path<String>,
) -> axum::response::Response {
    let grant = s
        .repo
        .grant_by_token_hash(&crate::core::grant::hash_token(&token))
        .await
        .ok()
        .flatten();

    let Some(grant) = grant else {
        return (
            StatusCode::NOT_FOUND,
            axum::response::Html(page(
                "This link is not valid",
                "<p>It may have been revoked, or it may have expired. \
                 Ask whoever shared it for a new one.</p>",
            )),
        )
            .into_response();
    };

    // Opening the link counts as picking it up.
    let (repo, id) = (s.repo.clone(), grant.id.clone());
    tokio::spawn(async move {
        let _ = repo.touch_grant(&id).await;
    });

    let visible = s
        .repo
        .snapshot(&grant.id)
        .await
        .map(|v| v.systems.len())
        .unwrap_or(0);

    let stage = match &grant.scope {
        Scope::All => "the whole vault".to_string(),
        Scope::Systems { ids } => format!("{} named system(s)", ids.len()),
        Scope::Tag { tag } => format!("everything tagged <code>#{}</code>", escape(tag)),
    };
    let label = if grant.label.trim().is_empty() {
        "Shared stage".to_string()
    } else {
        escape(&grant.label)
    };
    let expiry = match &grant.expires_at {
        Some(t) => format!("<p class=m>Expires {}.</p>", escape(t)),
        None => "<p class=m>No expiry.</p>".to_string(),
    };
    let may = if grant.can(Capability::Admin) {
        "view everything, edit it, and mint further links"
    } else if grant.can(Capability::Write) {
        "view and edit this stage"
    } else {
        "view this stage"
    };
    let tok = escape(&token);

    let body = format!(
        "<p>This link lets you {may}. It currently shows \
         <strong>{visible}</strong> system(s): {stage}.</p>\
         {expiry}\
         <h2>Open it in Parallax</h2>\
         <pre>PARALLAX_SERVER_URL={base}\nPARALLAX_TOKEN={tok}</pre>\
         <p class=m>Set those, then run the Parallax desktop application.</p>\
         <h2>Or read it directly</h2>\
         <p><a href=\"/vault?t={tok}\">/vault?t=…</a> returns this stage as JSON.</p>",
        base = escape(&s.base_url),
    );
    axum::response::Html(page(&label, &body)).into_response()
}

/// One small stylesheet, shared by both outcomes. Deliberately self-contained:
/// this page has to render for someone who has been sent a URL and nothing else.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8>\
         <meta name=viewport content=\"width=device-width,initial-scale=1\">\
         <title>{title} · Parallax</title>\
         <style>\
         body{{background:#E4E4DA;color:#16181A;font:15px/1.5 system-ui,sans-serif;\
         margin:0;padding:3rem 1.5rem;display:flex;justify-content:center}}\
         main{{max-width:34rem;width:100%}}\
         h1{{font-size:1.4rem;letter-spacing:.02em;margin:0 0 .25rem}}\
         h2{{font-size:.75rem;letter-spacing:.16em;text-transform:uppercase;\
         color:#6D7276;margin:2rem 0 .5rem;border-bottom:1px solid #B3B4A6;\
         padding-bottom:.35rem}}\
         .m{{color:#6D7276;font-size:.85rem}}\
         code,pre{{font-family:ui-monospace,monospace;font-size:.8rem}}\
         pre{{background:#DBDBD0;border:1px solid #B3B4A6;padding:.75rem;\
         overflow-x:auto;white-space:pre-wrap;word-break:break-all}}\
         a{{color:#1F3F9E}}\
         .b{{display:inline-block;font-size:.7rem;letter-spacing:.13em;\
         text-transform:uppercase;border:1px solid #1F3F9E;color:#1F3F9E;\
         padding:.1rem .4rem;border-radius:2px;margin-bottom:1rem}}\
         </style>\
         <main><span class=b>Parallax</span><h1>{title}</h1>{body}</main>"
    )
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
