// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "あなたの音楽を、あなたのやり方で",
  "login.heroTitle": "夜が来ると、ライブラリが目を覚ます。",
  "login.heroDetail":
    "プライベートなストリーミング、ひとつのカタログ、すべての画面で。",
  "login.server": "WaveFlow サーバー",
  "login.createAdmin": "管理者を作成",
  "login.welcome": "おかえりなさい",
  "login.setupDetail":
    "新しいサーバーです。最初の管理者アカウントを作成してください。",
  "login.setupToken": "セットアップトークン",
  "login.setupTokenHint":
    "サーバーはこのトークンをデータディレクトリ内の setup-token ファイルに書き出し、そのパスをログに記録しています。WAVEFLOW_SETUP_TOKEN を設定した場合は、その値を入力してください。",
  "login.setupTokenError":
    "セットアップトークンが正しくありません。setup-token ファイルからもう一度コピーしてください。",
  "login.username": "ユーザー名",
  "login.password": "パスワード",
  "login.setupError":
    "セットアップに失敗しました。有効なユーザー名と、12文字以上のパスワードを使用してください。",
  "login.error": "ユーザー名またはパスワードが正しくありません。",
  "login.wait": "お待ちください…",
  "login.create": "作成してサインイン",
  "login.signIn": "サインイン",
  "login.offline": {
    one: "このブラウザに保存した{count}曲を聴く",
    other: "このブラウザに保存した{count}曲を聴く",
  },
};
