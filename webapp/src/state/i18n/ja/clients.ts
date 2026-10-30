// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "登録済みのクライアント",
  "clients.detail":
    "あなたのライブラリへの認証情報を持っているデバイスやスクリプトです。無効化すると、すぐに使えなくなります。",
  "clients.name": "用途",
  "clients.register": "登録",
  "clients.once":
    "今すぐコピーしてください。保存されるのはハッシュだけなので、表示されるのはこの1回限りです：",
  "clients.revoke": "無効化",
  "clients.none":
    "あなたのライブラリへの認証情報を持つクライアントはありません。",
  "clients.error": "うまくいきませんでした。何も変更されていません。",
  "clients.registered": "{when}に登録",
  "clients.used": "最終使用：{when}",
  "clients.unused": "未使用",
  "subsonic.title": "Subsonic パスワード",
  "subsonic.detail":
    "Subsonic クライアントのサインインに使う専用のパスワードです。変更すると API キーも置き換わるため、漏れたキーは使えなくなります。",
  "subsonic.set": "{when}に設定",
  "subsonic.changed": "最終変更：{when}",
  "subsonic.password": "新しいパスワード",
  "subsonic.rotate": "変更",
  "subsonic.revoke": "無効にする",
  "subsonic.confirm": "はい、無効にします",
  "subsonic.once":
    "この API キーを今すぐコピーしてください。保存されるのはハッシュだけなので、表示されるのはこの1回限りです：",
  "subsonic.gone":
    "各クライアントで設定し直すまで、Subsonic クライアントは使えなくなります。",
  "subsonic.none":
    "Subsonic パスワードがないため、Subsonic クライアントからライブラリを読み込めません。管理者に発行を依頼してください。",
  "subsonic.short": "12文字以上にしてください。",
  "subsonic.error": "うまくいきませんでした。何も変更されていません。",
  "credential.password": "Subsonic 専用パスワード",
  "credential.rotate": "認証情報を更新",
  "credential.copy": "この API キーを今すぐコピーしてください：",
  "credential.error": "認証情報を更新できませんでした。",
  "authorize.title": "アクセスの許可",
  "authorize.invalid":
    "この許可リンクは不完全か、信頼できない場所への転送を求めています。",
  "authorize.client": "{client}を許可",
  "authorize.detail":
    "ライブラリの閲覧、音楽の再生、プレイリスト・お気に入り・評価の管理など、このアカウントでできることがすべて行えるようになります。",
  "authorize.redirect": "{url} に戻ります",
  "authorize.error":
    "アプリケーションから、応じることのできない許可リクエストが送られました。",
  "authorize.busy": "許可しています…",
  "authorize.approve": "許可",
};
