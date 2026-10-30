// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail": "這個帳號聽過的內容，根據它自己的播放紀錄統計。",
  "stats.period": "時間範圍",
  "stats.period7d": "最近 7 天",
  "stats.period30d": "最近 30 天",
  "stats.period365d": "最近 12 個月",
  "stats.periodAll": "所有時間",
  "stats.empty": "這段期間沒有收聽紀錄。",
  "stats.plays": "播放次數",
  "stats.listeningTime": "收聽時間",
  "stats.tracks": "曲目",
  "stats.artists": "演出者",
  "stats.albums": "專輯",
  "stats.estimate":
    "收聽時間是每首聽過的曲目長度加總；播放器不會告知實際聽了多少。",
  "stats.timelineDays": "每日播放次數",
  "stats.timelineMonths": "每月播放次數",
  "stats.bucket": {
    one: "{label}：播放 {count} 次",
    other: "{label}：播放 {count} 次",
  },
  "stats.topTracks": "熱門曲目",
  "stats.topArtists": "熱門演出者",
  "stats.topAlbums": "熱門專輯",
  "stats.topGenres": "熱門類型",
  "stats.playCount": { one: "播放 {count} 次", other: "播放 {count} 次" },
  "stats.firstPlay": "這段期間的第一次播放",
  "stats.lastPlay": "最近一次播放",
  "stats.moment": "{title}，{when}",
  "stats.minutes": { one: "{count} 分鐘", other: "{count} 分鐘" },
  "stats.hoursMinutes": "{hours} 小時 {minutes} 分鐘",
  "mixes.title": "今日為你推薦",
  "mixes.detail": {
    one: "{count} 首歌曲，今日精選",
    other: "{count} 首歌曲，今日精選",
  },
  "mixes.mostPlayed": "最常播放",
  "mixes.neverPlayed": "從未播放",
  "mixes.forgottenFavourites": "被遺忘的收藏",
  "mixes.recentlyAdded": "最近加入",
  "mixes.genre": "{genre} 精選集",
  "mixes.decade": "{decade} 年代",
  "mixes.failed": "無法開啟「{title}」，請再試一次。",
  "mixes.shelfTitle": "為你推薦的精選集",
  "mixes.shelfDetail":
    "根據你的收聽紀錄產生的播放清單，每天更新。你的專輯在下方。",
  "mixes.badge": "精選集",
};
