import { useEffect, useRef, useState } from "react";
import * as chain from "./chain.ts";

// ── Doom-style raycaster ─────────────────────────────────────────────────────
// The chain has no spatial geometry (a run is one room at a time: roomKind +
// enemyHp), so the 3D space here is *cosmetic*: a single procedurally-framed room
// you can free-look around, with a Freedoom sprite billboard for whatever the room
// holds. The game stays turn-based — the action buttons still drive the contract;
// this just renders the confrontation. Assets are extracted from Freedoom (BSD)
// into /public/doom and described by manifest.json.

type SpriteMeta = { file: string; w: number; h: number; ox: number; oy: number };
type Manifest = { sprites: Record<string, SpriteMeta>; flats: Record<string, { file: string }> };
type Tex = { w: number; h: number; data: Uint32Array };

const W = 320; // internal render width (scaled up, crunchy pixels)
const H = 200;
// Move transition timings. We walk out + fade (OUT_MS), hold in the dark until the
// run advances to the next room (capped at HOLD_MAX), then fade the new room in
// (IN_MS). This keeps the next room — and any monster — hidden until you arrive.
const OUT_MS = 360;
const IN_MS = 440;
const HOLD_MAX = 6000;
const HOLD_FADE = 0.84; // darkness held between rooms (masks the room swap)
const QUAFF_MS = 760; // potion-drink animation length
const smooth = (t: number) => t * t * (3 - 2 * t);

// 8×8 cosmetic room. 1 = wall, 0 = floor. A gap in the south wall reads as the
// way you came in. The occupant (monster/prop) stands near the north wall.
const MAP = [
  1, 1, 1, 1, 1, 1, 1, 1,
  1, 0, 0, 0, 0, 0, 0, 1,
  1, 0, 0, 0, 0, 0, 0, 1,
  1, 0, 0, 0, 0, 0, 0, 1,
  1, 0, 0, 0, 0, 0, 0, 1,
  1, 0, 0, 0, 0, 0, 0, 1,
  1, 0, 0, 0, 0, 0, 0, 1,
  1, 1, 1, 0, 0, 1, 1, 1,
];
const MW = 8;
const at = (x: number, y: number) => (x < 0 || y < 0 || x >= MW || y >= MW ? 1 : MAP[y * MW + x]);

// Which Freedoom prop stands in the room, per roomKind, and its animation frames.
// [entrance, monster, treasure, trap, shrine, empty]
const OCCUPANT: Record<number, { frames: string[]; fps: number } | null> = {
  0: { frames: ["COL1A"], fps: 0 }, // entrance: a lone column
  1: { frames: ["TROOA", "TROOB", "TROOC", "TROOD"], fps: 6 }, // monster: imp (walk)
  2: { frames: ["SOULA", "SOULB", "SOULC", "SOULD"], fps: 7 }, // treasure: soulsphere
  3: { frames: ["BAR1A", "BAR1B"], fps: 3 }, // trap: barrel
  4: { frames: ["CBRAA"], fps: 0 }, // shrine: candelabra
  5: { frames: [], fps: 0 }, // empty
};
// Monster (imp) state frames — front rotation.
const IMP_PAIN = "TROOH";
const IMP_ATK = ["TROOE", "TROOF", "TROOG"];
const IMP_DEATH = ["TROOI", "TROOJ", "TROOK", "TROOL", "TROOM"];
// Shotgun: idle + fire cycle, with a muzzle flash on the middle frames.
const GUN_IDLE = "SHTGA";
const GUN_FIRE = ["SHTGB", "SHTGC", "SHTGD", "SHTGA"];
const GUN_FLASH = ["SHTFA", "SHTFB"];
// Potion: a health vial raised + tilted to drink (its frames sparkle).
const POTION = ["BON1A", "BON1B", "BON1C", "BON1D"];

