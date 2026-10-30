// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "外部への Scrobble",
  "scrobbling.detail":
    "ここで聴いた曲を ListenBrainz、Maloja、Last.fm に送信します。",
  "scrobbling.none":
    "このサーバーには送信先がありません。これはサーバーの運用者が決めています。",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "インスタンス「{name}」",
  "scrobbling.unavailable": "ここでは利用できません：{reason}",
  "scrobbling.unavailable.no_application_configured":
    "このサーバーには Last.fm アプリケーションが登録されていません",
  "scrobbling.unavailable.browser_journey_needs_https":
    "ブラウザでの連携には、このサーバーの公開アドレスが https である必要があります。運用者はコマンドラインからアカウントを連携できます",
  "scrobbling.unavailable.unknown": "このサーバーは理由を示しませんでした",
  "scrobbling.retired":
    "このサーバーはこのインスタンスを提供しなくなりました。今後ここへ送信されることはありません。認可の取り消しは引き続き行えます。",
  "scrobbling.notLinked": "未連携です。",
  "scrobbling.secret": "API キー",
  "scrobbling.secretHint": "送信先のアカウントから取得してください。",
  "scrobbling.link": "連携",
  "scrobbling.working": "処理中…",
  "scrobbling.linkError":
    "認可が拒否されました。キーを確認して、もう一度お試しください。",
  "scrobbling.unlink": "連携解除",
  "scrobbling.unlinkError": "連携を解除できませんでした。",
  "scrobbling.connect": "Last.fm に接続",
  "scrobbling.connectDetail":
    "Last.fm では貼り付けられるキーが発行されないため、Last.fm のサイトで確認してからここに戻ります。",
  "scrobbling.connectError": "Last.fm との連携を開始できませんでした。",
  "scrobbling.returned": "Last.fm と連携しました：{name}。",
  "scrobbling.health.healthy": "送信中",
  "scrobbling.health.degraded": "送信が止まっています",
  "scrobbling.health.broken": "エラー",
  "scrobbling.healthDetail.healthy": "滞っているものはありません。",
  "scrobbling.healthDetail.degraded":
    "連携先は応答していますが、再生記録が届いていません。あなたの判断を待っているものがあるか、最も古いものが長く待ちすぎています。",
  "scrobbling.healthDetail.broken":
    "認証情報が拒否されたか、連携したときの名前ではこのサーバーからそのインスタンスに接続できなくなりました。",
  "scrobbling.pending": { one: "待機中 {count}件", other: "待機中 {count}件" },
  "scrobbling.retrying": {
    one: "再試行中 {count}件",
    other: "再試行中 {count}件",
  },
  "scrobbling.uncertainCount": {
    one: "あなたの判断待ち {count}件",
    other: "あなたの判断待ち {count}件",
  },
  "scrobbling.oldest": "最も古いものは{when}から待っています。",
  "scrobbling.lastSuccess": "最後に受け付けられたのは{when}です。",
  "scrobbling.lastFailure": "最後の拒否：{reason}。",
  "scrobbling.uncertain": "あなたにしか判断できない再生記録",
  "scrobbling.uncertainDetail":
    "これらを送信した後に接続が切れたため、送信先が記録したかどうかはわかりません。もう一度送ると、ほかの人も見る再生履歴に二重に記録されるかもしれません。破棄すると、履歴に抜けができます。このサーバーが代わりに選ぶことはなく、確認は一度だけです。",
  "scrobbling.uncertainNone": "あなたの判断を待っているものはありません。",
  "scrobbling.playedAt": "{when}に再生",
  "scrobbling.attempts": { one: "{count}回試行", other: "{count}回試行" },
  "scrobbling.retry": "もう一度送信",
  "scrobbling.discard": "破棄",
  "scrobbling.answered": "対応しました。",
  "scrobbling.answerError":
    "この再生記録に対応できませんでした。ページを再読み込みしてから、もう一度お試しください。",
  "scrobbling.unnamedTrack": "このアカウントに残っている履歴にはありません",
  "scrobbling.reason.rate_limited": "送信先から間隔を空けるよう求められました",
  "scrobbling.reason.retryable": "送信先が受け付けませんでした",
  "scrobbling.reason.auth_broken": "認可が拒否されました",
  "scrobbling.reason.rejected": "送信先が内容を確認したうえで拒否しました",
  "scrobbling.reason.destination_gone":
    "その名前ではこのサーバーからインスタンスに接続できなくなりました",
  "scrobbling.reason.credential_unreadable":
    "このサーバーは保存された認証情報を読み込めませんでした",
  "scrobbling.reason.attempts_exhausted": "試行回数が多すぎました",
  "scrobbling.reason.unlinked": "連携が解除されました",
  "scrobbling.reason.ambiguous": "応答が届きませんでした",
  "scrobbling.reason.interrupted": "送信中にサーバーが停止しました",
};
