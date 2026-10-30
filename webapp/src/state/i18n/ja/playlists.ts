// Playlists and their import, the queue, shares and bookmarks.
import type { TranslationValue } from "..";
import type { playlists as en } from "../en/playlists";

export const playlists: Record<keyof typeof en, TranslationValue> = {
  "playlists.detail": {
    one: "プレイリスト{count}件",
    other: "プレイリスト{count}件",
  },
  "playlists.newName": "新しいプレイリストの名前",
  "playlists.create": "プレイリストを作成",
  "playlists.play": "再生",
  "playlists.addQueue": "再生キューを追加",
  "playlists.emptyOne": "このプレイリストは空です。",
  "playlists.export": ".m3u8 として保存",
  "playlists.exportError": "このプレイリストを保存できませんでした。",
  "playlists.empty": "プレイリストを作成すると、曲をまとめておけます。",
  "playlists.createError": "プレイリストを作成できませんでした。",
  "playlists.queueError":
    "再生キューをこのプレイリストに追加できませんでした。",
  "playlists.deleteError": "プレイリストを削除できませんでした。",
  "playlists.public": "すべてのアカウントに公開",
  "playlists.publicError": "このプレイリストを公開できませんでした。",
  "playlists.others": "ほかのアカウントのプレイリスト",
  "playlists.othersDetail":
    "ほかのアカウントの公開プレイリストです。あなたのライブラリにある曲が表示されます。",
  "playlists.by": "作成者：{owner}",
  "playlists.moveUp": "{title}を上へ移動",
  "playlists.moveDown": "{title}を下へ移動",
  "playlists.removeEntry": "{title}をこのプレイリストから削除",
  "playlists.moveError":
    "この項目を移動できませんでした。プレイリストがほかの場所で変更された可能性があるため、読み込み直しました。",
  "playlists.removeEntryError": "この項目を削除できませんでした。",
  "import.title": "ほかのプレーヤーからプレイリストを取り込む",
  "import.detail":
    ".m3u または .m3u8 ファイルです。取り込めない項目を確認するまで、何も作成されません。",
  "import.pick": "プレイリストファイルを選択",
  "import.name": "プレイリスト名",
  "import.found": "{entries}件中{resolved}件がライブラリにあります。",
  "import.encoding":
    "このファイルは UTF-8 ではなかったため、{encoding} として読み込みました。取り込む前に、下の名前を確認してください。",
  "import.missing": {
    one: "配置できない項目：{count}件",
    other: "配置できない項目：{count}件",
  },
  "import.line": "{line}行目",
  "import.noMatch": "一致する曲がありません",
  "import.ambiguous": "複数の曲に一致します",
  "import.notAPath": "ライブラリ内のファイルではありません",
  "import.duplicate": "すでにこのプレイリストにあります",
  "import.confirm": "取り込む",
  "import.cancel": "キャンセル",
  "import.failed": "このファイルをプレイリストとして読み込めませんでした。",
  "queue.detail": {
    one: "アカウントと同期している{count}曲",
    other: "アカウントと同期している{count}曲",
  },
  "queue.playStart": "最初から再生",
  "queue.clear": "再生キューを消去",
  "queue.remove": "削除",
  "queue.empty":
    "アルバムやプレイリストを再生すると、同期される再生キューが作られます。",
  "shares.detail": "選んだ音楽の公開リンク",
  "shares.description": "説明",
  "shares.create": "現在の再生キューを共有",
  "shares.needQueue":
    "リンクを作成する前に、再生キューに曲を追加してください。",
  "shares.once": "このリンクが表示されるのは1回だけです：",
  "shares.defaultName": "音楽の共有",
  "shares.empty": "公開リンクはまだ作成されていません。",
  "shares.createError": "共有リンクを作成できませんでした。",
  "shares.tooLong":
    "共有リンクに入れられるのは{limit}曲までですが、再生キューには{count}曲あります。共有するには再生キューを短くしてください。",
  "shares.deleteError": "共有リンクを削除できませんでした。",
  "shares.maxVisits": "アクセス回数の上限（任意）",
  "shares.visits": { one: "{count}回のアクセス", other: "{count}回のアクセス" },
  "shares.visitsOf": "{max}回中{count}回アクセス",
  "share.kicker": "あなたへの共有",
  "share.defaultTitle": "あなたに共有された音楽",
  "share.until": "{date}まで利用可能",
  "share.empty": "これらの曲はどれも再生できなくなっています。",
  "share.player": "プレーヤー",
  "share.unavailableTitle": "このリンクでは何も開けません",
  "share.unavailableDetail":
    "期限切れか、取り消されたか、許可された回数まで開かれた可能性があります。",
  "bookmarks.title": "保存した再生位置",
  "bookmarks.detail": "1曲につき1つです。もう一度保存すると位置が移動します。",
  "bookmarks.save": "この位置を保存",
  "bookmarks.forget": "削除",
};