// Per-room texture + tint so each room feels distinct.
const THEME: Record<number, { wall: string; floor: string; ceil: string; tint: [number, number, number] }> = {
  0: { wall: "GRNROCK", floor: "FLOOR0_2", ceil: "CEIL5_1", tint: [1, 1, 1] },
  1: { wall: "GRNROCK", floor: "FLOOR0_2", ceil: "CEIL5_1", tint: [1, 0.92, 0.86] },
  2: { wall: "FLAT5_4", floor: "FLOOR7_1", ceil: "CEIL5_1", tint: [1.1, 1.0, 0.7] },
  3: { wall: "FLAT5_1", floor: "FLOOR4_8", ceil: "CEIL3_5", tint: [1.15, 0.7, 0.6] },
  4: { wall: "FLAT1_1", floor: "FLOOR0_2", ceil: "CEIL5_1", tint: [0.75, 0.9, 1.15] },
  5: { wall: "GRNROCK", floor: "FLOOR0_2", ceil: "CEIL5_1", tint: [0.85, 0.85, 0.85] },
};

// ── asset cache (loaded once, module-level) ──────────────────────────────────
let assetsPromise: Promise<{ imgs: Record<string, HTMLImageElement>; texs: Record<string, Tex>; man: Manifest }> | null = null;

function toTex(img: HTMLImageElement): Tex {
  const c = document.createElement("canvas");
  c.width = img.width;
  c.height = img.height;
  const g = c.getContext("2d")!;
  g.drawImage(img, 0, 0);
  const d = g.getImageData(0, 0, img.width, img.height);
  return { w: img.width, h: img.height, data: new Uint32Array(d.data.buffer.slice(0)) };
}
function loadImg(src: string): Promise<HTMLImageElement> {
  return new Promise((res, rej) => {
    const im = new Image();
    im.onload = () => res(im);
    im.onerror = rej;
    im.src = src;
  });
}
function loadAssets() {
  if (!assetsPromise) {
    assetsPromise = (async () => {
      const man: Manifest = await (await fetch("/doom/manifest.json")).json();
      const imgs: Record<string, HTMLImageElement> = {};
      const texs: Record<string, Tex> = {};
      // sprites → images (drawn via drawImage)
      await Promise.all(
        Object.entries(man.sprites).map(async ([k, m]) => {
          imgs[k] = await loadImg("/doom/" + m.file);
        }),
      );
      // flats → pixel buffers (sampled per-texel in the cast)
      await Promise.all(
        Object.entries(man.flats).map(async ([k, m]) => {
          texs[k] = toTex(await loadImg("/doom/" + m.file));
        }),
      );
      return { imgs, texs, man };
    })();
  }
  return assetsPromise;
}

// Pack r,g,b (0..255) into the canvas' native little-endian RGBA word.
const rgb = (r: number, g: number, b: number) => ((255 << 24) | (b << 16) | (g << 8) | r) >>> 0;
// Shade+tint a sampled texel word `c` by light factor `f` and a colour tint.
function shade(c: number, f: number, tr: number, tg: number, tb: number) {
  const r = Math.min(255, (c & 255) * f * tr);
  const g = Math.min(255, ((c >> 8) & 255) * f * tg);
  const b = Math.min(255, ((c >> 16) & 255) * f * tb);
  return rgb(r | 0, g | 0, b | 0);
}

type Engine = {
  posX: number;
  posY: number;
  dirX: number;
  dirY: number;
  planeX: number;
  planeY: number;
  keys: Set<string>;
  yaw: number; // mouse-look accumulator (radians from start dir)
  bob: number;
  // animation clocks
  occFrame: number;
  occClock: number;
  painUntil: number;
  atkUntil: number;
  deathFrame: number;
  deathStarted: boolean;
  firing: boolean;
  fireFrame: number;
  fireClock: number;
  lastFire: number;
  // transient screen tints
  hurtUntil: number;
  healUntil: number;
  quaffUntil: number; // potion-drink animation
  // Move transition state machine. The rendered room is `shownRun` (latched), not
  // the live run — so a new room (and its monster) is only revealed once we've
  // walked out, faded, and the run has actually advanced to it.
  phase: "load" | "live" | "out" | "hold" | "in";
  phaseT0: number;
  startDepth: number;
  fleeing: boolean; // Move issued while in combat → retreat instead of advance
  shownRun: chain.RunState | null;
};

