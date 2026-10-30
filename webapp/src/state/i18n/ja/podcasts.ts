// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail":
    "このサーバーがすべてのアカウントのためにフォローしている番組",
  "podcasts.off":
    "このサーバーではポッドキャストがオフになっています。これまでに保存したものは引き続き利用できますが、新しいものは取得されません。",
  "podcasts.url": "フィードのアドレス",
  "podcasts.shared":
    "すべてのアカウントに表示されます。このサーバーの誰もが番組を見られ、ダウンロードされたエピソードを再生できます。フィードのアドレスを見られるのは管理者だけです。",
  "podcasts.subscribe": "登録",
  "podcasts.refresh": "すべてのフィードを更新",
  "podcasts.refreshing": "フィードを取得し直しています。",
  "podcasts.reload": "再読み込み",
  "podcasts.empty": "ポッドキャストはまだありません。",
  "podcasts.emptyDetail":
    "管理者がここで番組のフィードを登録します。サーバーがエピソードをダウンロードし、このサーバーのすべてのアカウントで聴けるようになります。",
  "podcasts.addFirst": "最初のポッドキャストを追加",
  "podcasts.channels": "番組",
  "podcasts.showing": "{title}のエピソードを表示しています。",
  "podcasts.noEpisodes": "このフィードにはエピソードがありません。",
  "podcasts.episodes": {
    one: "{count}エピソード",
    other: "{count}エピソード",
  },
  "podcasts.failed": "前回の取得に失敗しました：{reason}",
  "podcasts.open": "エピソード",
  "podcasts.hide": "隠す",
  "podcasts.unsubscribe": "登録解除",
  "podcasts.download": "ダウンロード",
  "podcasts.play": "{title}を再生",
  "podcasts.pause": "{title}を一時停止",
  "podcasts.resumeAt": "{time}から再開",
  "podcasts.status.new": "未ダウンロード",
  "podcasts.status.downloading": "ダウンロード中",
  "podcasts.status.completed": "再生可能",
  "podcasts.status.error": "ダウンロード失敗",
  "podcasts.status.deleted": "削除済み",
  "podcasts.conflict": "このサーバーはすでにそのフィードをフォローしています。",
  "podcasts.refused":
    "フィードを取得または読み込めなかったか、このサーバーから接続できないアドレスです。",
  "podcasts.subscribeError": "フィードを追加できませんでした。",
  "podcasts.mutationError": "うまくいきませんでした。もう一度お試しください。",
  "podcasts.playbackError": "このエピソードを再生できませんでした。",
};
