//! Podcast methods (RFC-025 decision 12).
//!
//! Every one of them is new to this façade: they answered "not implemented"
//! before, and `podcastRole` was `false` for everyone, which was true. Nothing
//! that existed changes shape. The shapes follow the Subsonic schema —
//! `PodcastChannel` and `PodcastEpisode` — and the five writes answer the bare
//! envelope the specification documents (`empty_success_method`).
//!
//! **One list per instance**, as in the reference server: every account reads
//! the channels and plays what is stored; only an account holding
//! `podcastRole` — the switch is on and the account is an administrator — may
//! subscribe, refresh, download or delete, and anyone else is answered error
//! 50.

use super::*;

use crate::services::{PodcastChannelItem, PodcastEpisodeItem, PodcastFailure};

pub(super) async fn podcasts(
    state: &AppState,
    principal: &Principal,
    method: &str,
    params: &Params,
) -> Result<Node, ProtocolError> {
    match method {
        "getPodcasts" => {
            let with_episodes = params.bool_optional("includeEpisodes")?.unwrap_or(true);
            let channels = state
                .services
                .podcast_channels(principal.id, params.uuid_optional("id")?, with_episodes)
                .await
                .map_err(internal)?;
            Ok(Node::new("podcasts").children(channels.iter().map(channel_node)))
        }
        "getNewestPodcasts" => {
            let count = params.i64_or("count", crate::services::NEWEST_EPISODES_DEFAULT)?;
            let episodes = state
                .services
                .newest_podcast_episodes(principal.id, count)
                .await
                .map_err(internal)?;
            Ok(Node::new("newestPodcasts").children(episodes.iter().map(episode_node)))
        }
        // An OpenSubsonic addition: one episode, by id.
        "getPodcastEpisode" => {
            let episode = state
                .services
                .podcast_episode(principal.id, params.uuid("id")?)
                .await
                .map_err(service_protocol)?;
            Ok(episode_node(&episode).renamed("podcastEpisode"))
        }
        "refreshPodcasts" => {
            state
                .services
                .refresh_podcasts(principal.id)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("refreshPodcasts"))
        }
        "createPodcastChannel" => {
            let url = params.first("url").ok_or_else(missing)?;
            match state
                .services
                .create_podcast_channel(principal.id, url)
                .await
            {
                Ok(_) => Ok(Node::new("createPodcastChannel")),
                Err(PodcastFailure::Service(error)) => Err(service_protocol(error)),
                // The reason stays in the native answer; the protocol has one
                // field for a sentence, and a generic one says enough.
                Err(PodcastFailure::Feed(_)) => Err(ProtocolError {
                    code: 0,
                    message: "The podcast feed could not be fetched or read",
                }),
            }
        }
        "deletePodcastChannel" => {
            state
                .services
                .delete_podcast_channel(principal.id, params.uuid("id")?)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("deletePodcastChannel"))
        }
        "deletePodcastEpisode" => {
            state
                .services
                .delete_podcast_episode(principal.id, params.uuid("id")?)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("deletePodcastEpisode"))
        }
        "downloadPodcastEpisode" => {
            state
                .services
                .download_podcast_episode(principal.id, params.uuid("id")?)
                .await
                .map_err(service_protocol)?;
            Ok(Node::new("downloadPodcastEpisode"))
        }
        _ => unreachable!("podcast method dispatch is exhaustive"),
    }
}

/// A channel as `PodcastChannel` has it. The cover is the feed's own address,
/// as `originalImageUrl`; there is no `coverArt`, because the server does not
/// fetch the image (decision 10).
fn channel_node(channel: &PodcastChannelItem) -> Node {
    Node::new("channel")
        .attr("id", channel.id.to_string())
        .attr("url", channel.url.clone())
        .attr("title", channel.title.clone())
        .maybe_attr("description", channel.description.clone())
        .maybe_attr("originalImageUrl", channel.image_url.clone())
        .attr("status", channel.status.clone())
        .maybe_attr("errorMessage", channel.error.clone())
        .children(channel.episodes.iter().flatten().map(episode_node))
}

/// An episode as `PodcastEpisode` has it: a `Child` whose `streamId` is
/// present once it is stored, and is its own id — `stream` and `download`
/// take it.
pub(super) fn episode_node(episode: &PodcastEpisodeItem) -> Node {
    let completed = episode.status == "completed";
    Node::new("episode")
        .attr("id", episode.id.to_string())
        .maybe_attr("streamId", completed.then(|| episode.id.to_string()))
        .attr("channelId", episode.channel_id.to_string())
        .attr("parent", episode.channel_id.to_string())
        .attr("isDir", false)
        .attr("title", episode.title.clone())
        .attr("album", episode.channel_title.clone())
        .attr("artist", episode.channel_title.clone())
        .maybe_attr("description", episode.description.clone())
        .maybe_attr("publishDate", episode.published_at.map(iso_time))
        .maybe_attr(
            "year",
            episode
                .published_at
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|date| i64::from(chrono::Datelike::year(&date))),
        )
        .attr("genre", "Podcast")
        .attr("status", episode.status.clone())
        .maybe_attr("size", episode.size)
        .maybe_attr("suffix", episode.suffix.clone())
        .maybe_attr("contentType", episode.content_type.clone())
        .maybe_attr(
            "duration",
            episode.duration_ms.map(|duration| duration / 1000),
        )
        .attr("type", "podcast")
        .attr("isVideo", false)
        .attr("created", iso_time(episode.created_at))
}

/// The episode bookmarks of an account, as `getBookmarks` lists them after
/// the track ones.
pub(super) async fn episode_bookmark_nodes(
    state: &AppState,
    principal: &Principal,
) -> Result<Vec<Node>, ProtocolError> {
    Ok(state
        .services
        .episode_bookmarks(principal.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|bookmark| {
            Node::new("bookmark")
                .attr("position", bookmark.position_ms)
                .attr("username", principal.username.clone())
                .maybe_attr("comment", bookmark.comment.clone())
                .attr("created", iso_time(bookmark.created_at))
                .attr("changed", iso_time(bookmark.updated_at))
                .child(
                    episode_node(&bookmark.episode)
                        .renamed("entry")
                        .attr("bookmarkPosition", bookmark.position_ms),
                )
        })
        .collect())
}

/// A user record with `podcastRole` said truthfully: the switch is on and the
/// account is an enabled administrator.
pub(super) fn user_with_podcast_role(state: &AppState, user: &crate::services::UserItem) -> Node {
    user_node(user).attr(
        "podcastRole",
        state.services.podcast_role(user.role, user.disabled),
    )
}
