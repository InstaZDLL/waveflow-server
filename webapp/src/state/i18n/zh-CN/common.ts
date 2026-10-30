// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "加载中",
  "common.loadError": "无法加载此页面",
  "common.noAlbums": "还没有专辑。",
  "common.noArtists": "还没有艺人。",
  "common.variousArtists": "群星",
  "common.unknownArtist": "未知艺人",
  "common.album": "专辑",
  "common.artist": "艺人",
  "common.albums": { one: "{count} 张专辑", other: "{count} 张专辑" },
  "common.tracks": { one: "{count} 首曲目", other: "{count} 首曲目" },
  "common.delete": "删除",
  "common.cancel": "取消",
  "common.back": "返回",
  "file.megabytes": "{size} MB",
  "file.kilobytes": "{size} kB",
  "file.channels": { one: "单声道", other: "{count} 声道" },
  "card.play": "播放",
  "card.queue": "添加到队列",
  "rating.label": "评分",
  "rating.stars": { one: "{count} 星", other: "{count} 星" },
  "source.offer": "WaveFlow Server {version} 是自由软件（AGPL-3.0）。",
  "source.link": "源代码",
};
