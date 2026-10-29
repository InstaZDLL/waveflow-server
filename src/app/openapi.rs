//! The OpenAPI document: the routes and schemas it lists, the passes that
//! state once what holds of every route in a class, and the route that
//! serves it.

use axum::response::{IntoResponse, Response};
use utoipa::OpenApi;

use crate::{
    api, auth::authentication, library::lyrics, media, services, storage::catalog,
    storage::database, storage::sync,
};

pub const OPENAPI_JSON_PATH: &str = "/openapi.json";
pub const SCALAR_PATH: &str = "/reference";

#[derive(OpenApi)]
#[openapi(
    info(
        title = "WaveFlow Server API",
        version = env!("CARGO_PKG_VERSION"),
        description = "Self-hosted WaveFlow music server v2.",
        license(name = "AGPL-3.0-only")
    ),
    paths(
        api::health,
        api::ready,
        api::setup_status,
        api::setup,
        api::login,
        api::refresh,
        api::logout,
        api::web_login,
        api::web_refresh,
        api::web_logout,
        api::oauth_authorize,
        api::oauth_token,
        api::start_scan,
        api::list_libraries,
        api::create_library,
        api::update_library,
        api::delete_library,
        api::library_settings,
        api::unavailable_tracks,
        api::purge_unavailable_tracks,
        api::purged_tracks,
        api::restore_purged_tracks,
        api::list_library_members,
        api::set_library_member,
        api::remove_library_member,
        api::scan_status,
        api::scan_events,
        api::library_events,
        api::library_events_ack,
        api::negotiate_uploads,
        api::upload_session,
        api::upload_chunk,
        api::commit_upload,
        api::list_tracks,
        api::library_tree,
        api::list_folders,
        api::create_folder,
        api::update_folder,
        api::delete_folder,
        api::folder_tracks,
        api::place_tracks,
        api::attention_summary,
        api::attention_credits,
        api::attention_tracks,
        api::list_credit_decisions,
        api::create_credit_decision,
        api::delete_credit_decision,
        api::quality_summary,
        api::quality_tracks,
        api::request_quality_analysis,
        api::track_quality,
        api::track_spectrogram,
        api::get_track,
        api::update_track,
        api::get_track_lyrics,
        api::put_track_lyrics,
        api::delete_track_lyrics,
        api::get_track_lyrics_file,
        api::get_track_overrides,
        api::list_albums,
        api::list_genres,
        api::get_album,
        api::list_artists,
        api::get_artist,
        api::put_artist_image,
        api::delete_artist_image,
        api::list_artist_image_candidates,
        api::put_artist_deezer_portrait,
        api::list_artist_top_songs,
        api::list_artist_albums,
        api::search_catalog,
        api::list_songs,
        api::list_random_songs,
        api::get_listening_stats,
        api::list_mixes,
        api::get_mix,
        api::list_similar_songs,
        api::list_songs_by_genre,
        api::list_playlists,
        api::create_playlist,
        api::import_playlist,
        api::export_playlist,
        api::get_playlist,
        api::update_playlist,
        api::move_playlist_entry,
        api::remove_playlist_entry,
        api::delete_playlist,
        api::list_smart_playlists,
        api::create_smart_playlist,
        api::preview_smart_playlist,
        api::get_smart_playlist,
        api::update_smart_playlist,
        api::delete_smart_playlist,
        api::podcast_overview,
        api::create_podcast_channel,
        api::refresh_podcasts,
        api::get_podcast_channel,
        api::delete_podcast_channel,
        api::newest_podcast_episodes,
        api::get_podcast_episode,
        api::download_podcast_episode,
        api::delete_podcast_episode,
        api::list_favorites,
        api::add_favorite,
        api::remove_favorite,
        api::set_rating,
        api::list_ratings,
        api::create_scrobble,
        api::list_history,
        api::list_now_playing,
        api::list_scrobble_links,
        api::list_scrobble_destinations,
        api::link_scrobble,
        api::unlink_scrobble,
        api::authorize_lastfm,
        api::lastfm_callback,
        api::list_uncertain_scrobbles,
        api::discard_uncertain_scrobble,
        api::retry_uncertain_scrobble,
        api::get_queue,
        api::save_queue,
        api::list_shares,
        api::create_share,
        api::update_share,
        api::delete_share,
        api::sync_changes,
        api::sync_snapshot,
        api::sync_ack,
        api::sync_socket,
        api::transcode_status,
        api::list_users,
        api::create_user,
        api::update_user,
        api::delete_user,
        api::set_subsonic_credential,
        api::revoke_subsonic_credential,
        api::list_radio_stations,
        api::get_radio_station,
        api::create_radio_station,
        api::replace_radio_station,
        api::delete_radio_station,
        api::list_bookmarks,
        api::set_bookmark,
        api::delete_bookmark,
        api::list_own_api_tokens,
        api::create_own_api_token,
        api::revoke_own_api_token,
        api::own_subsonic_credential,
        api::rotate_own_subsonic_credential,
        api::revoke_own_subsonic_credential,
        api::list_api_tokens,
        api::create_api_token,
        api::revoke_api_token,
        media::stream_track,
        media::create_stream_ticket,
        media::stream_with_ticket,
        media::artwork,
        media::canvas_by_hash,
        media::canvas_for_track,
        media::put_canvas,
        media::delete_canvas,
        media::create_canvas_ticket,
        media::canvas_with_ticket
    ),
    components(schemas(
        api::ProbeResponse,
        api::ReadyResponse,
        api::SetupStatusResponse,
        api::SetupRequest,
        api::SetupResponse,
        api::LoginRequest,
        api::RefreshRequest,
        api::WebAuthResponse,
        api::ErrorResponse,
        authentication::AuthTokens,
        authentication::AuthUser,
        database::AccountRole,
        api::ScanQueuedResponse,
        catalog::ScanJobRecord,
        catalog::TrackRecord,
        catalog::LibraryAccess,
        api::DeleteLibraryRequest,
        services::LibraryUpdate,
        services::LibraryUpdated,
        services::LibrarySettings,
        services::LibraryDeleted,
        services::UnavailableTrack,
        services::UnavailableTracks,
        services::PurgeRequest,
        services::PurgeOutcome,
        services::RestoreOutcome,
        services::AlbumItem,
        services::ReleaseGroup,
        services::ArtistItem,
        services::GenreItem,
        services::LibraryMember,
        services::SongItem,
        services::TrackDetail,
        services::TrackOverrides,
        services::TrackOverrideValues,
        services::TrackSourceTags,
        lyrics::LyricsList,
        lyrics::StructuredLyrics,
        lyrics::LyricsLine,
        services::ArtistSummary,
        services::AlbumDetail,
        services::ArtistDetail,
        services::ArtistEnrichment,
        services::EnrichmentState,
        services::EnrichmentSource,
        services::EnrichedBio,
        services::EnrichedImage,
        services::EnrichedSimilar,
        services::ListeningStats,
        services::StatsBucketSize,
        services::StatsTotals,
        services::StatsBucket,
        services::TopTrack,
        services::TopAlbum,
        services::TopArtist,
        services::TopGenre,
        services::PlayMoment,
        services::MixSummary,
        services::Mix,
        services::SearchResult,
        services::PlaylistItem,
        services::SmartPlaylistItem,
        services::SmartPlaylistPreview,
        services::SmartRules,
        services::SmartCondition,
        services::SmartMatch,
        services::SmartField,
        services::SmartOperator,
        services::SmartSort,
        services::SmartOrder,
        api::SmartPlaylistRequest,
        api::SmartPlaylistPatch,
        api::SmartPreviewRequest,
        api::RuleErrorResponse,
        services::PodcastOverview,
        services::PodcastChannelItem,
        services::PodcastEpisodeItem,
        api::CreatePodcastChannelRequest,
        api::FeedErrorResponse,
        services::QueueItem,
        services::RatingItem,
        services::HistoryItem,
        services::ScrobbleProvider,
        services::ScrobbleLinkState,
        services::ScrobbleDestinationName,
        services::ScrobbleUnavailable,
        api::LastFmAuthorizationResponse,
        services::UncertainScrobble,
        api::LinkScrobbleRequest,
        api::RetriedScrobbleResponse,
        services::BookmarkItem,
        services::RadioStationItem,
        services::UserItem,
        api::CreatePlaylistRequest,
        api::UpdatePlaylistRequest,
        api::MovePlaylistEntryRequest,
        api::RemovePlaylistEntryRequest,
        api::RatingRequest,
        api::ScrobbleRequest,
        api::SaveQueueRequest,
        api::CreateShareRequest,
        api::UpdateShareRequest,
        api::ShareResponse,
        api::AuthorizeRequest,
        api::AuthorizeResponse,
        api::TokenRequest,
        api::StarredEntry,
        api::NowPlayingEntry,
        api::SyncAckRequest,
        api::SyncSnapshot,
        services::LibraryEventPage,
        services::DiskListing,
        services::DiskFolder,
        services::DiskTrack,
        services::FiledFolder,
        services::ImageChoices,
        services::ImageChoice,
        services::ArtistImageCandidate,
        api::DeezerPortraitPick,
        services::FiledTree,
        services::AttentionCategory,
        services::AttentionCount,
        services::AttentionTrack,
        services::CreditCandidate,
        services::CreditFragment,
        services::CreditDecisionKind,
        services::CreditDecisionRecord,
        services::QualitySummary,
        services::QualityAnalysis,
        services::AnalysisState,
        services::QualityVerdict,
        services::QualityVerdictCount,
        services::QualityTrack,
        services::TrackQuality,
        services::TrackQualityDetail,
        api::CreditDecisionRequest,
        services::FolderPatch,
        services::FolderRemoved,
        services::Placed,
        api::CreateFolderRequest,
        api::PlacementRequest,
        services::TrackMetadataPatch,
        services::UploadOffer,
        services::UploadVerdict,
        services::UploadDecision,
        services::UploadSessionState,
        services::CommittedUpload,
        api::NegotiateUploadsRequest,
        api::NegotiateUploadsResponse,
        services::LibraryEvent,
        api::TranscodeStatusResponse,
        api::CreateUserRequest,
        api::UpdateUserRequest,
        api::SetSubsonicCredentialRequest,
        api::SubsonicCredentialResponse,
        crate::storage::database::SubsonicCredentialStatus,
        api::BookmarkRequest,
        api::RadioStationRequest,
        api::CreateApiTokenRequest,
        api::CreateApiTokenResponse,
        database::ApiTokenRecord,
        api::CreateLibraryRequest,
        api::CreateLibraryResponse,
        api::SetLibraryMemberRequest,
        sync::SyncChange,
        sync::SyncPage,
        media::StreamTicketResponse,
        media::CanvasResponse,
        api::LibraryEventAckRequest
    )),
    modifiers(&SecurityAddon, &SourceOffer),
    tags(
        (name = "probes", description = "Process and SQLite health"),
        (name = "authentication", description = "Local WaveFlow sessions")
        ,(name = "catalog", description = "Authoritative library scans and catalogue reads")
        ,(name = "user-data", description = "Cross-protocol playlists and playback state")
        ,(name = "sync", description = "Durable WaveFlow Desktop user-data synchronization")
        ,(name = "administration", description = "Administrative user and credential management")
        ,(name = "scrobbling", description = "Per-account authorisations at external listening services, and the queue behind them")
        ,(name = "radio", description = "Internet radio stations, one list per account")
        ,(name = "smart-playlists", description = "Playlists defined by rules, evaluated when read (RFC-025)")
        ,(name = "podcasts", description = "Podcast channels and episodes, one list per instance (RFC-025)")
    )
)]
pub struct ApiDoc;

