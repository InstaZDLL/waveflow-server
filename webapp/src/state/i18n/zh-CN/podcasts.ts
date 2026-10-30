// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  // Podcasts (RFC-025).
  "podcasts.detail": "此服务器为所有账户订阅的节目",
  "podcasts.off":
    "此服务器已关闭播客功能。之前保存的内容仍可收听，但不会再获取新内容。",
  "podcasts.url": "订阅源地址",
  "podcasts.shared":
    "对所有账户可见：此服务器上的任何人都能看到该节目，并播放它已下载的单集。只有管理员能看到订阅源地址。",
  "podcasts.subscribe": "订阅",
  "podcasts.refresh": "刷新全部订阅源",
  "podcasts.refreshing": "正在重新获取订阅源。",
  "podcasts.reload": "重新加载",
  "podcasts.empty": "还没有播客。",
  "podcasts.emptyDetail":
    "管理员可以在这里订阅节目的订阅源。服务器会下载它的单集，此服务器上的每个账户都能收听。",
  "podcasts.addFirst": "添加第一个播客",
  "podcasts.channels": "节目",
  "podcasts.showing": "正在显示 {title} 的单集。",
  "podcasts.noEpisodes": "此订阅源没有单集。",
  "podcasts.episodes": {
    one: "{count} 个单集",
    other: "{count} 个单集",
  },
  "podcasts.failed": "上次获取失败：{reason}",
  "podcasts.open": "单集",
  "podcasts.hide": "收起",
  "podcasts.unsubscribe": "取消订阅",
  "podcasts.download": "下载",
  "podcasts.play": "播放 {title}",
  "podcasts.pause": "暂停 {title}",
  "podcasts.resumeAt": "从 {time} 继续",
  "podcasts.status.new": "未下载",
  "podcasts.status.downloading": "下载中",
  "podcasts.status.completed": "可播放",
  "podcasts.status.error": "下载失败",
  "podcasts.status.deleted": "已删除",
  "podcasts.conflict": "此服务器已经订阅了该订阅源。",
  "podcasts.refused":
    "无法获取或读取该订阅源，或者它的地址不在此服务器允许访问的范围内。",
  "podcasts.subscribeError": "无法添加该订阅源。",
  "podcasts.mutationError": "操作失败，请重试。",
  "podcasts.playbackError": "无法播放此单集。",
};
