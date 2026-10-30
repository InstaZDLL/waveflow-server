// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "你的音乐，你做主",
  "login.heroTitle": "夜幕降临，音乐库苏醒。",
  "login.heroDetail": "私人串流，一个曲库，所有屏幕。",
  "login.server": "WaveFlow 服务器",
  "login.createAdmin": "创建管理员",
  "login.welcome": "欢迎回来",
  "login.setupDetail": "这是一台新服务器。请创建第一个管理员账户。",
  "login.setupToken": "初始化令牌",
  "login.setupTokenHint":
    "服务器已将它写入数据目录中的 setup-token 文件，日志中会给出该文件的路径。如果你设置了 WAVEFLOW_SETUP_TOKEN，请输入那个值。",
  "login.setupTokenError":
    "初始化令牌不正确。请从 setup-token 文件中重新复制。",
  "login.username": "用户名",
  "login.password": "密码",
  "login.setupError": "初始化失败。请使用有效的用户名，密码至少 12 个字符。",
  "login.error": "用户名或密码错误。",
  "login.wait": "请稍候…",
  "login.create": "创建并登录",
  "login.signIn": "登录",
  "login.offline": {
    one: "收听此浏览器中离线保存的 {count} 首曲目",
    other: "收听此浏览器中离线保存的 {count} 首曲目",
  },
};
