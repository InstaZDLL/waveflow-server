// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "已註冊的用戶端",
  "clients.detail":
    "持有你音樂庫存取憑證的裝置和指令碼。撤銷後會立即停止它的存取。",
  "clients.name": "用途",
  "clients.register": "註冊",
  "clients.once":
    "請立即複製。伺服器只會儲存它的雜湊值，所以只會顯示這一次，之後不會再出現：",
  "clients.revoke": "撤銷",
  "clients.none": "沒有用戶端持有你音樂庫的存取憑證。",
  "clients.error": "操作失敗，沒有做任何變更。",
  "clients.registered": "註冊於 {when}",
  "clients.used": "上次使用於 {when}",
  "clients.unused": "從未使用",
  "subsonic.title": "Subsonic 密碼",
  "subsonic.detail":
    "Subsonic 用戶端登入時使用的獨立密碼。變更密碼也會一併更換 API 金鑰，外洩的舊金鑰就會失效。",
  "subsonic.set": "設定於 {when}",
  "subsonic.changed": "上次變更於 {when}",
  "subsonic.password": "新密碼",
  "subsonic.rotate": "變更",
  "subsonic.revoke": "關閉",
  "subsonic.confirm": "是，關閉",
  "subsonic.once":
    "請立即複製這組 API 金鑰。伺服器只會儲存它的雜湊值，所以只會顯示這一次，之後不會再出現：",
  "subsonic.gone":
    "在你重新設定密碼並逐一填入各個 Subsonic 用戶端之前，它們都將無法使用。",
  "subsonic.none":
    "你還沒有 Subsonic 密碼，所以任何 Subsonic 用戶端都無法讀取你的音樂庫。請向管理員申請。",
  "subsonic.short": "至少需要 12 個字元。",
  "subsonic.error": "操作失敗，沒有做任何變更。",
  "credential.password": "Subsonic 專用密碼",
  "credential.rotate": "更換憑證",
  "credential.copy": "請立即複製這組 API 金鑰：",
  "credential.error": "無法更換憑證。",
  "authorize.title": "授權",
  "authorize.invalid": "這個授權連結不完整，或要將你導向不受信任的位址。",
  "authorize.client": "授權 {client}",
  "authorize.detail":
    "它將能瀏覽你的音樂庫、播放你的音樂，並管理你的播放清單、收藏和評分，也就是這個帳號能做的一切。",
  "authorize.redirect": "正在將你導回 {url}",
  "authorize.error": "這個應用程式送出的授權要求無法處理。",
  "authorize.busy": "正在授權…",
  "authorize.approve": "授權",
};