/// Declares the bearer scheme and the two mutation headers.
///
/// Without this, a client generated from the document alone authenticates
/// nowhere and never sends `X-WaveFlow-Operation-Id` — losing replay safety,
/// which is the one thing the native API offers over the Subsonic facade.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            BEARER_SECURITY_SCHEME,
            SecurityScheme::Http(
                Http::builder()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Access token from /api/v2/auth/login, /api/v2/auth/refresh or the \
                         PKCE exchange, or a long-lived `wfapi_` token minted by the CLI.",
                    ))
                    .build(),
            ),
        );
        openapi.security = Some(vec![utoipa::openapi::security::SecurityRequirement::new(
            BEARER_SECURITY_SCHEME,
            Vec::<String>::new(),
        )]);

        clear_security_on_public_operations(openapi);
        annotate_scope_refusals(openapi);
        annotate_mutation_headers(openapi);
    }
}

/// Links the licence text and the source of this version (AGPL-3.0 §13).
///
/// A pass rather than `license(url = …)` and `external_docs(url = …)` in the
/// attribute: utoipa takes only a string literal there, and the addresses are
/// built from the manifest in `app::source`, so they cannot drift from
/// the version the document already states.
struct SourceOffer;

impl utoipa::Modify for SourceOffer {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(license) = openapi.info.license.as_mut() {
            license.url = Some(super::LICENSE_URL.to_owned());
        }
        openapi.external_docs = Some(
            utoipa::openapi::external_docs::ExternalDocsBuilder::new()
                .url(super::SOURCE_URL)
                .description(Some("Source code of this version"))
                .build(),
        );
    }
}

