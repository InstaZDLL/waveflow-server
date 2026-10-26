//! Internet radio stations through the façade.

use super::*;

/// The last empty container in the facade, filled.
///
/// `getInternetRadioStations` answered `{}` from the beginning, and it is the
/// one absence here a client cannot read as an absence: every other missing
/// feature declares a role `false`, while "zero stations" is also what a server
/// without radio says. This asserts the four methods, the two deliberate
/// deviations from the reference server — a per-account list, and the
/// specification's own `homepageUrl` in / `homePageUrl` out asymmetry — and the
/// shape of a container holding exactly one child, which is the only length
/// that can tell a declared array from an object.
#[tokio::test]
async fn internet_radio_stations_are_kept_per_account_and_handed_out_never_fetched() {
    let (_temp, config, state) = test_app().await;
    // Inserted rather than created, and every credential drawn at random: a
    // string literal flowing into a cryptographic call is what
    // `rust/hard-coded-cryptographic-value` reports, whatever the call is for.
    // These accounts never present a password, so nothing is hashed.
    let owner = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    for (id, username) in [(owner, "radio-owner"), (outsider, "radio-outsider")] {
        sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
             VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
        )
        .bind(id.to_string())
        .bind(username)
        .bind(now_ms())
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
    }
    let key_of = |who: Uuid| {
        let state = state.clone();
        async move {
            let api_key = format!("wfsk_{}", Uuid::new_v4());
            let secret = Uuid::new_v4().to_string();
            state
                .db
                .set_subsonic_credential(
                    who,
                    who,
                    &state.secret_box.encrypt(secret.as_bytes()).unwrap(),
                    &security::token_hash(&api_key),
                    now_ms(),
                )
                .await
                .unwrap();
            api_key
        }
    };
    let api_key = key_of(owner).await;
    let other_key = key_of(outsider).await;

    let router = waveflow_server::app(&config, state.clone());

    // Still the empty container before anything is stored — the answer this
    // method used to give always.
    let empty = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    assert!(empty["subsonic-response"]["internetRadioStations"].is_object());
    assert!(omits(
        &empty["subsonic-response"]["internetRadioStations"],
        "internetRadioStation"
    ));

    let created = subsonic_json(
        &router,
        "createInternetRadioStation",
        &api_key,
        "&name=Zed%20Radio&streamUrl=https://radio.example.org/zed.mp3&homepageUrl=https://radio.example.org/zed",
    )
    .await;
    assert_eq!(created["subsonic-response"]["status"], "ok");
    // A write the specification documents as answering nothing answers nothing.
    assert!(omits(
        &created["subsonic-response"],
        "createInternetRadioStation"
    ));

    // One child, which is the length that distinguishes a declared array from
    // an object. Reading it through `as_array().unwrap_or_default()` would
    // yield `[]` here and fail further down, saying nothing about the shape.
    let one = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    let one = &one["subsonic-response"]["internetRadioStations"];
    assert!(
        one["internetRadioStation"].is_array(),
        "internetRadioStations.internetRadioStation is a list: {one}"
    );
    let station = &one["internetRadioStation"][0];
    assert_eq!(station["name"], "Zed Radio");
    // Handed back exactly as typed. The server stores an address and never
    // fetches one, which is why this feature needs no allowlist.
    assert_eq!(station["streamUrl"], "https://radio.example.org/zed.mp3");
    // The specification's spelling: `homepageUrl` in, `homePageUrl` out.
    assert_eq!(station["homePageUrl"], "https://radio.example.org/zed");
    let station_id = station["id"].as_str().unwrap().to_owned();

    // XML carries the same asymmetry, and it is the surface most clients read.
    let xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getInternetRadioStations.view?apiKey={api_key}&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(xml.status(), StatusCode::OK);
    let xml = body_text(xml).await;
    assert!(
        xml.contains("<internetRadioStation ") && xml.contains("homePageUrl="),
        "{xml}"
    );

    // A second station, with no homepage, and a lowercase name against the
    // first's capital: under SQLite's default BINARY collation `Z` (90) sorts
    // before `a` (97), so this pair comes back in the other order the moment
    // the `COLLATE NOCASE` leaves the query. Two names of the same case would
    // have asserted nothing about the collation.
    subsonic_json(
        &router,
        "createInternetRadioStation",
        &api_key,
        // `homepageUrl=` and not an omitted parameter: a form with the box
        // left blank sends the name with no value, and that means the station
        // has no page rather than one spelled with zero characters. Refusing
        // it would make clearing the field an error (constat de revue, pris).
        "&name=alpha%20radio&streamUrl=http://alpha.example.org/stream&homepageUrl=",
    )
    .await;
    let two = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    let two = two["subsonic-response"]["internetRadioStations"]["internetRadioStation"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(two.len(), 2);
    assert_eq!(two[0]["name"], "alpha radio");
    assert_eq!(two[1]["name"], "Zed Radio");
    // Absent, not null: that station gave no homepage, and it said so with an
    // empty value rather than by omitting the parameter.
    assert!(omits(&two[0], "homePageUrl"));

    // The update carries every field, so omitting the homepage clears it. That
    // is the protocol's shape, and the reason the native route is a `PUT`.
    subsonic_json(
        &router,
        "updateInternetRadioStation",
        &api_key,
        &format!(
            "&id={station_id}&name=Zed%20Radio%20Deux&streamUrl=https://radio.example.org/zed2.mp3"
        ),
    )
    .await;
    let updated = subsonic_json(&router, "getInternetRadioStations", &api_key, "").await;
    let updated = &updated["subsonic-response"]["internetRadioStations"]["internetRadioStation"];
    let zed = updated
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == station_id.as_str())
        .unwrap();
    assert_eq!(zed["name"], "Zed Radio Deux");
    assert_eq!(zed["streamUrl"], "https://radio.example.org/zed2.mp3");
    assert!(omits(zed, "homePageUrl"));

    // Per account, which is the deviation from the reference server: the other
    // account has its own list, and it is empty.
    let foreign_list = subsonic_json(&router, "getInternetRadioStations", &other_key, "").await;
    assert!(omits(
        &foreign_list["subsonic-response"]["internetRadioStations"],
        "internetRadioStation"
    ));
    // And it cannot reach the owner's station by id. Missing and forbidden
    // answer identically.
    for method in ["updateInternetRadioStation", "deleteInternetRadioStation"] {
        let refused = subsonic_json(
            &router,
            method,
            &other_key,
            &format!("&id={station_id}&name=Stolen&streamUrl=https://x.example.org/s"),
        )
        .await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{method}"
        );
    }
    let absent = subsonic_json(
        &router,
        "deleteInternetRadioStation",
        &api_key,
        &format!("&id={}", Uuid::nil()),
    )
    .await;
    assert_eq!(absent["subsonic-response"]["error"]["code"], 70);

    // An address the server would have had to dereference to learn anything
    // about is refused. None of these needs escaping: a query string reads
    // `:`, `/`, `;` and `,` as data.
    //
    // `mms://` is the one that proves the scheme restriction: `file:` and
    // `data:` are refused for naming no host, so they would still be refused
    // with any scheme list at all. Only an authority-carrying scheme tells the
    // two rules apart.
    for address in [
        "mms://radio.example.org/stream",
        "file:///etc/passwd",
        "data:audio/mpeg;base64,AAAA",
        "/relative/stream.mp3",
    ] {
        let refused = subsonic_json(
            &router,
            "createInternetRadioStation",
            &api_key,
            &format!("&name=Elsewhere&streamUrl={address}"),
        )
        .await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 10,
            "{address}"
        );
    }
    // A blank name is refused too: it would list as a row nothing tells apart.
    let blank = subsonic_json(
        &router,
        "createInternetRadioStation",
        &api_key,
        "&name=%20%20&streamUrl=https://radio.example.org/blank",
    )
    .await;
    assert_eq!(blank["subsonic-response"]["error"]["code"], 10);

    // A required parameter missing is error 10, not a station created with a
    // default nobody asked for.
    for (method, extra) in [
        ("createInternetRadioStation", "&name=No%20Address"),
        (
            "createInternetRadioStation",
            "&streamUrl=https://radio.example.org/anonymous",
        ),
        ("deleteInternetRadioStation", ""),
    ] {
        let incomplete = subsonic_json(&router, method, &api_key, extra).await;
        assert_eq!(
            incomplete["subsonic-response"]["error"]["code"], 10,
            "{method}{extra}"
        );
    }

    // Nothing refused above created anything: two stations, both the owner's.
    assert_eq!(state.services.radio_stations(owner).await.unwrap().len(), 2);
    assert!(state
        .services
        .radio_stations(outsider)
        .await
        .unwrap()
        .is_empty());

    subsonic_json(
        &router,
        "deleteInternetRadioStation",
        &api_key,
        &format!("&id={station_id}"),
    )
    .await;
    assert_eq!(state.services.radio_stations(owner).await.unwrap().len(), 1);
}
