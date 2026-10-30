// Words shared across screens, file sizes, card buttons and ratings.
import type { TranslationValue } from "..";
import type { common as en } from "../en/common";

export const common: Record<keyof typeof en, TranslationValue> = {
  "common.loading": "불러오는 중",
  "common.loadError": "이 화면을 불러올 수 없어요",
  "common.noAlbums": "아직 앨범이 없어요.",
  "common.noArtists": "아직 아티스트가 없어요.",
  "common.variousArtists": "여러 아티스트",
  "common.unknownArtist": "알 수 없는 아티스트",
  "common.album": "앨범",
  "common.artist": "아티스트",
  "common.albums": { one: "앨범 {count}개", other: "앨범 {count}개" },
  "common.tracks": { one: "{count}곡", other: "{count}곡" },
  "common.delete": "삭제",
  "common.cancel": "취소",
  "common.back": "뒤로",
  "file.megabytes": "{size}MB",
  "file.kilobytes": "{size}kB",
  "file.channels": { one: "모노", other: "{count}채널" },
  "card.play": "재생",
  "card.queue": "재생 대기열에 추가",
  "rating.label": "별점",
  "rating.stars": { one: "별 {count}개", other: "별 {count}개" },
  "source.offer":
    "WaveFlow Server {version}: AGPL-3.0 라이선스의 자유 소프트웨어예요.",
  "source.link": "소스 코드",
};
