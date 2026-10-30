// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "載入中",
  "common.loadError": "無法載入這個頁面",
  "common.noAlbums": "還沒有專輯。",
  "common.noArtists": "還沒有演出者。",
  "common.variousArtists": "群星",
  "common.unknownArtist": "未知的演出者",
  "common.album": "專輯",
  "common.artist": "演出者",
  "common.albums": { one: "{count} 張專輯", other: "{count} 張專輯" },
  "common.tracks": { one: "{count} 首歌曲", other: "{count} 首歌曲" },
  "common.delete": "刪除",
  "common.cancel": "取消",
  "common.back": "返回",
  "file.megabytes": "{size} MB",
  "file.kilobytes": "{size} kB",
  "file.channels": { one: "單聲道", other: "{count} 聲道" },
  "card.play": "播放",
  "card.queue": "加入播放佇列",
  "rating.label": "評分",
  "rating.stars": { one: "{count} 顆星", other: "{count} 顆星" },
  "source.offer": "WaveFlow Server {version} 是自由軟體（AGPL-3.0）。",
  "source.link": "原始碼",
};
