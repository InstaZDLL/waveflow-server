// Podcast subscriptions and episodes.
export const podcasts = {
  "nav.podcasts": "Podcasts",
  "podcasts.detail": "Shows this server follows, for every account",
  "podcasts.off":
    "Podcasts are switched off on this server. What was kept before stays readable; nothing new is fetched.",
  "podcasts.url": "Feed address",
  "podcasts.shared":
    "Visible to every account: anyone on this server sees the show and can play the episodes it downloads. Only administrators see the feed address.",
  "podcasts.subscribe": "Subscribe",
  "podcasts.refresh": "Refresh every feed",
  "podcasts.refreshing": "The feeds are being fetched again.",
  "podcasts.reload": "Reload",
  "podcasts.empty": "No podcasts yet.",
  "podcasts.emptyDetail":
    "An administrator subscribes to a show’s feed here. The server downloads its episodes, and every account on this server can listen to them.",
  "podcasts.addFirst": "Add your first podcast",
  "podcasts.channels": "Shows",
  "podcasts.showing": "Showing the episodes of {title}.",
  "podcasts.noEpisodes": "This feed has no episodes.",
  "podcasts.episodes": {
    one: "{count} episode",
    other: "{count} episodes",
  },
  "podcasts.failed": "last fetch failed: {reason}",
  "podcasts.open": "Episodes",
  "podcasts.hide": "Hide",
  "podcasts.unsubscribe": "Unsubscribe",
  "podcasts.download": "Download",
  "podcasts.play": "Play {title}",
  "podcasts.pause": "Pause {title}",
  "podcasts.resumeAt": "resumes at {time}",
  "podcasts.status.new": "not downloaded",
  "podcasts.status.downloading": "downloading",
  "podcasts.status.completed": "ready",
  "podcasts.status.error": "download failed",
  "podcasts.status.deleted": "deleted",
  "podcasts.conflict": "This server already follows that feed.",
  "podcasts.refused":
    "The feed could not be fetched or read, or its address is not one this server may reach.",
  "podcasts.subscribeError": "The feed could not be added.",
  "podcasts.mutationError": "That did not work. Try again.",
  "podcasts.playbackError": "This episode could not be played.",
} as const;
