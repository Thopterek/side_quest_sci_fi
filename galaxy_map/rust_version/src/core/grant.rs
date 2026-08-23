//! Capability links, in place of accounts.
//!
//! The host mints a link and hands it to a person or a group; holding it is the
//! whole of the credential. There is no registration and no password, because
//! for a shared star chart the thing worth naming is the *stage* — which slice
//! of the vault you are looking at — not the viewer.
//!
//! Consequences worth being deliberate about:
//!
//! * A link is a bearer token. Anyone it is forwarded to has the same access,
//!   which is the intent for a group but means links should be treated as
//!   secrets and given expiries.
//! * Only the SHA-256 of a token is ever stored, so a database dump does not
//!   hand out working links. The plaintext exists once, in the response that
//!   created it, and cannot be recovered afterwards.
//! * Enforcement lives in SQL (`parallax.visible_systems`), not here. This
//!   module models grants; the database decides what they can see.

use serde::{Deserialize, Serialize};

/// What a grant may do. Ordered: `Admin` implies `Write` implies `Read`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// View its stage.
    Read,
    /// Also edit dossiers and add systems within its stage.
    Write,
    /// See everything and mint further grants.
    Admin,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::Read => "read",
            Capability::Write => "write",
            Capability::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Capability> {
        match s {
            "read" => Some(Capability::Read),
            "write" => Some(Capability::Write),
            "admin" => Some(Capability::Admin),
            _ => None,
        }
    }

    /// Does this grant permit `wanted`?
    pub fn allows(self, wanted: Capability) -> bool {
        self >= wanted
    }
}

/// Which slice of the vault a grant can see.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// The whole vault.
    All,
    /// An explicit list of system ids.
    Systems { ids: Vec<String> },
    /// Everything carrying a tag, so a stage can be curated from notes rather
    /// than maintained as a list that goes stale.
    Tag { tag: String },
}

impl Scope {
    pub fn kind(&self) -> &'static str {
        match self {
            Scope::All => "all",
            Scope::Systems { .. } => "systems",
            Scope::Tag { .. } => "tag",
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            Scope::Tag { tag } => Some(tag),
            _ => None,
        }
    }

    pub fn system_ids(&self) -> &[String] {
        match self {
            Scope::Systems { ids } => ids,
            _ => &[],
        }
    }

    /// Reconstruct from the three stored columns.
    pub fn from_columns(kind: &str, tag: Option<&str>, ids: Vec<String>) -> Option<Scope> {
        match kind {
            "all" => Some(Scope::All),
            "systems" => Some(Scope::Systems { ids }),
            "tag" => tag
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
                .map(|t| Scope::Tag { tag: t.to_lowercase() }),
            _ => None,
        }
    }

    /// Reject combinations the database would refuse, with a readable reason.
    pub fn validate(&self, capability: Capability) -> Result<(), &'static str> {
        if capability == Capability::Admin && !matches!(self, Scope::All) {
            // An admin restricted to a subset could mint an unrestricted grant
            // for itself, so the restriction would be decorative.
            return Err("an admin grant must have scope 'all'");
        }
        if let Scope::Tag { tag } = self {
            if tag.trim().is_empty() {
                return Err("a tag-scoped grant needs a tag");
            }
        }
        Ok(())
    }
}

/// A grant as stored. Never carries the plaintext token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub label: String,
    pub capability: Capability,
    pub scope: Scope,
    /// RFC 3339, or `None` for no expiry.
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub use_count: i64,
    pub last_used_at: Option<String>,
}

impl Grant {
    /// The grant every visitor gets before presenting anything.
    pub const ANONYMOUS_ID: &'static str = "anonymous";

    pub fn is_anonymous(&self) -> bool {
        self.id == Self::ANONYMOUS_ID
    }

    pub fn can(&self, wanted: Capability) -> bool {
        !self.revoked && self.capability.allows(wanted)
    }
}

/// A freshly minted grant. The token is present exactly once, here.
#[derive(Clone, Debug, Serialize)]
pub struct MintedGrant {
    pub grant: Grant,
    /// Show this to the host once. It cannot be recovered.
    pub token: String,
}

/* ------------------------------------------------------------------ tokens */

/// Prefix on every token, so one is recognisable on sight in a log or a paste
/// and can be matched by secret scanners.
pub const TOKEN_PREFIX: &str = "pxv_";