function freshEngine(): Engine {
  return {
    posX: 4.0,
    posY: 5.6,
    dirX: 0,
    dirY: -1,
    planeX: 0.66,
    planeY: 0,
    keys: new Set(),
    yaw: 0,
    bob: 0,
    occFrame: 0,
    occClock: 0,
    painUntil: 0,
    atkUntil: 0,
    deathFrame: 0,
    deathStarted: false,
    firing: false,
    fireFrame: 0,
    fireClock: 0,
    lastFire: 0,
    hurtUntil: 0,
    healUntil: 0,
    quaffUntil: 0,
    phase: "load",
    phaseT0: 0,
    startDepth: -1,
    fleeing: false,
    shownRun: null,
  };
}

export function DoomScene({ run, fx, fireNonce, walkNonce, useNonce }: { run: chain.RunState | null; fx: string | null; fireNonce: number; walkNonce: number; useNonce: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [ready, setReady] = useState(false);
  const [locked, setLocked] = useState(false);
  // latest props for the rAF loop
  const runRef = useRef(run);
  runRef.current = run;
  const fxRef = useRef(fx);
  fxRef.current = fx;
  const engRef = useRef<Engine>(freshEngine());

  // The fx string and the *Nonce counters are App-level and survive a remount, so
  // these effects must act only on a genuine *change* — never on mount. Otherwise
  // continuing a run remounts DoomScene with a stale non-zero nonce and replays
  // whatever animation you last triggered. We compare against the value captured
  // at mount (refs init to the current prop), so the mount run is always a no-op.
  const fxPrev = useRef(fx);
  const firePrev = useRef(fireNonce);
  const walkPrev = useRef(walkNonce);
  const usePrev = useRef(useNonce);

  // react to fx transitions (set transient tints / monster reactions)
  useEffect(() => {
    if (fx === fxPrev.current) return;
    fxPrev.current = fx;
    const e = engRef.current;
    const now = performance.now();
    if (fx === "hurt") {
      e.hurtUntil = now + 380;
      e.atkUntil = now + 360; // monster lunges when it hits you
    } else if (fx === "heal") {
      e.healUntil = now + 480;
    } else if (fx === "hit") {
      e.painUntil = now + 240; // monster flinches when struck
    }
  }, [fx]);

  // fire the weapon when the player attacks
  useEffect(() => {
    if (fireNonce === firePrev.current) return;
    firePrev.current = fireNonce;
    const e = engRef.current;
    e.firing = true;
    e.fireFrame = 0;
    e.fireClock = 0;
  }, [fireNonce]);

  // begin the walk-out when the player Moves; the room stays latched until we arrive
  useEffect(() => {
    if (walkNonce === walkPrev.current) return;
    walkPrev.current = walkNonce;
    const e = engRef.current;
    if (e.phase !== "live") return; // ignore double-taps mid-transition
    e.phase = "out";
    e.phaseT0 = performance.now();
    e.startDepth = runRef.current ? runRef.current.depth : -1;
    e.fleeing = !!(runRef.current && runRef.current.enemyHp > 0); // "Flee" vs "Move"
  }, [walkNonce]);

  // raise + drink a potion when the player Uses one
  useEffect(() => {
    if (useNonce === usePrev.current) return;
    usePrev.current = useNonce;
    engRef.current.quaffUntil = performance.now() + QUAFF_MS;
  }, [useNonce]);

  useEffect(() => {
    loadAssets().then(() => setReady(true));
  }, []);

  useEffect(() => {
    if (!ready) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    let raf = 0;
    let stop = false;
    let assets: Awaited<ReturnType<typeof loadAssets>>;

    // offscreen render target at internal resolution
    const off = document.createElement("canvas");
    off.width = W;
    off.height = H;
    const octx = off.getContext("2d")!;
    octx.imageSmoothingEnabled = false;
    const frame = octx.createImageData(W, H);
    const buf = new Uint32Array(frame.data.buffer);
    const zbuf = new Float32Array(W);

    const dctx = canvas.getContext("2d")!;
    dctx.imageSmoothingEnabled = false;

    // input
    const e = engRef.current;
    const onKey = (down: boolean) => (ev: KeyboardEvent) => {
      const k = ev.key.toLowerCase();
      if (["w", "a", "s", "d", "arrowup", "arrowdown", "arrowleft", "arrowright"].includes(k)) {
        if (down) e.keys.add(k);
        else e.keys.delete(k);
        ev.preventDefault();
      }
    };
    const kd = onKey(true);
    const ku = onKey(false);
    const onMove = (ev: MouseEvent) => {
      if (document.pointerLockElement === canvas) e.yaw += ev.movementX * 0.0026;
    };
    const onLock = () => setLocked(document.pointerLockElement === canvas);
    const onClick = () => {
      if (document.pointerLockElement !== canvas) canvas.requestPointerLock();
    };
    window.addEventListener("keydown", kd);
    window.addEventListener("keyup", ku);
    window.addEventListener("mousemove", onMove);
    document.addEventListener("pointerlockchange", onLock);
    canvas.addEventListener("click", onClick);

    loadAssets().then((a) => {
      assets = a;
      let last = performance.now();

      const loop = () => {
        if (stop) return;
        const now = performance.now();
        const dt = Math.min(0.05, (now - last) / 1000);
        last = now;
        // ── room transition state machine (latches the rendered room) ──
        const live = runRef.current;
        if (e.phase === "load") {
          // just mounted: stay dark until this run's data has actually loaded,
          // then fade it in (so continuing a run doesn't flash a default room)
          if (live) {
            e.shownRun = live;
            e.phase = "in";
            e.phaseT0 = now;
          }
        } else if (e.phase === "live") {
          e.shownRun = live;
        } else if (e.phase === "out") {
          if (now - e.phaseT0 >= OUT_MS) {
            e.phase = "hold";
            e.phaseT0 = now;
          }
        } else if (e.phase === "hold") {
          const advanced = !!live && e.startDepth >= 0 && live.depth > e.startDepth;
          if (advanced || now - e.phaseT0 >= HOLD_MAX) {
            e.shownRun = live; // commit: the new room is revealed from here
            e.phase = "in";
            e.phaseT0 = now;
          }
        } else {
          // "in": keep synced (e.g. combat in the freshly entered room)
          e.shownRun = live;
          if (now - e.phaseT0 >= IN_MS) e.phase = "live";
        }
        const transitioning = e.phase !== "live";
        const r = e.shownRun;
        const kind = r ? r.roomKind : 0;
        const theme = THEME[kind] ?? THEME[0];
        const wallT = assets.texs[theme.wall] ?? Object.values(assets.texs)[0];
        const floorT = assets.texs[theme.floor] ?? wallT;
        const ceilT = assets.texs[theme.ceil] ?? wallT;
        const [tr, tg, tb] = theme.tint;

        // ── movement (cosmetic, collision-clamped to the room) ──
        const base = { x: 0, y: -1 }; // start dir = north
        const ca = Math.cos(e.yaw),
          sa = Math.sin(e.yaw);
        e.dirX = base.x * ca - base.y * sa;
        e.dirY = base.x * sa + base.y * ca;
        e.planeX = -e.dirY * 0.66; // camera plane ⟂ dir, FOV ≈ 66°
        e.planeY = e.dirX * 0.66;

        if (e.phase === "load") {
          // waiting in the dark for the run to load — sit at the entrance
          e.posX = 4.0;
          e.yaw = 0;
          e.posY = 5.9;
        } else if (e.phase === "out") {
          // step out while fading: forward into the room on a normal Move, but
          // backward toward the entrance when fleeing, so we retreat from the monster
          const t = smooth(Math.min(1, (now - e.phaseT0) / OUT_MS));
          e.posX = 4.0;
          e.yaw = 0;
          e.posY = e.fleeing ? 5.6 + 1.7 * t : 5.6 - 2.4 * t;
          e.bob += dt * 17;
        } else if (e.phase === "hold") {
          // drifting through the dark between rooms
          e.posX = 4.0;
          e.yaw = 0;
          e.posY = 3.0;
          e.bob += dt * 11;
        } else if (e.phase === "in") {
          // arrive at the new room's entrance and settle forward
          const t = smooth(Math.min(1, (now - e.phaseT0) / IN_MS));
          e.posX = 4.0;
          e.yaw = 0;
          e.posY = 5.9 - 0.3 * t;
          e.bob += dt * 9;
        } else {
          let mv = 0;
          let strafe = 0;
          if (e.keys.has("w") || e.keys.has("arrowup")) mv += 1;
          if (e.keys.has("s") || e.keys.has("arrowdown")) mv -= 1;
          if (e.keys.has("d") || e.keys.has("arrowright")) strafe += 1;
          if (e.keys.has("a") || e.keys.has("arrowleft")) strafe -= 1;
          const spd = 1.8 * dt;
          const nx = e.posX + (e.dirX * mv + -e.dirY * strafe) * spd;
          const ny = e.posY + (e.dirY * mv + e.dirX * strafe) * spd;
          // keep a margin from walls; the occupant cell also blocks
          if (at(Math.floor(nx), Math.floor(e.posY)) === 0) e.posX = clampRoom(nx);
          if (at(Math.floor(e.posX), Math.floor(ny)) === 0) e.posY = clampRoom(ny);
          // view bob while moving
          if (mv || strafe) e.bob += dt * 9;
        }

        // ── floor + ceiling cast ──
        const dirX = e.dirX,
          dirY = e.dirY,
          planeX = e.planeX,
          planeY = e.planeY;
        const horizon = H >> 1;
        for (let y = horizon + 1; y < H; y++) {
          const p = y - horizon;
          const rowDist = (0.5 * H) / p;
          const stepX = (rowDist * 2 * planeX) / W;
          const stepY = (rowDist * 2 * planeY) / W;
          let fx0 = e.posX + rowDist * (dirX - planeX);
          let fy0 = e.posY + rowDist * (dirY - planeY);
          const f = Math.max(0.18, Math.min(1, 1.5 / (1 + rowDist * rowDist * 0.05)));
          const rowF = y * W;
          const rowC = (H - 1 - y) * W;
          for (let x = 0; x < W; x++) {
            const tx = ((fx0 - Math.floor(fx0)) * floorT.w) & (floorT.w - 1);
            const ty = ((fy0 - Math.floor(fy0)) * floorT.h) & (floorT.h - 1);
            buf[rowF + x] = shade(floorT.data[ty * floorT.w + tx], f, tr, tg, tb);
            const cx = ((fx0 - Math.floor(fx0)) * ceilT.w) & (ceilT.w - 1);
            const cy = ((fy0 - Math.floor(fy0)) * ceilT.h) & (ceilT.h - 1);
            buf[rowC + x] = shade(ceilT.data[cy * ceilT.w + cx], f * 0.7, tr, tg, tb);
            fx0 += stepX;
            fy0 += stepY;
          }
        }

        // ── wall cast (DDA) ──
        for (let x = 0; x < W; x++) {
          const camX = (2 * x) / W - 1;
          const rdx = dirX + planeX * camX;
          const rdy = dirY + planeY * camX;
          let mapX = Math.floor(e.posX);
          let mapY = Math.floor(e.posY);
          const ddx = Math.abs(1 / rdx);
          const ddy = Math.abs(1 / rdy);
          let stepX: number, stepY: number, sdx: number, sdy: number;
          if (rdx < 0) {
            stepX = -1;
            sdx = (e.posX - mapX) * ddx;
          } else {
            stepX = 1;
            sdx = (mapX + 1 - e.posX) * ddx;
          }
          if (rdy < 0) {
            stepY = -1;
            sdy = (e.posY - mapY) * ddy;
          } else {
            stepY = 1;
            sdy = (mapY + 1 - e.posY) * ddy;
          }
          let side = 0;
          let hit = 0;
          for (let g = 0; g < 32 && !hit; g++) {
            if (sdx < sdy) {
              sdx += ddx;
              mapX += stepX;
              side = 0;
            } else {
              sdy += ddy;
              mapY += stepY;
              side = 1;
            }
            if (at(mapX, mapY) > 0) hit = 1;
          }
          const perp = side === 0 ? sdx - ddx : sdy - ddy;
          zbuf[x] = perp;
          const lineH = Math.floor(H / perp);
          let dStart = -((lineH / 2) | 0) + horizon;
          let dEnd = ((lineH / 2) | 0) + horizon;
          if (dStart < 0) dStart = 0;
          if (dEnd >= H) dEnd = H - 1;
          let wallX = side === 0 ? e.posY + perp * rdy : e.posX + perp * rdx;
          wallX -= Math.floor(wallX);
          let texX = (wallX * wallT.w) | 0;
          if ((side === 0 && rdx > 0) || (side === 1 && rdy < 0)) texX = wallT.w - texX - 1;
          const f = Math.max(0.2, Math.min(1, 1.5 / (1 + perp * perp * 0.05))) * (side === 1 ? 0.72 : 1);
          const texStep = wallT.h / lineH;
          let texPos = (dStart - horizon + lineH / 2) * texStep;
          for (let y = dStart; y <= dEnd; y++) {
            const ty = ((texPos | 0) % wallT.h + wallT.h) % wallT.h;
            buf[y * W + x] = shade(wallT.data[ty * wallT.w + texX], f, tr, tg, tb);
            texPos += texStep;
          }
        }

        octx.putImageData(frame, 0, 0);

        // ── occupant billboard (hidden in the dark "load"/"hold" gaps) ──
        const occ = OCCUPANT[kind];
        if (e.phase !== "hold" && e.phase !== "load" && occ && occ.frames.length) {
          const spriteKey = pickOccupantFrame(e, r, occ, now, dt);
          if (spriteKey) drawSprite(octx, assets.imgs[spriteKey], assets.man.sprites[spriteKey], e, zbuf);
        }

        // ── first-person hands: potion while quaffing, else the weapon ──
        if (now < e.quaffUntil) drawPotion(octx, assets, e, now);
        else drawWeapon(octx, assets, e, dt);

        // ── screen tints (damage / heal) ──
        if (now < e.hurtUntil) {
          octx.fillStyle = `rgba(190,20,12,${0.5 * ((e.hurtUntil - now) / 380)})`;
          octx.fillRect(0, 0, W, H);
        } else if (now < e.healUntil) {
          octx.fillStyle = `rgba(60,200,110,${0.4 * ((e.healUntil - now) / 480)})`;
          octx.fillRect(0, 0, W, H);
        }

        // transition fade: out 0→full, hold full, in full→0. The room swap happens
        // under the darkness, so the next room emerges only as we fade back in.
        if (transitioning) {
          let fade = HOLD_FADE;
          if (e.phase === "out") fade = HOLD_FADE * smooth(Math.min(1, (now - e.phaseT0) / OUT_MS));
          else if (e.phase === "in") fade = HOLD_FADE * (1 - smooth(Math.min(1, (now - e.phaseT0) / IN_MS)));
          octx.fillStyle = `rgba(0,0,0,${fade})`;
          octx.fillRect(0, 0, W, H);
        }

        // blit scaled to the visible canvas
        dctx.drawImage(off, 0, 0, W, H, 0, 0, canvas.width, canvas.height);
        raf = requestAnimationFrame(loop);
      };
      raf = requestAnimationFrame(loop);
    });

    return () => {
      stop = true;
      cancelAnimationFrame(raf);
      window.removeEventListener("keydown", kd);
      window.removeEventListener("keyup", ku);
      window.removeEventListener("mousemove", onMove);
      document.removeEventListener("pointerlockchange", onLock);
      canvas.removeEventListener("click", onClick);
      if (document.pointerLockElement === canvas) document.exitPointerLock();
    };
  }, [ready]);

  // size the canvas to its container (keeps 8:5)
  const wrapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const wrap = wrapRef.current;
    const canvas = canvasRef.current;
    if (!wrap || !canvas) return;
    const ro = new ResizeObserver(() => {
      const w = wrap.clientWidth;
      canvas.width = Math.max(160, Math.round(w));
      canvas.height = Math.round((canvas.width * H) / W);
      const c = canvas.getContext("2d");
      if (c) c.imageSmoothingEnabled = false;
    });
    ro.observe(wrap);
    return () => ro.disconnect();
  }, [ready]);

  return (
    <div className="doom" ref={wrapRef}>
      <canvas ref={canvasRef} className="doom-canvas" />
      {!ready && <div className="doom-load">loading textures…</div>}
      {ready && !locked && <div className="doom-hint">click to look · WASD to move</div>}
    </div>
  );
}

