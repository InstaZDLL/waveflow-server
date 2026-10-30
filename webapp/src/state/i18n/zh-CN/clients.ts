// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "已注册的客户端",
  "clients.detail":
    "持有你音乐库访问凭据的设备和脚本。撤销后会立即停止其访问。",
  "clients.name": "用途",
  "clients.register": "注册",
  "clients.once": "请现在复制。服务器只保存它的哈希值，因此只会显示这一次：",
  "clients.revoke": "撤销",
  "clients.none": "没有客户端持有你音乐库的访问凭据。",
  "clients.error": "操作失败，未做任何更改。",
  "clients.registered": "注册于 {when}",
  "clients.used": "上次使用于 {when}",
  "clients.unused": "从未使用",
  "subsonic.title": "Subsonic 密码",
  "subsonic.detail":
    "Subsonic 客户端登录时使用的独立密码。修改它也会同时更换 API 密钥，泄露的旧密钥将随之失效。",
  "subsonic.set": "设置于 {when}",
  "subsonic.changed": "上次修改于 {when}",
  "subsonic.password": "新密码",
  "subsonic.rotate": "修改",
  "subsonic.revoke": "关闭",
  "subsonic.confirm": "确定关闭",
  "subsonic.once":
    "请现在复制此 API 密钥。服务器只保存它的哈希值，因此只会显示这一次：",
  "subsonic.gone":
    "在你重新设置密码并逐一填入各个 Subsonic 客户端之前，它们都将无法使用。",
  "subsonic.none":
    "你还没有 Subsonic 密码，因此任何 Subsonic 客户端都无法读取你的音乐库。请向管理员申请。",
  "subsonic.short": "至少需要 12 个字符。",
  "subsonic.error": "操作失败，未做任何更改。",
  "credential.password": "Subsonic 专用密码",
  "credential.rotate": "更换凭据",
  "credential.copy": "请现在复制此 API 密钥：",
  "credential.error": "更换凭据失败。",
  "authorize.title": "授权",
  "authorize.invalid": "此授权链接不完整，或试图将你转到不受信任的地址。",
  "authorize.client": "授权 {client}",
  "authorize.detail":
    "它将能够浏览你的音乐库、播放你的音乐，并管理你的歌单、收藏和评分，即此账户能做的一切。",
  "authorize.redirect": "正在返回 {url}",
  "authorize.error": "该应用发送的授权请求无法处理。",
  "authorize.busy": "正在授权…",
  "authorize.approve": "授权",
};
