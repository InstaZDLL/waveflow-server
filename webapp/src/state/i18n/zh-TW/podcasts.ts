// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail": "這台伺服器為所有帳號訂閱的節目",
  "podcasts.off":
    "這台伺服器已關閉 Podcast 功能。先前保存的內容仍可收聽，但不會再取得新內容。",
  "podcasts.url": "訂閱來源網址",
  "podcasts.shared":
    "所有帳號都能看到：這台伺服器上的任何人都能看到這個節目，並播放它已下載的單集。只有管理員能看到訂閱來源網址。",
  "podcasts.subscribe": "訂閱",
  "podcasts.refresh": "重新整理所有訂閱來源",
  "podcasts.refreshing": "正在重新取得訂閱來源。",
  "podcasts.reload": "重新載入",
  "podcasts.empty": "還沒有 Podcast。",
  "podcasts.emptyDetail":
    "管理員可以在這裡訂閱節目的訂閱來源。伺服器會下載它的單集，這台伺服器上的每個帳號都能收聽。",
  "podcasts.addFirst": "新增第一個 Podcast",
  "podcasts.channels": "節目",
  "podcasts.showing": "正在顯示 {title} 的單集。",
  "podcasts.noEpisodes": "這個訂閱來源沒有單集。",
  "podcasts.episodes": {
    one: "{count} 個單集",
    other: "{count} 個單集",
  },
  "podcasts.failed": "上次取得失敗：{reason}",
  "podcasts.open": "單集",
  "podcasts.hide": "收合",
  "podcasts.unsubscribe": "取消訂閱",
  "podcasts.download": "下載",
  "podcasts.play": "播放 {title}",
  "podcasts.pause": "暫停 {title}",
  "podcasts.resumeAt": "從 {time} 繼續",
  "podcasts.status.new": "未下載",
  "podcasts.status.downloading": "下載中",
  "podcasts.status.completed": "可以播放",
  "podcasts.status.error": "下載失敗",
  "podcasts.status.deleted": "已刪除",
  "podcasts.conflict": "這台伺服器已經訂閱了這個訂閱來源。",
  "podcasts.refused":
    "無法取得或讀取這個訂閱來源，或是它的網址不在這台伺服器允許連線的範圍內。",
  "podcasts.subscribeError": "無法新增這個訂閱來源。",
  "podcasts.mutationError": "操作失敗，請再試一次。",
  "podcasts.playbackError": "無法播放這個單集。",
};