function clampRoom(v: number) {
  return Math.max(1.18, Math.min(MW - 1.18, v));
}

// Choose the monster's current frame from run/combat state; advances its clock.
function pickOccupantFrame(e: Engine, r: chain.RunState | null, occ: { frames: string[]; fps: number }, now: number, dt: number): string | null {
  if (r && r.roomKind === 1) {
    // imp: death sequence wins, then pain, then lunge, else walk
    if (r.enemyHp <= 0) {
      if (!e.deathStarted) {
        e.deathStarted = true;
        e.deathFrame = 0;
        e.occClock = 0;
      }
      e.occClock += dt;
      if (e.occClock > 0.12 && e.deathFrame < IMP_DEATH.length - 1) {
        e.occClock = 0;
        e.deathFrame++;
      }
      return IMP_DEATH[e.deathFrame];
    }
    e.deathStarted = false;
    if (now < e.painUntil) return IMP_PAIN;
    if (now < e.atkUntil) {
      const i = Math.floor(((e.atkUntil - now) / 360) * IMP_ATK.length);
      return IMP_ATK[Math.max(0, Math.min(IMP_ATK.length - 1, i))];
    }
  }
  // generic looped animation (props + imp walk)
  if (occ.fps > 0) {
    e.occClock += dt;
    if (e.occClock > 1 / occ.fps) {
      e.occClock = 0;
      e.occFrame = (e.occFrame + 1) % occ.frames.length;
    }
  }
  return occ.frames[e.occFrame % occ.frames.length];
}

