// Audio quality labels and the quality analysis.
import type { TranslationValue } from "..";
import type { quality as en } from "../en/quality";

export const quality: Record<keyof typeof en, TranslationValue> = {
  "quality.label": "파일 음질",
  "quality.unknown": "알 수 없음",
  "quality.low": "낮음",
  "quality.standard": "표준",
  "quality.high": "높음",
  "quality.lossless": "무손실",
  "quality.hiRes": "Hi-Res",
  "quality.help":
    "컨테이너 정보를 읽지 않고 오디오 자체를 측정해요. 고음역이 어디서 끊기는지, 샘플이 몇 비트를 쓰는지 확인해요. 고음역이 16kHz 근처에서 끊기는 무손실 파일은 FLAC이 되기 전에 128kbps MP3였던 파일이에요. ‘손실 원본의 흔적 없음’은 증거가 아니에요. 전체 대역을 유지하는 인코더는 찾아낼 흔적을 남기지 않아요.",
  "quality.progress": "무손실 파일 {lossless}개 중 {measured}개 분석 완료",
  "quality.disabled": "이 서버에서는 분석이 꺼져 있어요.",
  "quality.noLossless": "이 보관함에는 무손실 파일이 없어요.",
  "quality.status.running":
    "분석 중: {lossless}개 중 {measured}개 완료, {inFlight}개 진행 중.",
  "quality.status.waitingForScan": "스캔이 끝나면 분석을 시작해요.",
  "quality.status.queued": "대기 중: 곧 이 파일들을 분석해요.",
  "quality.status.idle": "대기 중. 마지막 파일 분석: {when}.",
  "quality.status.idleNever": "대기 중. 아직 분석한 파일이 없어요.",
  "quality.filters": "판정",
  "quality.all": "모든 파일",
  "quality.format": "형식",
  "quality.allFormats": "모든 형식",
  "quality.empty": "이 판정에 해당하는 파일이 없어요.",
  "quality.column.track": "트랙",
  "quality.column.container": "파일 정보",
  "quality.column.highs": "고음역 차단 지점",
  "quality.column.verdict": "오디오 측정 결과",
  "quality.fullBand": "전체 대역",
  "quality.khz": "{value}kHz",
  "quality.db": "{value}dB",
  "quality.analyseAgain": "다시 분석",
  "quality.confirmAnalyse": "확인: 보관함 전체 다시 분석",
  "quality.requested": "백그라운드에서 보관함을 다시 분석해요.",
  "quality.requestFailed": "실패했어요. 다시 시도하세요.",
  "quality.section": "실제 음질",
  "quality.verdictLabel": "판정",
  "quality.highsStop": "고음역 차단 지점",
  "quality.wall": "해당 지점의 감쇠",
  "quality.bitsUsed": "실제 사용 비트 (측정값)",
  "quality.bitsOf": "{declared} 중 {used}",
  "quality.bitsNote":
    "샘플을 측정한 결과일 뿐, 진짜 Hi-Res라는 증거는 아니에요. 선언한 비트를 모두 쓰는 파일은 빈 비트로 채워지지 않았다는 뜻이지, 그 비트 깊이로 녹음되었다는 뜻은 아니에요.",
  "quality.analysed": "분석 일시",
  "quality.chart": "평균 스펙트럼: 고음역이 {where}에서 끊겨요.",
  "quality.chartFullBand": "평균 스펙트럼: 고음역이 끊기지 않고 이어져요.",
  "quality.showSpectrogram": "스펙트로그램 보기",
  "quality.drawing": "그리는 중…",
  "quality.spectrogramAlt": "{title}의 스펙트로그램, 시간에 따른 주파수",
  "quality.spectrogramBusy":
    "이미 스펙트로그램을 그리고 있어요. 잠시 후 다시 시도하세요.",
  "quality.spectrogramFailed": "스펙트로그램을 그릴 수 없어요.",
  "quality.verdict.lossy_source": "손실 파일에서 변환된 것으로 추정",
  "quality.verdict.upsampled": "업샘플링된 것으로 추정",
  "quality.verdict.padded": "비트 깊이 부풀림",
  "quality.verdict.unreadable": "읽을 수 없음",
  "quality.verdict.silent": "무음",
  "quality.verdict.pending": "분석 대기 중",
  "quality.verdict.consistent": "손실 원본의 흔적 없음",
  "quality.verdict.lossy": "손실",
  "quality.verdict.not_measured": "측정 안 함",
  "quality.explain.lossy_source":
    "고음역이 손실 인코더가 잘라 내는 지점에서 벽처럼 끊겨요. 이 파일은 MP3, AAC, Opus 파일을 디코딩해 무손실로 저장한 것일 가능성이 높아요.",
  "quality.explain.upsampled":
    "샘플레이트는 48kHz보다 높지만 고음역이 CD나 48kHz 녹음이 끝나는 지점에서 끊겨요. 그런 원본을 리샘플링한 것일 가능성이 높아요.",
  "quality.explain.padded":
    "컨테이너에 선언된 비트 수가 샘플이 실제로 쓰는 비트보다 많아요. 예를 들어 16비트 녹음을 24비트로 저장한 경우예요.",
  "quality.explain.unreadable":
    "FFmpeg가 이 파일을 디코딩할 수 없었어요. 파일이 바뀌면 다시 시도해요.",
  "quality.explain.silent": "이 파일에는 측정할 만큼 큰 소리가 없어요.",
  "quality.explain.pending":
    "이 파일은 아직 분석하지 않았어요. 분석은 백그라운드에서 진행돼요.",
  "quality.explain.consistent":
    "오디오에서 컨테이너 정보와 어긋나는 점을 찾지 못했어요. 하지만 증거는 아니에요. 전체 대역을 유지하는 인코더는 찾아낼 흔적을 남기지 않아요.",
  "quality.explain.lossy":
    "손실 파일은 스스로 손실 파일임을 밝혀요. 음질은 인코더와 비트레이트가 결정하므로 분석하지 않아요.",
  "quality.explain.not_measured": "이 형식은 분석하지 않아요.",
};
