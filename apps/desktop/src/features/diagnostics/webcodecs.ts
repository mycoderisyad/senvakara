// M0 spike: can this WebView2 decode H.264 with WebCodecs? Embedded scrcpy
// mirroring (ADR-017) depends on it. Reports facts only; nothing is assumed.

export interface CodecProbe {
  codec: string;
  label: string;
  decoder: boolean | string;
  encoder: boolean | string;
}

export interface RoundTrip {
  ok: boolean;
  detail: string;
}

export interface WebCodecsReport {
  available: boolean;
  userAgent: string;
  probes: CodecProbe[];
  h264RoundTrip: RoundTrip;
}

const CODECS: Array<[string, string]> = [
  ["avc1.42001E", "H.264 Baseline 3.0"],
  ["avc1.4D401F", "H.264 Main 3.1"],
  ["avc1.640028", "H.264 High 4.0"],
  ["hev1.1.6.L93.B0", "H.265 Main"],
  ["vp09.00.10.08", "VP9 Profile 0"],
  ["av01.0.04M.08", "AV1 Main"],
];

const W = 320;
const H = 240;

async function probe(codec: string): Promise<[boolean | string, boolean | string]> {
  const dec = await VideoDecoder.isConfigSupported({ codec, codedWidth: W, codedHeight: H })
    .then((r) => !!r.supported)
    .catch((e) => String(e));
  const enc = await VideoEncoder.isConfigSupported({ codec, width: W, height: H, bitrate: 500_000 })
    .then((r) => !!r.supported)
    .catch((e) => String(e));
  return [dec, enc];
}

/** Encode a few synthetic frames to H.264 and decode them again. */
async function h264RoundTrip(): Promise<RoundTrip> {
  const chunks: EncodedVideoChunk[] = [];
  let decoderConfig: VideoDecoderConfig | undefined;
  let encErr: unknown;
  const encoder = new VideoEncoder({
    output: (chunk, meta) => {
      chunks.push(chunk);
      if (meta?.decoderConfig) decoderConfig = meta.decoderConfig;
    },
    error: (e) => (encErr = e),
  });
  encoder.configure({ codec: "avc1.42001E", width: W, height: H, bitrate: 500_000, avc: { format: "annexb" } });

  const canvas = new OffscreenCanvas(W, H);
  const ctx = canvas.getContext("2d")!;
  for (let i = 0; i < 5; i++) {
    ctx.fillStyle = `hsl(${i * 60}, 60%, 50%)`;
    ctx.fillRect(0, 0, W, H);
    const frame = new VideoFrame(canvas, { timestamp: i * 33_333 });
    encoder.encode(frame, { keyFrame: i === 0 });
    frame.close();
  }
  await encoder.flush();
  encoder.close();
  if (encErr) return { ok: false, detail: `encoder error: ${String(encErr)}` };
  if (chunks.length === 0) return { ok: false, detail: "encoder produced no chunks" };

  let decoded = 0;
  let size = "";
  let decErr: unknown;
  const decoder = new VideoDecoder({
    output: (f) => {
      decoded++;
      size = `${f.codedWidth}x${f.codedHeight}`;
      f.close();
    },
    error: (e) => (decErr = e),
  });
  // Annex-B stream: SPS/PPS are in-band, so no `description` is needed —
  // this matches what scrcpy-server sends.
  decoder.configure({ codec: decoderConfig?.codec ?? "avc1.42001E", codedWidth: W, codedHeight: H });
  for (const c of chunks) decoder.decode(c);
  await decoder.flush();
  decoder.close();
  if (decErr) return { ok: false, detail: `decoder error: ${String(decErr)}` };
  return {
    ok: decoded === chunks.length,
    detail: `encoded ${chunks.length} chunks, decoded ${decoded} frames (${size})`,
  };
}

export async function runWebCodecsProbe(): Promise<WebCodecsReport> {
  const available = typeof VideoDecoder !== "undefined" && typeof VideoEncoder !== "undefined";
  const report: WebCodecsReport = {
    available,
    userAgent: navigator.userAgent,
    probes: [],
    h264RoundTrip: { ok: false, detail: "WebCodecs API not available" },
  };
  if (!available) return report;
  for (const [codec, label] of CODECS) {
    const [decoder, encoder] = await probe(codec);
    report.probes.push({ codec, label, decoder, encoder });
  }
  report.h264RoundTrip = await h264RoundTrip().catch((e) => ({ ok: false, detail: String(e) }));
  return report;
}