// Billboard: project the occupant (room centre) and draw it as z-tested 1px slices.
function drawSprite(ctx: CanvasRenderingContext2D, img: HTMLImageElement | undefined, meta: SpriteMeta | undefined, e: Engine, zbuf: Float32Array) {
  if (!img || !meta) return;
  const ox = 4.0 - e.posX; // occupant stands at room cell (4.0, 2.6)
  const oy = 2.6 - e.posY;
  const inv = 1 / (e.planeX * e.dirY - e.dirX * e.planeY);
  const tx = inv * (e.dirY * ox - e.dirX * oy);
  const ty = inv * (-e.planeY * ox + e.planeX * oy); // depth
  if (ty <= 0.2) return;
  const screenX = (W / 2) * (1 + tx / ty);
  // scale so a ~64px-tall sprite ≈ one wall height at this distance
  const drawH = Math.abs(H / ty) * (meta.h / 64);
  const drawW = drawH * (meta.w / meta.h);
  const groundY = H / 2 + H / (2 * ty); // floor line at this depth
  const startY = groundY - drawH;
  const startX = Math.floor(screenX - drawW / 2);
  // Draw as z-tested 1px-wide slices so wall edges occlude the billboard.
  for (let sx = 0; sx < drawW; sx++) {
    const x = startX + sx;
    if (x < 0 || x >= W) continue;
    if (ty >= zbuf[x]) continue; // behind a wall
    const texX = Math.floor((sx / drawW) * meta.w);
    ctx.drawImage(img, texX, 0, 1, meta.h, x, startY, 1, drawH);
  }
}

