// Folders, filing and uploads.
import type { TranslationValue } from "..";
import type { folders as en } from "../en/folders";

export const folders: Record<keyof typeof en, TranslationValue> = {
  "folders.count": {
    one: "このライブラリに作成されたフォルダ：{count}件",
    other: "このライブラリに作成されたフォルダ：{count}件",
  },
  "folders.where": "フォルダのパス",
  "folders.subfolders": "フォルダ",
  "folders.noLibrary": "このアカウントで使えるライブラリはありません。",
  "folders.none": "このライブラリにはまだフォルダが作成されていません。",
  "folders.system": "WaveFlow が管理",
  "folders.systemNote":
    "「未整理」には、どのフォルダにも入っていない曲がすべて集まります。WaveFlow が自動で管理するため、上のフォルダの数には含まれず、名前の変更、移動、削除はできません。",
  "filed.unfiled": "未整理",
  "filed.automatic": "自動",
  "filed.topLevel": "最上位",
  "filed.here": "現在の場所",
  "filed.destinations": "移動先のフォルダ",
  "filed.loadError": "フォルダを読み込めませんでした。",
  "filed.moveFailed": "移動できませんでした。もう一度お試しください。",
  "filed.moveTrack": "「{title}」をフォルダに移動",
  "filed.moveFolder": "「{name}」を移動",
  "filed.note":
    "WaveFlow の中だけにあるフォルダです。ここに曲を整理しても、ファイルは移動しません。",
  "filed.newFolder": "新規フォルダ",
  "filed.name": "フォルダ名",
  "filed.renameLabel": "新しい名前",
  "filed.create": "作成",
  "filed.save": "保存",
  "filed.cancel": "キャンセル",
  "filed.rename": "名前を変更",
  "filed.move": "移動…",
  "filed.delete": "削除",
  "filed.confirmDelete":
    "「{name}」とそのサブフォルダを削除しますか？中の曲は「未整理」に戻ります。ファイルには一切触れません。",
  "filed.created": "「{name}」を作成しました。",
  "filed.renamed": "名前を「{name}」に変更しました。",
  "filed.deleted": {
    one: "フォルダを削除しました。{count}曲が「未整理」に戻りました。",
    other: "フォルダを削除しました。{count}曲が「未整理」に戻りました。",
  },
  "filed.filed": {
    one: "{count}曲を{folder}に整理しました。",
    other: "{count}曲を{folder}に整理しました。",
  },
  "filed.folderMoved": "「{name}」を移動しました。",
  "filed.nameTaken": "同じ名前のフォルダがすでにあります。",
  "filed.refused":
    "そのフォルダはそこに移動できません。名前を確認してください。または、フォルダの階層が深すぎるか、いっぱいです。",
  "filed.failed": "うまくいきませんでした。もう一度お試しください。",
  "filed.missing": "このフォルダはもう存在しません。",
  "filed.empty": "ここにはまだ何も整理されていません。",
  "filed.allFiled": "すべての曲が整理されています。",
  "filed.uploadHere": "ここにアップロード",
  "filed.unfiledNote":
    "まだどのフォルダにも入っていない曲すべてです。WaveFlow がこのリストを自動で管理しており、曲はフォルダに整理されるとすぐにここから外れます。名前の変更や削除はできません。曲を整理するには、曲のメニューを開いて「フォルダに移動…」を選んでください。",
  "filed.unfiledNoteReader":
    "まだどのフォルダにも入っていない曲すべてです。WaveFlow がこのリストを自動で管理しており、曲はフォルダに整理されるとすぐにここから外れます。",
  "upload.into": "受信したファイルは {folder} フォルダに入ります。",
  "upload.doneUnfiled": "ライブラリの「未整理」に追加しました",
  "upload.folderMissing":
    "このアップロードの送り先のフォルダが見つかりません。受信したファイルは「未整理」に入ります。",
  "upload.folderUnchecked":
    "このアップロードの送り先のフォルダを確認できなかったため、まだファイルを受け付けていません。",
  "upload.checkAgain": "もう一度確認",
  "upload.title": "{library}にアップロード",
  "upload.detail":
    "各ファイルはまずブラウザでフィンガープリントを取るため、ライブラリにすでにあるファイルが再び送られることはありません。中断した転送は、止まったところから再開します。",
  "upload.pick": "オーディオファイルを選択",
  "upload.formats": "ライブラリが取り込める形式：{extensions}",
  "upload.noLibrary": "まずライブラリを選択してください。",
  "upload.notAllowed":
    "このライブラリにファイルを追加できるのは、オーナーかマネージャーだけです。",
  "upload.closed":
    "このライブラリはファイルを受け付けていません。これはサーバーの運用者が決めています。",
  "upload.waiting": "待機中",
  "upload.hashing": "フィンガープリントを取得中…",
  "upload.negotiating": "サーバーに問い合わせ中…",
  "upload.uploading": "送信中…",
  "upload.committing": "ライブラリに追加中…",
  "upload.done": "ライブラリに追加しました",
  "upload.present": "すでにライブラリにあります",
  "upload.refused": "拒否されました：{reason}",
  "upload.failed": "失敗しました：{reason}",
  "upload.progress": "進行状況：{name}",
  "upload.reason.unsupportedFormat":
    "ライブラリが取り込めるオーディオ形式ではありません",
  "upload.reason.tooLarge": "サーバーが受け付けるサイズを超えています",
  "upload.reason.quotaExceeded": "ライブラリに空き容量がありません",
  "upload.reason.libraryClosed": "このライブラリはファイルを受け付けていません",
  "upload.reason.tooManySessions":
    "開いている転送が多すぎます。少し待ってからもう一度お試しください",
  "upload.reason.refusedFile":
    "受信した内容がファイルと一致しないか、ライブラリが読み込めるオーディオではありません",
  "upload.reason.unavailable":
    "サーバーが今は受け付けられませんでした。もう一度お試しください",
  "upload.reason.network": "接続が切れました",
  "upload.reason.notAllowed":
    "このライブラリにファイルを追加する権限がなくなりました",
  "upload.reason.error": "問題が発生しました",
  "upload.reason.hashing":
    "このブラウザではファイルのフィンガープリントを取得できませんでした",
};