/// 32 bytes of entropy, hex encoded. Long enough that guessing is not a
/// consideration and short enough to paste.
pub fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the OS must provide randomness");
    let mut out = String::with_capacity(TOKEN_PREFIX.len() + 64);
    out.push_str(TOKEN_PREFIX);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Hex SHA-256 of a token. This is the only form that reaches the database.
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(token.trim().as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Pull a token out of an `Authorization: Bearer …` header or a `?t=` query.
///
/// Both are supported deliberately: the header is correct, and the query
/// parameter is what makes a shareable *link* possible at all.
pub fn extract_token(auth_header: Option<&str>, query_token: Option<&str>) -> Option<String> {
    if let Some(h) = auth_header {
        let h = h.trim();
        if let Some(rest) = h.strip_prefix("Bearer ").or_else(|| h.strip_prefix("bearer ")) {
            let t = rest.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    query_token.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

/// The link to hand out for a token.
pub fn share_link(base_url: &str, token: &str) -> String {
    format!("{}/v/{}", base_url.trim_end_matches('/'), token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_are_ordered_so_admin_implies_the_rest() {
        assert!(Capability::Admin.allows(Capability::Read));
        assert!(Capability::Admin.allows(Capability::Write));
        assert!(Capability::Write.allows(Capability::Read));
        assert!(!Capability::Read.allows(Capability::Write));
        assert!(!Capability::Write.allows(Capability::Admin));
    }

    #[test]
    fn capability_round_trips_through_its_stored_form() {
        for c in [Capability::Read, Capability::Write, Capability::Admin] {
            assert_eq!(Capability::parse(c.as_str()), Some(c));
        }
        assert_eq!(Capability::parse("root"), None);
    }

    #[test]
    fn tokens_are_unique_prefixed_and_long() {
        let a = mint_token();
        let b = mint_token();
        assert_ne!(a, b, "two mints must not collide");
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(a.len(), TOKEN_PREFIX.len() + 64);
        assert!(a[TOKEN_PREFIX.len()..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hashing_is_stable_and_does_not_reveal_the_token() {
        let t = mint_token();
        assert_eq!(hash_token(&t), hash_token(&t));
        assert_ne!(hash_token(&t), t);
        assert_eq!(hash_token(&t).len(), 64);
        // Surrounding whitespace from a sloppy paste must not change identity.
        assert_eq!(hash_token(&t), hash_token(&format!("  {t}\n")));
        assert_ne!(hash_token("a"), hash_token("b"));
    }

    #[test]
    fn a_known_vector_pins_the_hash_function() {
        // If this changes, every stored grant is invalidated, so it should be a
        // deliberate migration rather than an accident.
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_bearer_header_is_accepted_in_either_case() {
        assert_eq!(extract_token(Some("Bearer pxv_x"), None).as_deref(), Some("pxv_x"));
        assert_eq!(extract_token(Some("bearer pxv_x"), None).as_deref(), Some("pxv_x"));
        assert_eq!(extract_token(Some("  Bearer  pxv_x  "), None).as_deref(), Some("pxv_x"));
    }

    #[test]
    fn a_query_token_makes_the_link_shareable() {
        assert_eq!(extract_token(None, Some("pxv_y")).as_deref(), Some("pxv_y"));
        // The header wins when both are present.
        assert_eq!(extract_token(Some("Bearer a"), Some("b")).as_deref(), Some("a"));
    }

    #[test]
    fn nothing_presented_means_no_token_rather_than_an_empty_one() {
        assert_eq!(extract_token(None, None), None);
        assert_eq!(extract_token(Some(""), None), None);
        assert_eq!(extract_token(Some("Bearer   "), None), None);
        assert_eq!(extract_token(None, Some("  ")), None);
        // Not a bearer scheme; must not be mistaken for one.
        assert_eq!(extract_token(Some("Basic abc"), None), None);
    }

    #[test]
    fn scopes_round_trip_through_their_stored_columns() {
        let cases = [
            Scope::All,
            Scope::Systems { ids: vec!["sol".into(), "gj-1061".into()] },
            Scope::Tag { tag: "habitable-zone".into() },
        ];
        for s in cases {
            let back = Scope::from_columns(
                s.kind(),
                s.tag(),
                s.system_ids().to_vec(),
            );
            assert_eq!(back.as_ref(), Some(&s));
        }
    }

    #[test]
    fn tag_scopes_are_normalised_to_lower_case() {
        let s = Scope::from_columns("tag", Some("Habitable-Zone"), vec![]).unwrap();
        assert_eq!(s, Scope::Tag { tag: "habitable-zone".into() });
    }

    #[test]
    fn a_restricted_admin_is_rejected() {
        // Otherwise it could simply mint itself an unrestricted grant.
        let scope = Scope::Systems { ids: vec!["sol".into()] };
        assert!(scope.validate(Capability::Admin).is_err());
        assert!(scope.validate(Capability::Write).is_ok());
        assert!(Scope::All.validate(Capability::Admin).is_ok());
    }

    #[test]
    fn an_empty_tag_scope_is_rejected_rather_than_silently_empty() {
        assert!(Scope::Tag { tag: "  ".into() }.validate(Capability::Read).is_err());
        assert_eq!(Scope::from_columns("tag", Some(""), vec![]), None);
        assert_eq!(Scope::from_columns("tag", None, vec![]), None);
    }

    #[test]
    fn share_links_are_well_formed_regardless_of_trailing_slash() {
        assert_eq!(share_link("https://h.example", "pxv_a"), "https://h.example/v/pxv_a");
        assert_eq!(share_link("https://h.example/", "pxv_a"), "https://h.example/v/pxv_a");
    }

    #[test]
    fn a_revoked_grant_can_do_nothing_whatever_its_capability() {
        let g = Grant {
            id: "g".into(),
            label: String::new(),
            capability: Capability::Admin,
            scope: Scope::All,
            expires_at: None,
            revoked: true,
            use_count: 0,
            last_used_at: None,
        };
        assert!(!g.can(Capability::Read));
        assert!(!g.can(Capability::Admin));
    }
}