const BEARER_SECURITY_SCHEME: &str = "bearer";

/// Operations that carry their own credential, or none at all. The global
/// requirement above would otherwise claim a token is needed to log in.
///
/// The two `{ticket}` routes belong here on purpose: the sealed ticket in the
/// path *is* the credential, because neither `<audio src>` nor `<video src>`
/// can send a header. A document that omitted them would generate clients
/// convinced a bearer is required, and they would send one that is not read.
pub(super) const PUBLIC_OPERATIONS: &[(&str, &str)] = &[
    ("/health", "get"),
    ("/ready", "get"),
    ("/api/v2/setup", "get"),
    ("/api/v2/setup", "post"),
    ("/api/v2/auth/login", "post"),
    ("/api/v2/auth/refresh", "post"),
    ("/api/v2/auth/logout", "post"),
    ("/api/v2/web/auth/login", "post"),
    ("/api/v2/web/auth/refresh", "post"),
    ("/api/v2/web/auth/logout", "post"),
    ("/api/v2/oauth/token", "post"),
    ("/api/v2/stream/{ticket}", "get"),
    ("/api/v2/canvas-stream/{ticket}", "get"),
    // The Last.fm return, and the one entry here that is not a ticket. It is
    // reached by a person's browser coming back from another site, with no
    // `Authorization` header and — every cookie of this server being
    // `SameSite=Strict` — no session cookie either. What stands in for proof is
    // narrower than an `Access`: a single-use random, an expiry of a few
    // minutes, and a cookie good for this path alone, all three of which must
    // agree. Listed so the document says what the route does rather than
    // advertising a requirement nothing checks.
    ("/api/v2/scrobble-links/lastfm/callback/{state}", "get"),
];