function drawWeapon(ctx: CanvasRenderingContext2D, assets: Awaited<ReturnType<typeof loadAssets>>, e: Engine, dt: number) {
  let key = GUN_IDLE;
  let flash: string | null = null;
  if (e.firing) {
    e.fireClock += dt;
    const step = 0.07;
    e.fireFrame = Math.floor(e.fireClock / step);
    if (e.fireFrame >= GUN_FIRE.length) {
      e.firing = false;
      e.fireFrame = 0;
      e.fireClock = 0;
    } else {
      key = GUN_FIRE[e.fireFrame];
      if (e.fireFrame === 0) flash = GUN_FLASH[0];
      else if (e.fireFrame === 1) flash = GUN_FLASH[1];
    }
  }
  const img = assets.imgs[key];
  const meta = assets.man.sprites[key];
  if (!img || !meta) return;
  const scale = (H * 0.58) / 128; // weapons are ~128 tall; size to ~58% screen
  const bobX = Math.sin(e.bob) * 6;
  const bobY = Math.abs(Math.cos(e.bob)) * 5;
  const recoil = e.firing ? 10 : 0;
  const dw = meta.w * scale;
  const dh = meta.h * scale;
  const dx = (W - dw) / 2 + bobX;
  const dy = H - dh + bobY + recoil;
  if (flash) {
    const fi = assets.imgs[flash];
    const fm = assets.man.sprites[flash];
    if (fi && fm) {
      const fw = fm.w * scale;
      const fh = fm.h * scale;
      ctx.drawImage(fi, W / 2 - fw / 2 + bobX, dy - fh * 0.7, fw, fh);
    }
  }
  ctx.drawImage(img, dx, dy, dw, dh);
}

