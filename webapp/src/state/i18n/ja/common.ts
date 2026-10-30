// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "読み込み中",
  "common.loadError": "この画面を読み込めませんでした",
  "common.noAlbums": "アルバムはまだありません。",
  "common.noArtists": "アーティストはまだいません。",
  "common.variousArtists": "さまざまなアーティスト",
  "common.unknownArtist": "不明なアーティスト",
  "common.album": "アルバム",
  "common.artist": "アーティスト",
  "common.albums": { one: "アルバム{count}枚", other: "アルバム{count}枚" },
  "common.tracks": { one: "{count}曲", other: "{count}曲" },
  "common.delete": "削除",
  "common.cancel": "キャンセル",
  "common.back": "戻る",
  "file.megabytes": "{size} MB",
  "file.kilobytes": "{size} kB",
  "file.channels": { one: "モノラル", other: "{count}ch" },
  "card.play": "再生",
  "card.queue": "再生キューに追加",
  "rating.label": "評価",
  "rating.stars": { one: "星{count}つ", other: "星{count}つ" },
  "source.offer":
    "WaveFlow Server {version} はフリーソフトウェアです（AGPL-3.0）。",
  "source.link": "ソースコード",
};
