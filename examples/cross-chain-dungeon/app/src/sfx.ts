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

// Mute preference, persisted across reloads. When muted, sfx() is a no-op.
let muted = false;
try {
  muted = localStorage.getItem("sfx-muted") === "1";
} catch {
  /* localStorage unavailable */
}
export function isSfxMuted(): boolean {
  return muted;
}
export function setSfxMuted(v: boolean): void {
  muted = v;
  try {
    localStorage.setItem("sfx-muted", v ? "1" : "0");
  } catch {
    /* ignore */
  }
}

// Master volume (0..1), persisted. Multiplies the per-clip levels.
let volume = 1;
try {
  const v = parseFloat(localStorage.getItem("sfx-volume") ?? "");
  if (!Number.isNaN(v)) volume = Math.min(1, Math.max(0, v));
} catch {
  /* localStorage unavailable */
}
export function getSfxVolume(): number {
  return volume;
}
export function setSfxVolume(v: number): void {
  volume = Math.min(1, Math.max(0, v));
  try {
    localStorage.setItem("sfx-volume", String(volume));
  } catch {
    /* ignore */
  }
}

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
  if (!ctx || muted) return;
  if (ctx.state === "suspended") void ctx.resume();
  const buf = buffers[name];
  if (!buf) return;
  const src = ctx.createBufferSource();
  src.buffer = buf;
  const gain = ctx.createGain();
  gain.gain.value = (VOL[name] ?? 0.6) * vol * volume;
  src.connect(gain).connect(ctx.destination);
  src.start();
}
