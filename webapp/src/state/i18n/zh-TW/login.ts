// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "你的音樂，由你作主",
  "login.heroTitle": "夜幕降臨，音樂庫甦醒。",
  "login.heroDetail": "私人串流，一個曲庫，所有螢幕。",
  "login.server": "WaveFlow 伺服器",
  "login.createAdmin": "建立管理員",
  "login.welcome": "歡迎回來",
  "login.setupDetail": "這是一台新的伺服器。請建立第一個管理員帳號。",
  "login.setupToken": "初始設定權杖",
  "login.setupTokenHint":
    "伺服器已將它寫入資料目錄中的 setup-token 檔案，記錄檔中會列出該檔案的路徑。如果你設定了 WAVEFLOW_SETUP_TOKEN，請輸入那個值。",
  "login.setupTokenError":
    "初始設定權杖不正確。請從 setup-token 檔案重新複製。",
  "login.username": "使用者名稱",
  "login.password": "密碼",
  "login.setupError":
    "初始設定失敗。請使用有效的使用者名稱，密碼至少 12 個字元。",
  "login.error": "使用者名稱或密碼錯誤。",
  "login.wait": "請稍候…",
  "login.create": "建立並登入",
  "login.signIn": "登入",
  "login.offline": {
    one: "收聽這個瀏覽器中離線保存的 {count} 首歌曲",
    other: "收聽這個瀏覽器中離線保存的 {count} 首歌曲",
  },
};
