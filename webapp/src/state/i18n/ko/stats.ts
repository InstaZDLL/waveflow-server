// Listening statistics and mixes.
import type { TranslationValue } from "..";
import type { stats as en } from "../en/stats";

export const stats: Record<keyof typeof en, TranslationValue> = {
  "stats.detail": "이 계정의 재생 기록으로 집계한 감상 내역이에요.",
  "stats.period": "기간",
  "stats.period7d": "최근 7일",
  "stats.period30d": "최근 30일",
  "stats.period365d": "최근 12개월",
  "stats.periodAll": "전체 기간",
  "stats.empty": "이 기간에 들은 음악이 없어요.",
  "stats.plays": "재생 수",
  "stats.listeningTime": "감상 시간",
  "stats.tracks": "곡",
  "stats.artists": "아티스트",
  "stats.albums": "앨범",
  "stats.estimate":
    "감상 시간은 들은 곡의 길이를 합산한 값이에요. 실제로 얼마나 들었는지 알려 주는 플레이어는 없어요.",
  "stats.timelineDays": "일별 재생 수",
  "stats.timelineMonths": "월별 재생 수",
  "stats.bucket": {
    one: "{label}: {count}회 재생",
    other: "{label}: {count}회 재생",
  },
  "stats.topTracks": "인기 곡",
  "stats.topArtists": "인기 아티스트",
  "stats.topAlbums": "인기 앨범",
  "stats.topGenres": "인기 장르",
  "stats.playCount": { one: "{count}회 재생", other: "{count}회 재생" },
  "stats.firstPlay": "이 기간의 첫 재생",
  "stats.lastPlay": "마지막 재생",
  "stats.moment": "{title}, {when}",
  "stats.minutes": { one: "{count}분", other: "{count}분" },
  "stats.hoursMinutes": "{hours}시간 {minutes}분",
  "mixes.title": "오늘의 추천",
  "mixes.detail": {
    one: "오늘을 위해 고른 {count}곡",
    other: "오늘을 위해 고른 {count}곡",
  },
  "mixes.mostPlayed": "가장 많이 재생한 곡",
  "mixes.neverPlayed": "한 번도 재생하지 않은 곡",
  "mixes.forgottenFavourites": "잊고 있던 즐겨찾기",
  "mixes.recentlyAdded": "최근 추가",
  "mixes.genre": "{genre} 믹스",
  "mixes.decade": "{decade}년대",
  "mixes.failed": "‘{title}’ 믹스를 열 수 없어요. 다시 시도하세요.",
  "mixes.shelfTitle": "나를 위한 믹스",
  "mixes.shelfDetail":
    "내 감상 기록으로 만든 플레이리스트로, 매일 새로 바뀌어요. 내 앨범은 아래에 있어요.",
  "mixes.badge": "믹스",
};