// Potion quaff: the vial rises from the bottom in an arc, tilts back to drink at
// the peak, with a green heal glow that's strongest mid-drink.
function drawPotion(ctx: CanvasRenderingContext2D, assets: Awaited<ReturnType<typeof loadAssets>>, e: Engine, now: number) {
  const t = 1 - (e.quaffUntil - now) / QUAFF_MS; // 0..1
  const rise = Math.sin(Math.min(1, Math.max(0, t)) * Math.PI); // 0 → 1 → 0
  const key = POTION[Math.floor(t * 8) % POTION.length];
  const img = assets.imgs[key];
  const meta = assets.man.sprites[key];
  // green heal glow under the vial, peaking mid-drink
  ctx.fillStyle = `rgba(70,225,120,${0.34 * rise})`;
  ctx.fillRect(0, 0, W, H);
  if (!img || !meta) return;
  const scale = (H * 0.42) / meta.h; // vial is tiny (~15px); blow it up
  const dw = meta.w * scale;
  const dh = meta.h * scale;
  ctx.save();
  ctx.translate(W / 2, H - dh * 0.4 - rise * (H * 0.34));
  ctx.rotate(-0.5 * rise); // tilt back to drink
  ctx.drawImage(img, -dw / 2, -dh / 2, dw, dh);
  ctx.restore();
}
