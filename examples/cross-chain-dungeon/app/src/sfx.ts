// Sound effects via Web Audio. Clips are extracted from Freedoom (BSD) into
// public/doom/sfx and played on actions + callouts. Per-clip gain levels the very
// uneven source loudness. No-ops gracefully until the context is unlocked by a
// user gesture (every action sound originates from a click, so it unlocks itself).

const VOL: Record<string, number> = {
  shotgun: 0.5,
  door: 0.4,
  getpow: 0.6,
  pain: 0.55,
  noway: 0.7,
  growl: 0.55,
  snarl: 0.55,
  teleport: 0.5,
  switch: 0.5,
  death: 0.75,
};

let ctx: AudioContext | null = null;
const buffers: Record<string, AudioBuffer> = {};

/** Create the audio context and preload every clip. Safe to call repeatedly. */
export function initSfx() {
  if (ctx) return;
  const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!Ctor) return;
  ctx = new Ctor();
  const c = ctx;
  for (const name of Object.keys(VOL)) {
    fetch(`/doom/sfx/${name}.wav`)
      .then((r) => r.arrayBuffer())
      .then((a) => c.decodeAudioData(a))
      .then((b) => {
        buffers[name] = b;
      })
      .catch(() => {});
  }
}

/** Play a preloaded clip. No-op if not loaded yet; resumes the context on demand. */
export function sfx(name: string, vol = 1) {
  if (!ctx) return;
  if (ctx.state === "suspended") void ctx.resume();
  const buf = buffers[name];
  if (!buf) return;
  const src = ctx.createBufferSource();
  src.buffer = buf;
  const gain = ctx.createGain();
  gain.gain.value = (VOL[name] ?? 0.6) * vol;
  src.connect(gain).connect(ctx.destination);
  src.start();
}
