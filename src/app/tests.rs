use super::router::trace_path;

/// The scrobbling routes carry no operation-id protocol, and must not
/// advertise one.
///
/// `annotate_mutation_headers` injects a `409` and the two mutation headers
/// into every `user-data` write, because every one of those goes through
/// `mutation_context`. These do not — RFC-010 decision 12 keeps this state
/// out of synchronisation entirely — so they carry their own tag.
///
/// Written because removing a hand-written `409` from the retry route did
/// not remove the `409`: it vacated the entry that the injected one then
/// filled, with no response schema and a description about operation ids
/// that was false for it. The document was worse than before the fix, and
/// nothing said so.
#[test]
fn the_scrobbling_routes_advertise_no_operation_id_protocol() {
    use utoipa::openapi::RefOr;
    use utoipa::OpenApi;

    let openapi = super::ApiDoc::openapi();
    let mut checked = 0;
    for (path, item) in &openapi.paths.paths {
        if !path.starts_with("/api/v2/scrobble-") {
            continue;
        }
        for operation in [
            item.put.as_ref(),
            item.post.as_ref(),
            item.delete.as_ref(),
            item.patch.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            checked += 1;
            assert!(
                !operation.responses.responses.contains_key("409"),
                "{path} advertises a conflict it cannot produce"
            );
            let names: Vec<&str> = operation
                .parameters
                .iter()
                .flatten()
                .map(|parameter| match parameter {
                    RefOr::T(parameter) => parameter.name.as_str(),
                    RefOr::Ref(reference) => panic!(
                        "{path} advertises a parameter by reference ({}), which this check cannot read",
                        reference.ref_location
                    ),
                })
                .collect();
            assert!(
                !names.contains(&crate::api::OPERATION_ID_HEADER),
                "{path} advertises an operation id it never reads"
            );
            assert!(
                !names.contains(&crate::api::DEVICE_ID_HEADER),
                "{path} advertises a device id it never reads"
            );
        }
    }
    assert_eq!(
        checked, 5,
        "two on the link, two on the queue, and the Last.fm authorisation — a count that \
         falls if a route is added without deciding which protocol it belongs to"
    );
}

/// Every `user-data` mutation advertises both headers and a `409`.
///
/// Written because nothing asserted the presence of what
/// `annotate_mutation_headers` injects — only its absence on the
/// scrobbling routes. An annotation that stopped injecting would have made
/// that neighbouring test pass more easily, and the document would have
/// lost the operation-id protocol with a green suite.
///
/// `user-data` is the tag the annotation selects on, so this walk asks the
/// same question the pass answers rather than a wider one it never claimed.
#[test]
fn every_user_data_mutation_advertises_the_operation_id_protocol() {
    use utoipa::openapi::RefOr;
    use utoipa::OpenApi;

    // Sixteen today: the favourite, rating, play-queue and now-playing
    // mutations, the playlist import — which claims an operation like
    // every other playlist creation, because that is the verb it calls —
    // and the move and the removal of one playlist entry, which journal as
    // an update does. A new one lands here deliberately, not silently.
    let expected = 16;
    let openapi = super::ApiDoc::openapi();
    let mut checked = 0;
    for (path, item) in &openapi.paths.paths {
        for operation in [
            item.put.as_ref(),
            item.post.as_ref(),
            item.delete.as_ref(),
            item.patch.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !operation
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == "user-data"))
            {
                continue;
            }
            assert!(
                operation.responses.responses.contains_key("409"),
                "{path} takes an operation id without documenting the conflict it can answer"
            );
            let names: Vec<&str> = operation
                .parameters
                .iter()
                .flatten()
                .map(|parameter| match parameter {
                    RefOr::T(parameter) => parameter.name.as_str(),
                    RefOr::Ref(reference) => panic!(
                        "{path} advertises a parameter by reference ({}), which this check cannot read",
                        reference.ref_location
                    ),
                })
                .collect();
            assert!(
                names.contains(&crate::api::OPERATION_ID_HEADER),
                "{path} mutates without advertising an operation id"
            );
            assert!(
                names.contains(&crate::api::DEVICE_ID_HEADER),
                "{path} mutates without advertising a device id"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, expected,
        "{checked} tagged mutations examined, not {expected} — a walk that stops finding \
         them makes every assertion above vacuous, so the count is pinned rather than \
         merely non-zero"
    );
}

