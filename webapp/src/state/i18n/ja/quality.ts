// Audio quality labels and the quality analysis.
import type { TranslationValue } from "..";
import type { quality as en } from "../en/quality";

export const quality: Record<keyof typeof en, TranslationValue> = {
  "quality.label": "ファイルの音質",
  "quality.unknown": "不明",
  "quality.low": "低",
  "quality.standard": "標準",
  "quality.high": "高",
  "quality.lossless": "ロスレス",
  "quality.hiRes": "ハイレゾ",
  "quality.help":
    "コンテナの情報を読むのではなく、音声そのものから測定します。高域がどこで途切れるか、サンプルが何ビットを使っているかです。高域が16 kHz付近で途切れているロスレスファイルは、FLAC になる前は128 kbps の MP3 でした。「非可逆音源の形跡なし」は証明ではありません。全帯域を残すエンコーダーでは、見つかる痕跡がないからです。",
  "quality.progress": "ロスレスファイル{lossless}件中{measured}件を分析済み",
  "quality.disabled": "このサーバーでは分析がオフになっています。",
  "quality.noLossless": "このライブラリにロスレスファイルはありません。",
  "quality.status.running":
    "分析中：{lossless}件中{measured}件が完了、{inFlight}件を処理中です。",
  "quality.status.waitingForScan": "スキャンが終わるのを待ってから分析します。",
  "quality.status.queued":
    "待機中：まもなくこれらのファイルの分析が始まります。",
  "quality.status.idle": "待機中です。最後にファイルを分析したのは{when}です。",
  "quality.status.idleNever": "待機中です。まだ分析したファイルはありません。",
  "quality.filters": "判定",
  "quality.all": "すべてのファイル",
  "quality.format": "形式",
  "quality.allFormats": "すべての形式",
  "quality.empty": "この判定のファイルはありません。",
  "quality.column.track": "曲",
  "quality.column.container": "ファイルの表示",
  "quality.column.highs": "高域の上限",
  "quality.column.verdict": "音声の実測",
  "quality.fullBand": "全帯域",
  "quality.khz": "{value} kHz",
  "quality.db": "{value} dB",
  "quality.analyseAgain": "再分析",
  "quality.confirmAnalyse": "確認：ライブラリ全体を再分析",
  "quality.requested": "ライブラリはバックグラウンドで再分析されます。",
  "quality.requestFailed": "うまくいきませんでした。もう一度お試しください。",
  "quality.section": "実際の音質",
  "quality.verdictLabel": "判定",
  "quality.highsStop": "高域の上限",
  "quality.wall": "上限での減衰",
  "quality.bitsUsed": "実際に使われているビット数（実測）",
  "quality.bitsOf": "{declared}中{used}",
  "quality.bitsNote":
    "サンプルの測定値であり、本物のハイレゾである証明ではありません。宣言したすべてのビットを使っているファイルは水増しされていないということで、そのビット深度で録音されたという意味ではありません。",
  "quality.analysed": "分析日時",
  "quality.chart": "平均スペクトル：高域は{where}で途切れています。",
  "quality.chartFullBand":
    "平均スペクトル：高域は途切れることなく続いています。",
  "quality.showSpectrogram": "スペクトログラムを表示",
  "quality.drawing": "描画中…",
  "quality.spectrogramAlt": "{title}のスペクトログラム（周波数と時間）",
  "quality.spectrogramBusy":
    "スペクトログラムをすでに描画中です。少し待ってからもう一度お試しください。",
  "quality.spectrogramFailed": "スペクトログラムを描画できませんでした。",
  "quality.verdict.lossy_source": "非可逆ファイルからの変換の可能性",
  "quality.verdict.upsampled": "アップサンプリングの可能性",
  "quality.verdict.padded": "ビット深度の水増し",
  "quality.verdict.unreadable": "読み込めませんでした",
  "quality.verdict.silent": "無音",
  "quality.verdict.pending": "分析待ち",
  "quality.verdict.consistent": "非可逆音源の形跡なし",
  "quality.verdict.lossy": "非可逆",
  "quality.verdict.not_measured": "未測定",
  "quality.explain.lossy_source":
    "高域が、非可逆エンコーダーがカットする位置で途切れています。このファイルは MP3、AAC、Opus のファイルからデコードされ、ロスレスとして保存された可能性があります。",
  "quality.explain.upsampled":
    "サンプルレートは48 kHzを超えていますが、高域は CD や48 kHzの録音が終わる位置で途切れています。それらからリサンプリングされた可能性があります。",
  "quality.explain.padded":
    "コンテナが、サンプルが実際に使っているより多くのビット数を宣言しています。たとえば、16ビットの録音を24ビットで保存した場合です。",
  "quality.explain.unreadable":
    "FFmpeg でこのファイルをデコードできませんでした。ファイルが変更されたときに、もう一度試します。",
  "quality.explain.silent":
    "このファイルには、測定できるほど大きな音がありません。",
  "quality.explain.pending":
    "このファイルはまだ分析されていません。分析はバックグラウンドで行われます。",
  "quality.explain.consistent":
    "音声の中に、コンテナの情報と矛盾するものはありません。ただし証明ではありません。全帯域を残すエンコーダーでは、見つかる痕跡がないからです。",
  "quality.explain.lossy":
    "非可逆ファイルは、その性質がそのまま表示されています。音質はエンコーダーとビットレートで決まるため、分析はしません。",
  "quality.explain.not_measured": "この形式は分析しません。",
};
