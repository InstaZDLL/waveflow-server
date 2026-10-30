// Administration, scans, maintenance and missing files.
import type { TranslationValue } from "..";
import type { admin as en } from "../en/admin";

export const admin: Record<keyof typeof en, TranslationValue> = {
  "admin.title": "管理",
  "admin.detail": "ライブラリ、スキャン、アカウント",
  "admin.libraries": "ライブラリ",
  "admin.libraryName": "ライブラリ名",
  "admin.libraryPath": "サーバー上のフォルダの絶対パス",
  "admin.registering": "登録しています…",
  "admin.register": "登録してスキャン",
  "admin.scan": "今すぐスキャン",
  "admin.accounts": "アカウント",
  "admin.webPassword": "Web パスワード（12文字以上）",
  "admin.creating": "作成しています…",
  "admin.create": "アカウントを作成",
  "admin.enable": "有効にする",
  "admin.disable": "無効にする",
  "admin.currentAccount": "使用中のアカウントは無効にできません",
  "admin.libraryError": "ライブラリのパスを登録できませんでした。",
  "admin.accountError": "アカウントを作成できませんでした。",
  "admin.statusError": "アカウントの状態を変更できませんでした。",
  "admin.scanError": "スキャンを開始できませんでした。",
  "admin.scanQueued": "スキャン {id} を待機列に追加しました。",
  "admin.initialScan": "初回スキャン {id} を待機列に追加しました。",
  "admin.scanning": "スキャン中",
  "admin.nowPlaying": "いま再生中",
  "admin.nobodyListening": "いま聴いている人はいません。",
  "admin.tokens": "{username}の API トークン",
  "admin.tokenName": "用途",
  "admin.tokenCreate": "発行",
  "admin.tokenOnce":
    "今すぐコピーしてください。表示されるのはこの1回限りです：",
  "admin.tokenError": "トークンを発行できませんでした。",
  "admin.tokenRevoke": "無効化",
  "admin.tokenUsed": "使用済み",
  "admin.tokenUnused": "未使用",
  "admin.tokenNone": "このアカウントにトークンはありません。",
  "admin.members": "このライブラリを見られるユーザー",
  "admin.memberRole": "役割",
  "admin.memberRemove": "外す",
  "admin.memberOwner": "オーナー（変更不可）",
  "admin.memberAdd": "アクセス権を付与",
  "admin.memberChoose": "アカウントを選択…",
  "admin.memberError": "メンバーを変更できませんでした。",
  "admin.roleAdmin": "管理者",
  "admin.roleUser": "ユーザー",
  "admin.roleOwner": "オーナー",
  "admin.roleManager": "マネージャー",
  "admin.roleListener": "リスナー",
  "admin.accountLibraries": {
    one: "ライブラリ{count}件",
    other: "ライブラリ{count}件",
  },
  "admin.manage": "名前、フォルダ、削除",
  "admin.rename": "名前",
  "admin.path": "サーバー上のフォルダ",
  "admin.followLinks": "リンク先がライブラリ内にあるシンボリックリンクをたどる",
  "admin.watch":
    "フォルダを監視して、変更をスキャンする（ほかのマシンが変更するネットワーク共有ではオフ）",
  "admin.save": "保存",
  "admin.saved": "保存しました。",
  "admin.savedScan": "保存しました。スキャン {id} を待機列に追加しました。",
  "admin.saveError":
    "変更を保存できませんでした。フォルダはサーバー上に存在し、シンボリックリンクではない必要があります。",
  "admin.saveBusy":
    "ライブラリをスキャン中か、ほかのライブラリがすでにそのフォルダを使っています。スキャンが終わってから、もう一度お試しください。",
  "admin.delete": "このライブラリを削除",
  "admin.deleteNote":
    "すべてのメンバーについて、このライブラリをカタログから削除します。曲、アルバム、アーティストと、それらのプレイリスト内の項目、お気に入り、再生記録も削除されます。ディスク上のファイルには一切触れません。確認のため、ライブラリ名を入力してください。",
  "admin.deleteConfirm": "確認のためのライブラリ名",
  "admin.deleteButton": "完全に削除",
  "admin.deleted": "{name}を削除しました。{tracks}曲がカタログから外れました。",
  "admin.deleteError": "ライブラリを削除できませんでした。",
  "scan.connecting": "最初の読み取りを待っています…",
  "scan.queued": "待機中",
  "scan.running": "スキャン中",
  "scan.completed": "完了",
  "scan.failed": "失敗",
  "scan.progress": "スキャンの進行状況",
  "scan.counted": "{processed} / {total} ファイル",
  "scan.tally":
    "追加 {added} · 更新 {updated} · 移動 {moved} · 変更なし {skipped} · エラー {errors}",
  "scan.lost":
    "進行状況のストリームを開けませんでした。スキャン自体には影響ありません。",
  "scan.leftOut": "除外 {excluded} · たどらなかったリンク {links}",
  "maintenance.sections": "メンテナンスの項目",
  "maintenance.none":
    "このライブラリには、あなたがメンテナンスできるものはありません。",
  "missing.total": {
    one: "スキャンで見つからなくなったファイル：{count}件",
    other: "スキャンで見つからなくなったファイル：{count}件",
  },
  "missing.nothing":
    "すべてのファイルが、カタログの想定どおりの場所にあります。",
  "missing.notYours":
    "このライブラリの見つからないファイルを見られるのは、オーナーとマネージャー、管理者だけです。",
  "missing.note":
    "これらの曲は、カタログから削除するまで、お気に入り、プレイリスト内の項目、再生記録とともにカタログに残ります。その前にファイルが戻れば、曲もそのまま残ります。カタログから削除しても、しばらくの間は元に戻せます。その後は、曲と、すべてのメンバーについてそれに紐づくものが、すべて完全に失われます。",
  "missing.since": "{date}から見つかりません",
  "missing.cost":
    "プレイリスト {playlists} · お気に入り {favorites} · 再生 {plays}",
  "missing.select": "{title}を選択",
  "missing.purgeSelected": "選択した項目をカタログから削除（{count}）",
  "missing.purgeAll": "すべてカタログから削除",
  "missing.confirm": "確認：{count}件をカタログから削除",
  "missing.purged": {
    one: "{count}曲をカタログから削除しました。",
    other: "{count}曲をカタログから削除しました。",
  },
  "missing.scanning":
    "ライブラリをスキャン中です。スキャンが終わってから、もう一度お試しください。",
  "missing.failed":
    "カタログから削除できませんでした。もう一度お試しください。",
  "missing.more": "さらに表示",
  "missing.purgedUntil": {
    one: "{count}曲をカタログから削除しました。{date}まで元に戻せます。",
    other: "{count}曲をカタログから削除しました。{date}まで元に戻せます。",
  },
  "missing.purgedTitle": {
    one: "カタログから削除済み・まだ復元可能（{count}）",
    other: "カタログから削除済み・まだ復元可能（{count}）",
  },
  "missing.purgedNote":
    "まだ何も失われていません。復元した曲は、紐づいていたものすべてとともに見つからないファイルに戻ります。ファイルが戻ってきた場合は、その曲が自動的に復元されます。",
  "missing.deletedAfter": "{date}を過ぎると完全に失われます",
  "missing.restore": "復元",
  "missing.restoreOne": "{title}を復元",
  "missing.restoreAll": "すべて復元",
  "missing.restored": {
    one: "{count}曲を復元しました。",
    other: "{count}曲を復元しました。",
  },
  "missing.restoreFailed": "復元できませんでした。もう一度お試しください。",
};
