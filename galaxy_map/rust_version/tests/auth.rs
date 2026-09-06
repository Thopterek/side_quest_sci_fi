//! Capability links, exercised against a live server.
//!
//! These are the tests that matter most in the whole suite: everything else
//! being wrong produces a bad drawing, whereas this being wrong hands someone
//! the vault. Each one goes over real HTTP against real PostgreSQL, because
//! filtering that only holds in a unit test is not access control.

#![cfg(all(feature = "server", feature = "db"))]

use std::sync::Arc;

use parallax::core::grant::{hash_token, Capability, Scope};
use parallax::server::{router, AppState, Repo};

fn url() -> Option<String> {
    std::env::var("PARALLAX_TEST_DATABASE_URL").ok()
}

/// Boot a server on an ephemeral port against a freshly seeded vault.
async fn serve() -> Option<(String, Arc<Repo>)> {
    let url = url()?;
    let repo = Repo::connect(&url, 4).expect("connect");
    repo.migrate().await.expect("migrate");
    repo.truncate_all().await.expect("truncate");
    repo.seed(true).await.expect("seed");

    let repo = Arc::new(repo);
    let (changes, _) = tokio::sync::broadcast::channel(16);
    // Without this the broadcast channel exists but nothing ever publishes to
    // it, and /events is an open stream that says nothing.
    parallax::server::listen::spawn(url.clone(), changes.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let base = format!("http://{addr}");
    let app = router(AppState {
        repo: repo.clone(),
        changes,
        base_url: base.clone(),
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Some((base, repo))
}

/// Every test body goes through here, so a missing database skips rather than
/// fails and a clean checkout stays green.
macro_rules! api_test {
    ($name:ident, |$base:ident, $repo:ident| $body:block) => {
        #[tokio::test]
        async fn $name() {
            let Some(($base, $repo)) = serve().await else {
                eprintln!("skipped: PARALLAX_TEST_DATABASE_URL not set");
                return;
            };
            let _ = &$repo;
            $body
        }
    };
}

struct Res {
    status: u16,
    body: serde_json::Value,
}

async fn req(
    method: &str,
    url: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> Res {
    let c = reqwest::Client::new();
    let mut b = c.request(method.parse().unwrap(), url);
    if let Some(t) = token {
        b = b.bearer_auth(t);
    }
    if let Some(j) = body {
        b = b.json(&j);
    }
    let r = b.send().await.expect("request");
    let status = r.status().as_u16();
    let body = r.json().await.unwrap_or(serde_json::Value::Null);
    Res { status, body }
}

async fn get(url: &str, token: Option<&str>) -> Res {
    req("GET", url, token, None).await
}

fn system_ids(v: &serde_json::Value) -> Vec<String> {
    v["systems"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s["system"]["id"].as_str().or_else(|| s["id"].as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Mint a link directly through the repo, standing in for an admin who already
/// has one.
async fn mint(repo: &Repo, cap: Capability, scope: Scope) -> String {
    repo.mint_grant("test", cap, &scope, None).await.expect("mint").token
}

/* ------------------------------------------------------------- the stage -- */

api_test!(a_first_time_visitor_sees_only_the_solar_system, |base, repo| {
    let r = get(&format!("{base}/vault"), None).await;
    assert_eq!(r.status, 200, "arriving with no link must not be an error");
    let ids = system_ids(&r.body);
    assert_eq!(ids, vec!["sol".to_string()], "the lonely solar system, and nothing else");
});

api_test!(an_explicit_stage_shows_its_systems_plus_the_origin, |base, repo| {
    let token = mint(
        &repo,
        Capability::Read,
        Scope::Systems { ids: vec!["gj-1061".into(), "trappist-1".into()] },
    )
    .await;
    let ids = system_ids(&get(&format!("{base}/vault"), Some(&token)).await.body);
    let mut sorted = ids.clone();
    sorted.sort();
    // Sol is always present: it is the origin every distance is measured from,
    // so a cube without it has no anchor. A landmark, not a secret.
    assert_eq!(sorted, vec!["gj-1061", "sol", "trappist-1"]);
});

api_test!(a_tag_stage_is_curated_from_the_notes_themselves, |base, repo| {
    let token = mint(&repo, Capability::Read, Scope::Tag { tag: "habitable-zone".into() }).await;
    let ids = system_ids(&get(&format!("{base}/vault"), Some(&token)).await.body);
    assert!(ids.contains(&"gj-1061".to_string()));
    assert!(ids.contains(&"proxima-centauri".to_string()));
    assert!(!ids.contains(&"gj-367".to_string()), "not tagged, so not in the stage");
    assert!(ids.len() > 2 && ids.len() < 13);
});

api_test!(an_admin_link_sees_the_whole_vault, |base, repo| {
    let token = mint(&repo, Capability::Admin, Scope::All).await;
    assert_eq!(system_ids(&get(&format!("{base}/vault"), Some(&token)).await.body).len(), 13);
});

api_test!(withheld_systems_never_reach_the_wire, |base, repo| {
    // Filtering in the client would leave the data in the response for anyone
    // who opened the network tab. Check the raw bytes, not the parsed ids.
    let token = mint(&repo, Capability::Read, Scope::Systems { ids: vec!["gj-1061".into()] }).await;
    let raw = reqwest::Client::new()
        .get(format!("{base}/vault"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request")
        .text()
        .await
        .expect("body");
    assert!(raw.contains("GJ 1061"), "the stage's own content must be present");

    // Assert on values that could only come from a withheld row. Naming a
    // withheld system is not proof of a leak, because a visible note may
    // legitimately mention one — see the test below.
    for fingerprint in [
        "GJ 699",       // Barnard's Star, catalogue name
        "M8 V",         // TRAPPIST-1's spectral type
        "12.467",       // TRAPPIST-1's distance
        "Lalande",      // GJ 411's dossier
        "0.06189",      // TRAPPIST-1 h's semi-major axis
    ] {
        assert!(
            !raw.contains(fingerprint),
            "a withheld system leaked into the response: {fingerprint}"
        );
    }
});

api_test!(a_visible_note_may_still_name_a_system_outside_the_stage, |base, repo| {
    // Worth pinning as a known property rather than leaving it unexamined.
    //
    // GJ 1061's dossier contains the wikilink [[TRAPPIST-1]]. A stage
    // containing GJ 1061 therefore carries that *string*, even when TRAPPIST-1
    // itself is withheld. No row leaks — no coordinates, no planets, no
    // dossier — but the name does, because the host wrote it into a note they
    // chose to share.
    //
    // The fix is editorial, not technical: curate the notes that go into a
    // stage. Stripping unresolvable wikilinks would silently mangle the
    // operator's own text, which is worse.
    let token = mint(&repo, Capability::Read, Scope::Systems { ids: vec!["gj-1061".into()] }).await;
    let raw = reqwest::Client::new()
        .get(format!("{base}/vault"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request")
        .text()
        .await
        .expect("body");

    assert!(raw.contains("TRAPPIST-1"), "the wikilink is part of a note that is in scope");
    assert!(!raw.contains("M8 V"), "but no field of the withheld row travels with it");

    // And the link is inert: the target is not in the vault the client received.
    let ids = system_ids(&get(&format!("{base}/vault"), Some(&token)).await.body);
    assert!(!ids.contains(&"trappist-1".to_string()));
});

/* ------------------------------------------------------ links as secrets -- */

api_test!(a_token_that_does_not_resolve_is_refused_rather_than_downgraded, |base, repo| {
    // Silently falling back to the public stage would look like the link
    // worked and show the wrong thing.
    let r = get(&format!("{base}/vault"), Some("pxv_not-a-real-token")).await;
    assert_eq!(r.status, 401);
});

api_test!(a_revoked_link_stops_working_immediately, |base, repo| {
    let token = mint(&repo, Capability::Read, Scope::All).await;
    let url = format!("{base}/vault");
    assert_eq!(get(&url, Some(&token)).await.status, 200);

    let id = repo
        .grant_by_token_hash(&hash_token(&token))
        .await
        .expect("lookup")
        .expect("live")
        .id;
    assert!(repo.revoke_grant(&id).await.expect("revoke"));

    assert_eq!(get(&url, Some(&token)).await.status, 401, "revocation must take effect at once");
});

api_test!(the_plaintext_token_is_never_stored, |base, repo| {
    let token = mint(&repo, Capability::Read, Scope::All).await;
    let admin = mint(&repo, Capability::Admin, Scope::All).await;
    let listed = get(&format!("{base}/grants"), Some(&admin)).await;
    assert_eq!(listed.status, 200);
    let raw = serde_json::to_string(&listed.body).unwrap();
    assert!(!raw.contains(&token), "a token came back out of the database");
    assert!(!raw.contains("token_hash"), "the digest need not be exposed either");
});

api_test!(the_link_can_be_carried_in_the_query_so_it_is_shareable, |base, repo| {
    let token = mint(&repo, Capability::Read, Scope::Systems { ids: vec!["gj-1061".into()] }).await;
    let r = get(&format!("{base}/vault?t={token}"), None).await;
    assert_eq!(r.status, 200);
    assert!(system_ids(&r.body).contains(&"gj-1061".to_string()));
});

/* ------------------------------------------------------------ capability -- */

api_test!(a_read_link_cannot_edit_anything, |base, repo| {
    let token = mint(&repo, Capability::Read, Scope::All).await;
    let r = req(
        "PATCH",
        &format!("{base}/systems/gj-1061/record"),
        Some(&token),
        Some(serde_json::json!({ "imperial_name": "Trespass" })),
    )
    .await;
    assert_eq!(r.status, 403);
    // And the vault is genuinely unchanged, not merely reported as such.
    let after = repo.snapshot("anonymous").await.expect("snapshot");
    assert!(after.systems.iter().all(|s| s.system.record.imperial_name != "Trespass"));
});

api_test!(a_write_link_can_edit_inside_its_stage, |base, repo| {
    let token = mint(&repo, Capability::Write, Scope::Systems { ids: vec!["gj-1061".into()] }).await;
    let r = req(
        "PATCH",
        &format!("{base}/systems/gj-1061/record"),
        Some(&token),
        Some(serde_json::json!({ "imperial_name": "Kestrel Reach" })),
    )
    .await;
    assert_eq!(r.status, 200, "{:?}", r.body);
});

api_test!(a_write_link_cannot_edit_outside_its_stage, |base, repo| {
    // The interesting case: the capability is sufficient but the scope is not.
    let token = mint(&repo, Capability::Write, Scope::Systems { ids: vec!["gj-1061".into()] }).await;
    let r = req(
        "PATCH",
        &format!("{base}/systems/trappist-1/record"),
        Some(&token),
        Some(serde_json::json!({ "imperial_name": "Trespass" })),
    )
    .await;
    assert_eq!(r.status, 404, "and 404 rather than 403, so ids cannot be enumerated");
});

api_test!(only_an_admin_link_may_delete_or_mint, |base, repo| {
    let writer = mint(&repo, Capability::Write, Scope::All).await;
    assert_eq!(
        req("DELETE", &format!("{base}/systems/gj-1061"), Some(&writer), None).await.status,
        403,
        "a shared write link must not destroy rows for everyone else holding it"
    );
    let r = req(
        "POST",
        &format!("{base}/grants"),
        Some(&writer),
        Some(serde_json::json!({ "capability": "admin", "scope": { "kind": "all" } })),
    )
    .await;
    assert_eq!(r.status, 403, "a write link must not be able to promote itself");
});

api_test!(an_admin_mints_a_link_and_gets_it_exactly_once, |base, repo| {
    let admin = mint(&repo, Capability::Admin, Scope::All).await;
    let r = req(
        "POST",
        &format!("{base}/grants"),
        Some(&admin),
        Some(serde_json::json!({
            "label": "Public tour",
            "capability": "read",
            "scope": { "kind": "systems", "ids": ["gj-1061", "trappist-1"] }
        })),
    )
    .await;
    assert_eq!(r.status, 200, "{:?}", r.body);

    let token = r.body["token"].as_str().expect("a token").to_string();
    let link = r.body["link"].as_str().expect("a link");
    assert!(link.ends_with(&token), "the link must carry the token");
    assert!(link.contains("/v/"));

    // And it works, with exactly the stage that was asked for.
    let mut ids = system_ids(&get(&format!("{base}/vault"), Some(&token)).await.body);
    ids.sort();
    assert_eq!(ids, vec!["gj-1061", "sol", "trappist-1"]);
});

api_test!(a_restricted_admin_link_cannot_be_minted, |base, repo| {
    // It could simply mint itself an unrestricted one, so the restriction
    // would be decorative.
    let admin = mint(&repo, Capability::Admin, Scope::All).await;
    let r = req(
        "POST",
        &format!("{base}/grants"),
        Some(&admin),
        Some(serde_json::json!({
            "capability": "admin",
            "scope": { "kind": "systems", "ids": ["sol"] }
        })),
    )
    .await;
    assert!(r.status >= 400, "expected a refusal, got {}", r.status);
});

api_test!(the_boundary_cannot_be_redirected_by_a_hostile_search_path, |base, repo| {
    // `visible_systems` is the access boundary. A SQL function that resolves
    // names through the caller's search_path can be pointed at a table the
    // caller controls — a standard PostgreSQL escalation. Both it and
    // `live_grant` pin `search_path`, and this proves the pin holds.
    let _ = base;
    repo.raw_batch(
        "create schema if not exists evil;
         create table if not exists evil.systems (id text, origin boolean);
         insert into evil.systems values ('everything', true);",
    )
    .await
    .expect("stage the hostile table");

    let seen = repo
        .visible_ids_with_search_path("anonymous", "evil, public")
        .await
        .expect("call the boundary");

    assert_eq!(seen, vec!["sol".to_string()], "the boundary was redirected");

    repo.raw_batch("drop schema evil cascade;").await.ok();
});

/* ------------------------------------------------------------- the view -- */

api_test!(whoami_tells_the_face_what_to_offer, |base, repo| {
    let anon = get(&format!("{base}/whoami"), None).await;
    assert_eq!(anon.body["anonymous"], true);
    assert_eq!(anon.body["can_write"], false);

    let token = mint(&repo, Capability::Write, Scope::Tag { tag: "compact".into() }).await;
    let who = get(&format!("{base}/whoami"), Some(&token)).await;
    assert_eq!(who.body["can_write"], true);
    assert_eq!(who.body["can_administer"], false);
    assert_eq!(who.body["scope"]["kind"], "tag");
    assert_eq!(who.body["scope"]["tag"], "compact");
});

api_test!(two_links_do_not_share_a_cursor, |base, repo| {
    let a = mint(&repo, Capability::Read, Scope::All).await;
    let b = mint(&repo, Capability::Read, Scope::All).await;
    for (t, sel) in [(&a, "gj-1061"), (&b, "trappist-1")] {
        let r = req(
            "PUT",
            &format!("{base}/settings"),
            Some(t),
            Some(serde_json::json!({ "selected": sel })),
        )
        .await;
        assert_eq!(r.status, 204, "{:?}", r.body);
    }
    let sa = get(&format!("{base}/vault"), Some(&a)).await;
    let sb = get(&format!("{base}/vault"), Some(&b)).await;
    assert_eq!(sa.body["settings"]["selected"], "gj-1061");
    assert_eq!(sb.body["settings"]["selected"], "trappist-1");
});

api_test!(a_read_link_may_still_move_its_own_cursor, |base, repo| {
    // Where you are looking is not a change to the vault.
    let token = mint(&repo, Capability::Read, Scope::All).await;
    let r = req(
        "PUT",
        &format!("{base}/settings"),
        Some(&token),
        Some(serde_json::json!({ "selected": "sol" })),
    )
    .await;
    assert_eq!(r.status, 204);
});

/* ------------------------------------------------------- live propagation -- */

#[cfg(feature = "client")]
api_test!(the_face_learns_of_another_operators_edit_without_polling, |base, repo| {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    let token = mint(&repo, Capability::Write, Scope::All).await;

    // Stand in for the face: a listener on the SSE stream.
    let hits = StdArc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let listener = parallax::client::ChangeListener::spawn(&base, Some(token.clone()), move || {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    // Give the stream time to attach before changing anything.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing has changed yet");

    // Another operator edits the vault.
    let r = req(
        "PATCH",
        &format!("{base}/systems/gj-1061/record"),
        Some(&token),
        Some(serde_json::json!({ "imperial_name": "Kestrel Reach" })),
    )
    .await;
    assert_eq!(r.status, 200, "{:?}", r.body);

    // The listener should notice, after its coalescing window.
    let mut noticed = false;
    for _ in 0..40 {
        if hits.load(Ordering::SeqCst) > 0 {
            noticed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(noticed, "an edit by someone else never reached the face");

    drop(listener);
});

#[cfg(feature = "client")]
api_test!(a_burst_of_edits_coalesces_into_few_reloads, |base, repo| {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc as StdArc;

    let token = mint(&repo, Capability::Write, Scope::All).await;
    let hits = StdArc::new(AtomicUsize::new(0));
    let seen = hits.clone();
    let listener = parallax::client::ChangeListener::spawn(&base, Some(token.clone()), move || {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    // Twenty edits in quick succession, as typing into a dossier produces.
    for i in 0..20 {
        req(
            "PATCH",
            &format!("{base}/systems/gj-1061/record"),
            Some(&token),
            Some(serde_json::json!({ "notes": format!("draft {i}") })),
        )
        .await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    let n = hits.load(Ordering::SeqCst);
    assert!(n >= 1, "the burst must be noticed at all");
    assert!(n <= 8, "20 edits should not mean 20 reloads, got {n}");
    drop(listener);
});

/* ------------------------------------------------------------ share link -- */

api_test!(opening_a_share_link_in_a_browser_lands_somewhere_useful, |base, repo| {
    // Regression: `share_link` produced /v/<token> from the beginning, but no
    // route served it, so every link the host handed out answered 404.
    let token = mint(&repo, Capability::Read, Scope::Tag { tag: "habitable-zone".into() }).await;
    let link = parallax::core::grant::share_link(&base, &token);

    let r = reqwest::get(&link).await.expect("open the link");
    assert_eq!(r.status(), 200, "a live share link must not 404");
    let html = r.text().await.expect("body");

    assert!(html.contains("<!doctype html"), "a browser should get a page");
    assert!(html.contains(&token), "the page must hand over the token to paste");
    assert!(html.contains("PARALLAX_SERVER_URL"), "and say how to use it");
    assert!(html.contains("habitable-zone"), "and name the stage");
});

api_test!(a_revoked_link_says_so_rather_than_looking_like_a_dead_server, |base, repo| {
    let token = mint(&repo, Capability::Read, Scope::All).await;
    let id = repo
        .grant_by_token_hash(&hash_token(&token))
        .await
        .expect("lookup")
        .expect("live")
        .id;
    repo.revoke_grant(&id).await.expect("revoke");

    let r = reqwest::get(parallax::core::grant::share_link(&base, &token))
        .await
        .expect("open");
    assert_eq!(r.status(), 404);
    let html = r.text().await.expect("body");
    assert!(html.contains("not valid"), "the recipient should learn why");
});

api_test!(the_landing_page_escapes_a_label_rather_than_rendering_it, |base, repo| {
    let minted = repo
        .mint_grant(
            "<script>alert(1)</script>",
            Capability::Read,
            &Scope::All,
            None,
        )
        .await
        .expect("mint");
    let html = reqwest::get(parallax::core::grant::share_link(&base, &minted.token))
        .await
        .expect("open")
        .text()
        .await
        .expect("body");
    assert!(!html.contains("<script>alert"), "label was interpolated raw");
    assert!(html.contains("&lt;script&gt;"), "label should be escaped");
});

api_test!(the_base_url_explains_itself_instead_of_answering_404, |base, repo| {
    // Regression: opening http://host:8080/ met a bare 404, which is
    // indistinguishable from a container that never started.
    let _ = repo;
    let r = reqwest::get(&base).await.expect("open the base URL");
    assert_eq!(r.status(), 200, "the front door must not 404");
    let html = r.text().await.expect("body");
    assert!(html.contains("<!doctype html"));
    assert!(html.contains("server is running"), "it should say the server is up");
    assert!(html.contains("ADMIN LINK"), "and how the host gets in");
});

api_test!(the_index_discloses_nothing_beyond_the_public_stage, |base, repo| {
    // It is reachable without a token, so it must not leak the vault's size or
    // contents. The seed vault has 13 systems; anonymous sees 1.
    let _ = repo;
    let html = reqwest::get(&base).await.expect("open").text().await.expect("body");
    assert!(html.contains("<strong>1</strong>"), "should report the anonymous count");
    // Matched against the rendered count, not a bare "13": the stylesheet
    // contains `letter-spacing:.13em`, and asserting on the loose substring
    // failed on that rather than on any leak.
    assert!(!html.contains("<strong>13</strong>"), "must not disclose the full vault size");
    for withheld in ["TRAPPIST-1", "GJ 699", "Kestrel"] {
        assert!(!html.contains(withheld), "leaked {withheld}");
    }
});
