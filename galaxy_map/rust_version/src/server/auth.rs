//! Turning a share link into a stage.
//!
//! Two rules the rest of the server depends on:
//!
//! 1. **A missing or unusable token is anonymous, not an error.** Someone
//!    arriving at the address with no link should see the lonely solar system,
//!    not a 401. Refusing entry would be the classical model this deliberately
//!    is not.
//! 2. **Presenting a token that does not resolve *is* an error.** A revoked or
//!    mistyped link silently downgrading to the public stage would look like the
//!    link "worked" and show the wrong thing, which is worse than being told.

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use axum::http::StatusCode;
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::grant::{extract_token, hash_token, Capability, Grant};

use super::repo::{Repo, RepoError};

/// The resolved caller, attached to every request.
#[derive(Clone, Debug)]
pub struct Caller {
    pub grant: Grant,
}

impl Caller {
    pub fn id(&self) -> &str {
        &self.grant.id
    }

    /// Settings are per grant, so a shared link shares a cursor and two
    /// separate links do not fight over one.
    pub fn settings_key(&self) -> &str {
        &self.grant.id
    }

    pub fn can(&self, wanted: Capability) -> bool {
        self.grant.can(wanted)
    }

    /// `Err` carries the status and a message suitable for a client.
    pub fn require(&self, wanted: Capability) -> Result<(), AuthError> {
        if self.can(wanted) {
            return Ok(());
        }
        Err(AuthError::Forbidden {
            have: self.grant.capability,
            need: wanted,
            anonymous: self.grant.is_anonymous(),
        })
    }
}

#[derive(Debug)]
pub enum AuthError {
    /// A token was presented and did not resolve to a live grant.
    UnknownToken,
    Forbidden { have: Capability, need: Capability, anonymous: bool },
    Backend(String),
}

impl axum::response::IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            AuthError::UnknownToken => (
                StatusCode::UNAUTHORIZED,
                "that link is not valid — it may have been revoked or have expired".to_string(),
            ),
            AuthError::Forbidden { have, need, anonymous } => (
                StatusCode::FORBIDDEN,
                if anonymous {
                    format!("this needs {} access; open the vault with a share link", need.as_str())
                } else {
                    format!(
                        "this link grants {} access and this needs {}",
                        have.as_str(),
                        need.as_str()
                    )
                },
            ),
            AuthError::Backend(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        (status, axum::Json(serde_json::json!({ "error": message }))).into_response()
    }
}

impl From<RepoError> for AuthError {
    fn from(e: RepoError) -> Self {
        AuthError::Backend(e.to_string())
    }
}

/// Axum extractor. Put `caller: Caller` in a handler and it is resolved for you,
/// which means a handler cannot forget to authenticate — the worst it can do is
/// forget to *authorise*, and the capability checks are explicit for that.
impl<S> FromRequestParts<S> for Caller
where
    S: Send + Sync,
    Arc<Repo>: axum::extract::FromRef<S>,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        use axum::extract::FromRef;
        let repo: Arc<Repo> = Arc::from_ref(state);

        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let query: HashMap<String, String> = Query::<HashMap<String, String>>::try_from_uri(&parts.uri)
            .map(|q| q.0)
            .unwrap_or_default();

        let token = extract_token(header.as_deref(), query.get("t").map(String::as_str));

        match token {
            // No link presented: the public stage.
            None => Ok(Caller { grant: repo.anonymous_grant().await? }),
            Some(t) => {
                // Only the digest is ever compared, and the lookup is by unique
                // index, so nothing here is timing-sensitive in the token.
                match repo.grant_by_token_hash(&hash_token(&t)).await? {
                    Some(grant) => {
                        // Detached deliberately. Awaiting it put a second round
                        // trip on the critical path of every request, and the
                        // bookkeeping it does is not worth delaying a response
                        // for. Failures are ignored for the same reason: a
                        // statistic must never be able to deny access.
                        let (repo, id) = (repo.clone(), grant.id.clone());
                        tokio::spawn(async move {
                            let _ = repo.touch_grant(&id).await;
                        });
                        Ok(Caller { grant })
                    }
                    None => Err(AuthError::UnknownToken),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::grant::Scope;

    fn grant(id: &str, capability: Capability) -> Grant {
        Grant {
            id: id.into(),
            label: String::new(),
            capability,
            scope: Scope::All,
            expires_at: None,
            revoked: false,
            use_count: 0,
            last_used_at: None,
        }
    }

    #[test]
    fn a_read_link_cannot_write_or_administer() {
        let c = Caller { grant: grant("g", Capability::Read) };
        assert!(c.require(Capability::Read).is_ok());
        assert!(c.require(Capability::Write).is_err());
        assert!(c.require(Capability::Admin).is_err());
    }

    #[test]
    fn a_write_link_can_edit_but_not_mint_further_links() {
        let c = Caller { grant: grant("g", Capability::Write) };
        assert!(c.require(Capability::Write).is_ok());
        assert!(c.require(Capability::Admin).is_err());
    }

    #[test]
    fn the_anonymous_caller_is_told_to_use_a_link_rather_than_to_log_in() {
        let c = Caller { grant: grant(Grant::ANONYMOUS_ID, Capability::Read) };
        let Err(AuthError::Forbidden { anonymous, .. }) = c.require(Capability::Write) else {
            panic!("should be forbidden");
        };
        assert!(anonymous, "the message must point at share links, not accounts");
    }

    #[test]
    fn settings_are_keyed_per_grant_so_two_links_do_not_share_a_cursor() {
        let a = Caller { grant: grant("tour", Capability::Read) };
        let b = Caller { grant: grant("survey", Capability::Read) };
        assert_ne!(a.settings_key(), b.settings_key());
    }
}