/// Every entry in `PUBLIC_OPERATIONS` names an operation that exists, and
/// leaves it carrying no security requirement.
///
/// The list is looked up by two strings, so a path that is renamed or a
/// method spelled `PUT` instead of `put` matches nothing, clears nothing,
/// and goes on looking correct. That was cheap while the result was only a
/// missing security block. `annotate_scope_refusals` reads what this pass
/// leaves behind, so a public operation that keeps the global requirement
/// is now handed a 403 it can never answer — a refusal documented on a
/// route that holds no credential to refuse.
#[test]
fn every_public_operation_is_found_and_cleared() {
    use utoipa::OpenApi;

    let openapi = super::ApiDoc::openapi();
    for (path, method) in super::openapi::PUBLIC_OPERATIONS {
        let item = openapi
            .paths
            .paths
            .get(*path)
            .unwrap_or_else(|| panic!("{path} is named here but is not a documented path"));
        let operation = match *method {
            "get" => item.get.as_ref(),
            "post" => item.post.as_ref(),
            "put" => item.put.as_ref(),
            "delete" => item.delete.as_ref(),
            "patch" => item.patch.as_ref(),
            other => panic!("{other} is not a method this list may name"),
        }
        .unwrap_or_else(|| panic!("{method} {path} is not a documented operation"));
        assert!(
            operation.security.as_ref().is_some_and(Vec::is_empty),
            "{method} {path} is public and must carry no security requirement"
        );
    }
}

/// Every entry in the table redacts, and no entry can shadow another.
///
/// A property over the table rather than one `assert_eq!` per entry,
/// because a hand-written assertion per entry has exactly the weakness the
/// table was written to fix: it cannot notice the entry nobody wrote. This
/// form also catches the edit that would quietly undo the whole thing — a
/// broad prefix inserted above a narrow one, which first-match-wins would
/// silently relabel.
#[test]
fn every_credential_bearing_prefix_is_redacted() {
    for (prefix, redacted) in super::router::CREDENTIAL_BEARING_PREFIXES {
        assert_eq!(
            redacted,
            format!("{prefix}{{redacted}}"),
            "{prefix} must replace its next segment and nothing else"
        );
        assert_eq!(
            trace_path(&format!("{prefix}a-sealed-secret")),
            redacted,
            "{prefix} must not reach a trace sink carrying its credential"
        );
        // A share token is followed by more path, and the whole tail goes.
        assert_eq!(
            trace_path(&format!("{prefix}a-sealed-secret/tracks/id/stream")),
            redacted
        );
    }

    for (outer, _) in super::router::CREDENTIAL_BEARING_PREFIXES {
        for (inner, _) in super::router::CREDENTIAL_BEARING_PREFIXES {
            assert!(
                outer == inner || !inner.starts_with(outer),
                "{outer} shadows {inner}: first match wins, so {inner} would never be reached"
            );
        }
    }

    // And a path that carries nothing is left exactly as it is.
    assert_eq!(trace_path("/rest/ping.view"), "/rest/ping.view");
}

/// Every public route whose path ends in a ticket has a redaction prefix.
///
/// **This is the assertion that would have caught the canvas ticket**, and
/// the test above would not have. A table is self-consistent on its own: an
/// entry with a typo in its prefix redacts its own typo and satisfies every
/// property there, while the real route goes on being logged. What no
/// property over the table alone can do is notice a route nobody added to
/// it.
///
/// `PUBLIC_OPERATIONS` had `/api/v2/canvas-stream/{ticket}` listed
/// correctly, two hundred lines from a redaction list that did not. The two
/// are tied together here so that distance stops mattering.
#[test]
fn every_public_ticket_route_has_a_redaction_prefix() {
    for (path, _) in super::openapi::PUBLIC_OPERATIONS {
        // `{state}` as well as `{ticket}`: the Last.fm return is the first
        // public route whose path carries a credential under another name,
        // and a check that knew only one spelling would have let it through
        // — which is the shape of the canvas ticket this test was written
        // for.
        let Some(prefix) = path
            .strip_suffix("{ticket}")
            .or_else(|| path.strip_suffix("{state}"))
        else {
            continue;
        };
        assert!(
            super::router::CREDENTIAL_BEARING_PREFIXES
                .iter()
                .any(|(candidate, _)| *candidate == prefix),
            "{path} carries a credential in its path and nothing redacts it"
        );
    }
}

/// A canvas size the platform cannot represent falls back to a finite ceiling.
///
/// `Config::from_env` refuses such a size, but a `Config` built by hand never
/// meets that check. The conversion used to fall back to `usize::MAX`, which
/// is no ceiling at all on the one route that takes a whole video in a body.
#[test]
fn a_canvas_size_that_does_not_fit_keeps_a_finite_ceiling() {
    use super::state::canvas_body_limit;
    use crate::config::CanvasLimits;

    let limits = |max_bytes| CanvasLimits {
        max_bytes,
        max_duration_secs: 8,
        library_quota_bytes: i64::MAX,
    };
    assert_eq!(canvas_body_limit(&limits(256 * 1024)), 256 * 1024);
    assert_eq!(canvas_body_limit(&limits(-1)), 4 * 1024 * 1024);
}
