// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "外部播放记录同步",
  "scrobbling.detail":
    "把你在这里听过的歌曲同步到 ListenBrainz、Maloja 或 Last.fm。",
  "scrobbling.none": "这台服务器没有提供任何同步目标，这由服务器管理员决定。",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "实例“{name}”",
  "scrobbling.unavailable": "此处不可用：{reason}",
  "scrobbling.unavailable.no_application_configured":
    "这台服务器没有配置 Last.fm 应用",
  "scrobbling.unavailable.browser_journey_needs_https":
    "通过浏览器授权需要这台服务器的公开地址使用 https；管理员仍可以在命令行中关联账户",
  "scrobbling.unavailable.unknown": "服务器没有说明原因",
  "scrobbling.retired":
    "这台服务器已不再提供这个实例，之后不会再向它发送任何记录。你仍然可以撤销授权。",
  "scrobbling.notLinked": "未关联。",
  "scrobbling.secret": "API 密钥",
  "scrobbling.secretHint": "在你于该服务的账户中获取。",
  "scrobbling.link": "关联",
  "scrobbling.working": "处理中…",
  "scrobbling.linkError": "授权被拒绝。请检查密钥后重试。",
  "scrobbling.unlink": "取消关联",
  "scrobbling.unlinkError": "无法取消这个关联。",
  "scrobbling.connect": "连接 Last.fm",
  "scrobbling.connectDetail":
    "Last.fm 不提供可直接粘贴的密钥，会先跳转到 Last.fm 让你确认，再带你回到这里。",
  "scrobbling.connectError": "无法打开 Last.fm 授权页面。",
  "scrobbling.returned": "已关联 Last.fm：{name}。",
  "scrobbling.health.healthy": "正常同步中",
  "scrobbling.health.degraded": "队列停滞",
  "scrobbling.health.broken": "已中断",
  "scrobbling.healthDetail.healthy": "没有卡住的记录。",
  "scrobbling.healthDetail.degraded":
    "关联仍有响应，但播放记录没有发送出去：可能有一条在等你处理，也可能最早的一条已经等得太久。",
  "scrobbling.healthDetail.broken":
    "凭据被拒绝，或者这台服务器已无法通过关联时使用的名称访问该实例。",
  "scrobbling.pending": { one: "{count} 条等待中", other: "{count} 条等待中" },
  "scrobbling.retrying": {
    one: "{count} 条重试中",
    other: "{count} 条重试中",
  },
  "scrobbling.uncertainCount": {
    one: "{count} 条待你处理",
    other: "{count} 条待你处理",
  },
  "scrobbling.oldest": "最早的一条从 {when} 起一直在等待。",
  "scrobbling.lastSuccess": "上次成功：{when}。",
  "scrobbling.lastFailure": "上次被拒：{reason}。",
  "scrobbling.uncertain": "需要你决定的播放记录",
  "scrobbling.uncertainDetail":
    "这些记录发出后连接中断了，所以无法确定对方是否已经收到。重新发送可能让它在别人也能看到的播放历史中出现两次；丢弃则会留下空缺。服务器不会替你决定，而且只会询问一次。",
  "scrobbling.uncertainNone": "没有需要你处理的记录。",
  "scrobbling.playedAt": "播放于 {when}",
  "scrobbling.attempts": {
    one: "已尝试 {count} 次",
    other: "已尝试 {count} 次",
  },
  "scrobbling.retry": "重新发送",
  "scrobbling.discard": "丢弃",
  "scrobbling.answered": "已处理。",
  "scrobbling.answerError": "无法处理这条播放记录。请刷新页面后再试。",
  "scrobbling.unnamedTrack": "这个账户保留的播放历史中已没有这首曲目",
  "scrobbling.reason.rate_limited": "对方要求降低发送频率",
  "scrobbling.reason.retryable": "对方没有接收",
  "scrobbling.reason.auth_broken": "授权被拒绝",
  "scrobbling.reason.rejected": "对方收到后拒绝了这条记录",
  "scrobbling.reason.destination_gone": "服务器已无法通过这个名称访问该实例",
  "scrobbling.reason.credential_unreadable": "服务器无法读取保存的凭据",
  "scrobbling.reason.attempts_exhausted": "重试次数过多",
  "scrobbling.reason.unlinked": "关联已取消",
  "scrobbling.reason.ambiguous": "一直没有收到回应",
  "scrobbling.reason.interrupted": "服务器在发送途中停止了",
};