fn clear_security_on_public_operations(openapi: &mut utoipa::openapi::OpenApi) {
    for (path, method) in PUBLIC_OPERATIONS {
        let Some(item) = openapi.paths.paths.get_mut(*path) else {
            continue;
        };
        // Every method the list may name, not only the two it happens to name
        // today. `annotate_scope_refusals` reads what this leaves behind, so a
        // public `PUT` that fell through here would keep the global requirement
        // and be handed a 403 it can never answer — the pass would be declaring
        // a refusal on a route that has no credential to refuse.
        let operation = match *method {
            "get" => item.get.as_mut(),
            "post" => item.post.as_mut(),
            "put" => item.put.as_mut(),
            "delete" => item.delete.as_mut(),
            "patch" => item.patch.as_mut(),
            _ => None,
        };
        if let Some(operation) = operation {
            operation.security = Some(Vec::new());
        }
    }
}

/// Declares the refusal a narrowed credential meets, on every mutation that can
/// meet it.
///
/// [`api::authenticated`] answers `Forbidden` — 403 — whenever a route asks for
/// `Access::Write` or `Access::Admin` and the caller's scopes do not grant it.
/// Every mutation asks for one of the two, so every secured mutation can answer
/// 403; nineteen of them documented 401 and 422 and stopped there. A client
/// generated from that document has no branch for the refusal and reads a
/// perfectly ordinary answer as a transport failure.
///
/// Injected here rather than written onto each `#[utoipa::path]`, for the
/// reason [`annotate_mutation_headers`] is: a property that holds of every
/// route in a class is stated once, somewhere adding a route cannot forget it.
/// The nineteen were not an oversight repeated nineteen times — they are what
/// happens when the same true sentence has to be typed again per route.
///
/// Reads are excluded because a read normally cannot produce it: `Access::Read`
/// is granted by every scope list. That is a statement about `Access::Read` and
/// not about GET, and the two come apart — `GET /api/v2/uploads/{session_id}`
/// asks for `Access::Write`, because reading a transfer's state belongs to the
/// write flow. Such a route documents its own 403, and
/// `every_refusal_a_narrow_token_meets_is_documented` is what notices when one
/// does not: it asks the server rather than this list.
fn annotate_scope_refusals(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::{ContentBuilder, Ref, RefOr, ResponseBuilder};

    for item in openapi.paths.paths.values_mut() {
        // GET excluded on purpose: see above.
        let operations = [
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.patch.as_mut(),
        ];
        for operation in operations.into_iter().flatten() {
            // Emptied by `clear_security_on_public_operations`, which must
            // therefore have run before this: a route that carries no
            // credential has none to have refused.
            if operation.security.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }
            // `or_insert_with`, so a route that already describes its own 403
            // — the administrative ones, and the library feed acknowledgement —
            // keeps the wording it chose.
            operation
                .responses
                .responses
                .entry("403".to_owned())
                .or_insert_with(|| {
                    ResponseBuilder::new()
                        .description(
                            "Forbidden. `code` is `forbidden`: the credential authenticates, \
                             but its scopes do not admit this operation. Retrying is futile \
                             — the token has to be reissued with wider scopes.",
                        )
                        .content(
                            "application/json",
                            ContentBuilder::new()
                                .schema(Some(RefOr::Ref(Ref::from_schema_name("ErrorResponse"))))
                                .build(),
                        )
                        .build()
                        .into()
                });
        }
    }
}

