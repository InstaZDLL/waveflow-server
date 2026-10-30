// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  // RFC-024: listening statistics and mixes.
  "stats.detail": "这个账户听过的内容，根据它自己的播放记录统计。",
  "stats.period": "时间范围",
  "stats.period7d": "最近 7 天",
  "stats.period30d": "最近 30 天",
  "stats.period365d": "最近 12 个月",
  "stats.periodAll": "全部时间",
  "stats.empty": "这段时间内没有收听记录。",
  "stats.plays": "播放次数",
  "stats.listeningTime": "收听时长",
  "stats.tracks": "曲目",
  "stats.artists": "艺人",
  "stats.albums": "专辑",
  "stats.estimate":
    "收听时长是每首播放过的曲目时长之和；播放器不会告知实际听了多少。",
  "stats.timelineDays": "每日播放次数",
  "stats.timelineMonths": "每月播放次数",
  "stats.bucket": {
    one: "{label}：{count} 次播放",
    other: "{label}：{count} 次播放",
  },
  "stats.topTracks": "热门曲目",
  "stats.topArtists": "热门艺人",
  "stats.topAlbums": "热门专辑",
  "stats.topGenres": "热门流派",
  "stats.playCount": { one: "{count} 次播放", other: "{count} 次播放" },
  "stats.firstPlay": "这段时间的首次播放",
  "stats.lastPlay": "最近一次播放",
  "stats.moment": "{title}，{when}",
  "stats.minutes": { one: "{count} 分钟", other: "{count} 分钟" },
  "stats.hoursMinutes": "{hours} 小时 {minutes} 分钟",
  "mixes.title": "今日为你推荐",
  "mixes.detail": {
    one: "{count} 首曲目，今日精选",
    other: "{count} 首曲目，今日精选",
  },
  "mixes.mostPlayed": "最常播放",
  "mixes.neverPlayed": "从未播放",
  "mixes.forgottenFavourites": "被遗忘的收藏",
  "mixes.recentlyAdded": "最近添加",
  "mixes.genre": "{genre} 合集",
  "mixes.decade": "{decade} 年代",
  "mixes.failed": "无法打开“{title}”，请重试。",
  "mixes.shelfTitle": "为你推荐的合集",
  "mixes.shelfDetail": "根据你的收听记录生成的歌单，每天更新。你的专辑在下方。",
  "mixes.badge": "合集",
};
