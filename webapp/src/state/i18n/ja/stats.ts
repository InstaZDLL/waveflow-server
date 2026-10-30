// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail":
    "このアカウントで聴いたものを、このアカウント自身の再生記録から集計しています。",
  "stats.period": "期間",
  "stats.period7d": "過去7日間",
  "stats.period30d": "過去30日間",
  "stats.period365d": "過去12か月",
  "stats.periodAll": "全期間",
  "stats.empty": "この期間には何も聴いていません。",
  "stats.plays": "再生回数",
  "stats.listeningTime": "再生時間",
  "stats.tracks": "曲",
  "stats.artists": "アーティスト",
  "stats.albums": "アルバム",
  "stats.estimate":
    "再生時間は、再生した各曲の長さを合計したものです。実際にどこまで聴いたかは、どのプレーヤーも報告しません。",
  "stats.timelineDays": "1日あたりの再生回数",
  "stats.timelineMonths": "1か月あたりの再生回数",
  "stats.bucket": {
    one: "{label}：{count}回再生",
    other: "{label}：{count}回再生",
  },
  "stats.topTracks": "トップソング",
  "stats.topArtists": "トップアーティスト",
  "stats.topAlbums": "トップアルバム",
  "stats.topGenres": "トップジャンル",
  "stats.playCount": { one: "{count}回再生", other: "{count}回再生" },
  "stats.firstPlay": "この期間の最初の再生",
  "stats.lastPlay": "最後の再生",
  "stats.moment": "{title}、{when}",
  "stats.minutes": { one: "{count}分", other: "{count}分" },
  "stats.hoursMinutes": "{hours}時間{minutes}分",
  "mixes.title": "今日のあなたへのおすすめ",
  "mixes.detail": {
    one: "今日のために選んだ{count}曲",
    other: "今日のために選んだ{count}曲",
  },
  "mixes.mostPlayed": "よく聴いた曲",
  "mixes.neverPlayed": "まだ聴いていない曲",
  "mixes.forgottenFavourites": "忘れかけていたお気に入り",
  "mixes.recentlyAdded": "最近追加した曲",
  "mixes.genre": "{genre} ミックス",
  "mixes.decade": "{decade}年代",
  "mixes.failed": "「{title}」を開けませんでした。もう一度お試しください。",
  "mixes.shelfTitle": "あなたへのミックス",
  "mixes.shelfDetail":
    "あなたのリスニングから作られ、毎日新しくなるプレイリストです。アルバムはこの下にあります。",
  "mixes.badge": "ミックス",
};
