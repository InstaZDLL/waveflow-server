use super::*;

fn version(raw: &str) -> semver::Version {
    semver::Version::parse(raw).unwrap()
}

fn manifest(raw_version: &str) -> Manifest {
    Manifest {
        version: raw_version.to_owned(),
        tag: format!("v{raw_version}"),
        published_at: "2026-11-06T06:53:33Z".to_owned(),
        page: "https://github.com/InstaZDLL/waveflow-server/releases/tag/v2.1.2".to_owned(),
        notes: Some(
            "https://download.waveflow.app/server/releases/v2.1.2/RELEASE-NOTES.md".to_owned(),
        ),
    }
}

#[test]
fn a_pre_release_follows_beta_and_a_release_follows_stable() {
    assert_eq!(Channel::of(&version("2.1.2")), Channel::Stable);
    assert_eq!(Channel::of(&version("2.2.0-rc.1")), Channel::Beta);
    assert_eq!(Channel::of(&version("2.0.0-beta.1")), Channel::Beta);
}

#[test]
fn newer_follows_semantic_versioning_precedence() {
    // The order the release script holds itself to, so the two agree.
    let ordered = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "2.1.0-rc.1",
        "2.1.0",
        "2.1.2",
        "2.10.0",
    ];
    for pair in ordered.windows(2) {
        assert!(is_newer(&version(pair[1]), &version(pair[0])), "{pair:?}");
        assert!(!is_newer(&version(pair[0]), &version(pair[1])), "{pair:?}");
    }
    assert!(!is_newer(&version("2.1.2"), &version("2.1.2")));
    // Build metadata is not precedence: the same version, built twice.
    assert!(!is_newer(&version("2.1.2+b.2"), &version("2.1.2+b.1")));
}

#[test]
fn a_coherent_manifest_is_accepted() {
    assert_eq!(
        validate(&manifest("2.1.2"), Channel::Stable),
        Ok(version("2.1.2"))
    );
    assert_eq!(
        validate(&manifest("2.2.0-rc.1"), Channel::Beta),
        Ok(version("2.2.0-rc.1"))
    );
    let mut without_notes = manifest("2.1.2");
    without_notes.notes = None;
    assert!(validate(&without_notes, Channel::Stable).is_ok());
}

#[test]
fn an_incoherent_manifest_is_refused() {
    let mut cases = Vec::new();

    let mut bad_version = manifest("2.1.2");
    bad_version.version = "2.1".to_owned();
    bad_version.tag = "v2.1".to_owned();
    cases.push(("not semver", bad_version, Channel::Stable));

    let mut tag_mismatch = manifest("2.1.2");
    tag_mismatch.tag = "v2.1.1".to_owned();
    cases.push(("tag mismatch", tag_mismatch, Channel::Stable));

    cases.push((
        "pre-release on stable",
        manifest("2.2.0-rc.1"),
        Channel::Stable,
    ));

    let mut script_page = manifest("2.1.2");
    script_page.page = "javascript:alert(1)".to_owned();
    cases.push(("script page", script_page, Channel::Stable));

    let mut data_notes = manifest("2.1.2");
    data_notes.notes = Some("data:text/html,hi".to_owned());
    cases.push(("data notes", data_notes, Channel::Stable));

    let mut relative_page = manifest("2.1.2");
    relative_page.page = "/releases/v2.1.2".to_owned();
    cases.push(("relative page", relative_page, Channel::Stable));

    for (name, case, channel) in cases {
        assert_eq!(validate(&case, channel), Err(Failure::Invalid), "{name}");
    }
}

#[test]
fn the_running_version_parses() {
    assert_eq!(running_version().to_string(), env!("CARGO_PKG_VERSION"));
}
