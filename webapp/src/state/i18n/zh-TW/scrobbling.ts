// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "外部播放紀錄同步",
  "scrobbling.detail":
    "把你在這裡聽過的歌曲同步到 ListenBrainz、Maloja 或 Last.fm。",
  "scrobbling.none":
    "這台伺服器沒有提供任何同步目的地，這由伺服器的維運人員決定。",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "執行個體「{name}」",
  "scrobbling.unavailable": "在這裡無法使用：{reason}",
  "scrobbling.unavailable.no_application_configured":
    "這台伺服器沒有設定 Last.fm 應用程式",
  "scrobbling.unavailable.browser_journey_needs_https":
    "要透過瀏覽器授權，這台伺服器的公開網址必須使用 https；維運人員仍可以從命令列連結帳號",
  "scrobbling.unavailable.unknown": "這台伺服器沒有說明原因",
  "scrobbling.retired":
    "這台伺服器已不再提供這個執行個體，之後不會再傳送任何紀錄給它。你仍然可以撤銷授權。",
  "scrobbling.notLinked": "尚未連結。",
  "scrobbling.secret": "API 金鑰",
  "scrobbling.secretHint": "從你在該服務的帳號中取得。",
  "scrobbling.link": "連結",
  "scrobbling.working": "處理中…",
  "scrobbling.linkError": "授權遭到拒絕。請檢查金鑰後再試一次。",
  "scrobbling.unlink": "取消連結",
  "scrobbling.unlinkError": "無法取消這個連結。",
  "scrobbling.connect": "連接 Last.fm",
  "scrobbling.connectDetail":
    "Last.fm 不提供可以直接貼上的金鑰，所以會先帶你到 Last.fm 確認，再帶你回到這裡。",
  "scrobbling.connectError": "無法開啟 Last.fm 授權頁面。",
  "scrobbling.returned": "已連結 Last.fm：{name}。",
  "scrobbling.health.healthy": "正常同步中",
  "scrobbling.health.degraded": "佇列停滯",
  "scrobbling.health.broken": "已中斷",
  "scrobbling.healthDetail.healthy": "沒有卡住的紀錄。",
  "scrobbling.healthDetail.degraded":
    "連結仍有回應，但播放紀錄沒有傳送出去：可能有一筆在等你處理，也可能最早的一筆已經等太久了。",
  "scrobbling.healthDetail.broken":
    "憑證遭到拒絕，或這台伺服器已經無法透過建立連結時使用的名稱連線到該執行個體。",
  "scrobbling.pending": { one: "{count} 筆等待中", other: "{count} 筆等待中" },
  "scrobbling.retrying": {
    one: "{count} 筆重試中",
    other: "{count} 筆重試中",
  },
  "scrobbling.uncertainCount": {
    one: "{count} 筆待你處理",
    other: "{count} 筆待你處理",
  },
  "scrobbling.oldest": "最早的一筆從 {when} 起就一直在等待。",
  "scrobbling.lastSuccess": "上次成功：{when}。",
  "scrobbling.lastFailure": "上次遭拒：{reason}。",
  "scrobbling.uncertain": "需要由你決定的播放紀錄",
  "scrobbling.uncertainDetail":
    "這些紀錄送出後連線就中斷了，所以無法確定對方是否已經記錄下來。重新傳送可能會讓它在其他人也看得到的收聽紀錄中出現兩次；捨棄則會留下空缺。這台伺服器不會替你決定，而且只會詢問一次。",
  "scrobbling.uncertainNone": "沒有需要你處理的紀錄。",
  "scrobbling.playedAt": "播放於 {when}",
  "scrobbling.attempts": {
    one: "已嘗試 {count} 次",
    other: "已嘗試 {count} 次",
  },
  "scrobbling.retry": "重新傳送",
  "scrobbling.discard": "捨棄",
  "scrobbling.answered": "已處理。",
  "scrobbling.answerError": "無法處理這筆播放紀錄。請重新載入頁面後再試。",
  "scrobbling.unnamedTrack": "這個帳號保留的收聽紀錄中已沒有這首曲目",
  "scrobbling.reason.rate_limited": "對方要求降低傳送頻率",
  "scrobbling.reason.retryable": "對方沒有接收",
  "scrobbling.reason.auth_broken": "授權遭到拒絕",
  "scrobbling.reason.rejected": "對方讀取後拒絕了這筆紀錄",
  "scrobbling.reason.destination_gone":
    "這台伺服器已經無法透過這個名稱連線到該執行個體",
  "scrobbling.reason.credential_unreadable": "這台伺服器無法讀取儲存的憑證",
  "scrobbling.reason.attempts_exhausted": "嘗試次數過多",
  "scrobbling.reason.unlinked": "連結已取消",
  "scrobbling.reason.ambiguous": "一直沒有收到回應",
  "scrobbling.reason.interrupted": "伺服器在傳送途中停止了",
};