/// Attaches the two optional mutation headers, and the 409 they can produce, to
/// every user-data *write*. `mutation_context` accepts them on any such route,
/// so they are declared per operation rather than described in prose nobody
/// generates a client from.
///
/// Reads are skipped: a GET carries no operation id and cannot conflict.
fn annotate_mutation_headers(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::path::{ParameterBuilder, ParameterIn};
    use utoipa::openapi::{RefOr, Required, ResponseBuilder, Schema, Type};

    // utoipa 6 holds a parameter list as `RefOr`, so a parameter built here
    // is wrapped on the way in rather than pushed bare.
    let header = |name: &str, description: &str| {
        RefOr::T(
            ParameterBuilder::new()
                .name(name)
                .parameter_in(ParameterIn::Header)
                .required(Required::False)
                .description(Some(description.to_owned()))
                .schema(Some(Schema::Object({
                    let mut object = utoipa::openapi::Object::new();
                    object.schema_type = utoipa::openapi::schema::SchemaType::new(Type::String);
                    object.format = Some(utoipa::openapi::SchemaFormat::Custom("uuid".to_owned()));
                    object
                })))
                .build(),
        )
    };

    for item in openapi.paths.paths.values_mut() {
        // GET excluded on purpose: reads carry no operation id.
        let operations = [
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.patch.as_mut(),
        ];
        for operation in operations.into_iter().flatten() {
            if !operation
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == "user-data"))
            {
                continue;
            }
            operation
                .responses
                .responses
                .entry("409".to_owned())
                .or_insert_with(|| {
                    ResponseBuilder::new()
                        .description(
                            "Conflict. `code` is `conflict`: the operation id was already used \
                         for a different payload. Retrying verbatim will fail again — mint \
                         a new operation id.",
                        )
                        .build()
                        .into()
                });
            let parameters = operation.parameters.get_or_insert_with(Vec::new);
            parameters.push(header(
                api::OPERATION_ID_HEADER,
                "Stable id for this logical mutation. Repeating it replays the original \
                 outcome instead of applying twice; reusing it for a different payload is \
                 rejected as a conflict. Generated server-side when absent.",
            ));
            parameters.push(header(
                api::DEVICE_ID_HEADER,
                "Device originating the mutation. Rejected when it belongs to another \
                 account. Lets other devices skip their own echo in the sync journal.",
            ));
        }
    }
}

pub(super) async fn openapi_response(openapi: utoipa::openapi::OpenApi) -> Response {
    match serde_json::to_string(&openapi) {
        Ok(body) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(error = %error, "OpenAPI serialization failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "OpenAPI serialization failed",
            )
                .into_response()
        }
    }
}
