// The attention lists and artist splits.
import type { TranslationValue } from "..";
import type { attention as en } from "../en/attention";

export const attention: Record<keyof typeof en, TranslationValue> = {
  "attention.lists": "リスト",
  "attention.total": {
    one: "確認が必要な項目：{count}件",
    other: "確認が必要な項目：{count}件",
  },
  "attention.nothing": "確認が必要な項目はありません。",
  "attention.notYours":
    "このライブラリの要確認項目を見られるのは、オーナーとマネージャーだけです。",
  "attention.empty": "このリストには何もありません。",
  "attention.tracksNote":
    "曲のメニューから修正してください。修正されると、その曲はリストから外れます。",
  "attention.creditsNote":
    "これらのクレジットは、ひとつの文字列で複数のアーティストを指しています。指しているアーティストごとに分割するか、ひとつの名前であれば1つのまま残してください。この判断は、以降のすべてのスキャンに適用されます。",
  "attention.pending": "未決定",
  "attention.decided": "決定済み",
  "attention.split": "分割",
  "attention.confirmSplit": "確認：{count}つに分割",
  "attention.keep": "1つのまま残す",
  "attention.splitDone": "{name}を分割しました。",
  "attention.keepDone": "{name}は1つのまま残ります。",
  "attention.undone": "{name}についての判断を取り消しました。",
  "attention.alreadyDecided":
    "このクレジットはその間に決定されました。リストを再読み込みしました。",
  "attention.failed": "うまくいきませんでした。もう一度お試しください。",
  "attention.noCredits": "判断を待っているクレジットはありません。",
  "attention.noDecisions": "まだ何も決定されていません。",
  "attention.fragments": "{name}を分割した場合",
  "attention.knownFragment": "このライブラリにすでにあるアーティスト",
  "attention.decidedSplit": "{parts}に分割",
  "attention.decidedKeep": "1つのまま",
  "attention.decidedBy": "{who}、{when}",
  "attention.undo": "取り消す",
  "attention.duplicateGroup": "重複の可能性があるグループ {group}",
  "attention.split_credits": "分割するアーティスト",
  "attention.missing_artist": "アーティストなし",
  "attention.missing_album": "アルバムなし",
  "attention.missing_year": "リリース年なし",
  "attention.missing_track_number": "トラック番号なし",
  "attention.missing_cover": "ジャケットなし",
  "attention.album_year_conflict": "複数のリリース年があるアルバム",
  "attention.compilation_no_album_artist":
    "アルバムアーティストのないコンピレーション",
  "attention.duplicate_track_number": "重複したトラック番号",
  "attention.track_number_gap": "トラック番号の欠番",
  "attention.probable_duplicate": "重複の可能性",
  "attention.probable_fake_lossless": "偽ロスレスの可能性",
  "artistSplit.offer": "{count}組のアーティストに分割",
  "artistSplit.confirm": "確認：{count}つに分割",
  "artistSplit.done": "{parts}に分割しました。",
  "artistSplit.failed":
    "このクレジットを分割できませんでした。もう一度お試しください。",
};
