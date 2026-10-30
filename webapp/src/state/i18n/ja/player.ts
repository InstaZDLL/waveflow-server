// The player bar, now playing, radio and offline listening.
import type { TranslationValue } from "..";
import type { player as en } from "../en/player";

export const player: Record<keyof typeof en, TranslationValue> = {
  "player.error":
    "再生を開始できませんでした。サーバーを確認して、もう一度お試しください。",
  "player.previous": "前の曲",
  "player.play": "再生",
  "player.pause": "一時停止",
  "player.next": "次の曲",
  "player.seek": "シーク",
  "player.shuffle": "シャッフル",
  "player.repeatOff": "リピート：オフ",
  "player.repeatAll": "リピート：再生キュー全体",
  "player.repeatOne": "リピート：この曲",
  "player.volume": "音量",
  "player.mute": "ミュート",
  "player.unmute": "ミュート解除",
  "player.openAlbum": "アルバムを開く",
  "player.live": "ライブ",
  "player.liveRadio": "ライブラジオ",
  "player.stopLive": "ステーションを停止",
  "player.liveError": "このステーションを再生できませんでした。",
  "player.closeAside": "停止して閉じる",
  "playing.idle": "何も再生していません。",
  "playing.empty": "曲を再生すると、歌詞とともにここに表示されます。",
  "playing.viewLabel": "この曲の表示方法",
  "playing.viewDetail": "詳細",
  "playing.viewLyrics": "歌詞",
  "playing.viewImmersive": "没入",
  "playing.immersiveTitle": "没入表示",
  "playing.leaveImmersive": "没入表示を終了",
  "radio.detail": "アカウントに保存したインターネットラジオ局",
  "radio.name": "ステーション名",
  "radio.address": "ストリームのアドレス",
  "radio.homepage": "ステーションのページ（任意）",
  "radio.image": "ステーションの画像（任意）",
  "radio.imageHint":
    "保存したときにサーバーが一度だけダウンロードし、ジャケットと一緒に保管します。ブラウザが画像のホストに接続することはありません。",
  "radio.imageMissing":
    "画像を取得できませんでした。ステーションを保存すると、もう一度試します。",
  "radio.add": "ステーションを追加",
  "radio.save": "ステーションを保存",
  "radio.edit": "編集",
  "radio.replacing":
    "保存するとすべての項目が置き換わります。ページのアドレスを空にすると、保存されているアドレスは消去されます。",
  "radio.listen": "聴く",
  "radio.stop": "停止",
  "radio.visit": "ステーションのページ",
  "radio.empty":
    "ステーションはまだありません。追加すると、ここに保存されます。",
  "radio.addressRefused":
    "アドレスは http:// または https:// で始まる必要があります。",
  "radio.mixedContent":
    "このページは HTTPS で配信されているため、ブラウザは暗号化されていないストリームを拒否します。",
  "radio.full": "このアカウントに保存できるステーションの上限に達しています。",
  "radio.saveError": "ステーションを保存できませんでした。",
  "radio.deleteError": "ステーションを削除できませんでした。",
  "radio.playbackError":
    "このステーションを再生できませんでした。サーバーはステーションに接続しないため、これはブラウザとステーションの間の問題です。",
  "radio.stations": "ステーション",
  "radio.newTitle": "ステーションを追加",
  "radio.editTitle": "「{name}」を編集",
  "radio.listenTo": "{name}を聴く",
  "radio.stopNamed": "{name}を停止",
  "radio.actions": "{name}の操作",
  "radio.confirmDelete":
    "ステーション「{name}」を削除しますか？アドレスと画像は削除されます。",
  "offline.kicker": "このブラウザ内",
  "offline.title": "オフライン用に保存",
  "offline.detail":
    "これらの曲は、最後にサインインしたアカウント用にこのブラウザに保存されており、接続なしで再生できます。サインアウトすると、このブラウザから削除されます。曲のメニューから、さらに保存できます。",
  "offline.unsupported":
    "このブラウザでは、オフライン再生用に曲を保存できません。",
  "offline.back": "ライブラリに戻る",
  "offline.empty": "まだ何も保存されていません。曲のメニューから保存できます。",
  "offline.player": "オフラインプレーヤー",
  "offline.gone": "そのコピーはもうこのブラウザにありません。",
  "offline.forget": "{title}のオフラインコピーを削除",
  "offline.forgetShort": "削除",
  "offline.forgetDetail": "{size}が空きます",
};
