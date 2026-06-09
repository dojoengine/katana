import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import * as chain from "./chain.ts";
import { useWallet } from "./wallet.tsx";
import { Tutorial } from "./tutorial.tsx";
import { DoomScene } from "./doom.tsx";
import { sfx, initSfx, isSfxMuted, setSfxMuted, getSfxVolume, setSfxVolume } from "./sfx.ts";
import { lookupAddresses } from "@cartridge/controller";

// Canonical key for an address — strips zero-padding so Torii's format and
// Cartridge's lookup results compare equal regardless of leading zeros.
const addrKey = (a: string): string => {
  try {
    return "0x" + BigInt(a).toString(16);
  } catch {
    return a.toLowerCase();
  }
};


// The game world is deployed (deployments.json carries a real GAME_SYSTEM). The global
// run-outcome feed / leaderboard / stats are appchain-world state, not per-user, so they
// load whenever this is true — even with no wallet connected.
const DEPLOYED = BigInt(chain.GAME_SYSTEM) !== 0n;

// Toast kinds rendered as a big centered banner (dramatic events) rather than the
// small bottom-left gains/losses feed.
const EVENT_KINDS = ["ambush", "flee", "mimic", "escaped"];

// The right-column "Runs" run-outcome panel is hidden for now. Flip this to true to
// bring it back (the code is preserved below, just gated off). Kept as a flag rather
// than a comment so the panel's refs/handlers (logRef, catchUpLog, …) stay referenced.
const SHOW_RUNS_PANEL = false;

// Sound effect played for each toast kind (Freedoom clips, see sfx.ts). The minor
// pickups (gold / HP / potion) are intentionally silent — only damage and the
// dramatic callouts get a sound.
const KIND_SFX: Record<string, string> = {
  dmg: "pain",
  flee: "noway",
  ambush: "growl",
  mimic: "snarl",
  escaped: "teleport",
};

// Doom-style status-bar face, picked from the hp ratio + combat/death, with a
// transient "ouch" on the frame you take damage. The 3D scene itself lives in
// doom.tsx (a Freedoom-textured raycaster); this only drives the HUD face.
const FACE_OK = [",-----.", "|o   o|", "| \\_/ |", "`-----'"];
const FACE_HURT = [",-----.", "|-   o|", "| ___ |", "`-----'"];
const FACE_CRIT = [",-----.", "|@   @|", "|/~~~\\|", "`-----'"];
const FACE_SNARL = [",-----.", "|>   <|", "|WWWWW|", "`-----'"];
const FACE_OUCH = [",-----.", "|x   x|", "| >o< |", "`-----'"];
const FACE_DEAD = [",-----.", "|X   X|", "| --- |", "`-----'"];

function doomFace(run: chain.RunState, fx: string | null): string[] {
  if (!run.alive) return FACE_DEAD;
  if (fx === "hurt") return FACE_OUCH;
  const r = run.hp / Math.max(run.maxHp, 1);
  if (run.enemyHp > 0) return r < 0.3 ? FACE_CRIT : FACE_SNARL;
  if (r > 0.66) return FACE_OK;
  if (r > 0.33) return FACE_HURT;
  return FACE_CRIT;
}

// Run-finish time for the outcome log — local HH:MM:SS (24h).
const fmtTime = (ms: number): string =>
  Number.isFinite(ms)
    ? new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })
    : "--:--:--";

// Derive a transient visual effect ("hurt" | "heal" | "hit" | "step") by diffing
// the polled run state across renders. Drives the scene shake/flash, the weapon
// recoil, the monster lunge frame, and the status-bar "ouch" face. It reads net
// state changes from polling, so it needs no wiring into the action handlers.
function useRunFx(run: chain.RunState | null): string | null {
  const [fx, setFx] = useState<string | null>(null);
  const prev = useRef<{ runNo: number; hp: number; enemyHp: number; depth: number } | null>(null);
  useEffect(() => {
    if (!run) {
      prev.current = null;
      return;
    }
    const p = prev.current;
    prev.current = { runNo: run.runNo, hp: run.hp, enemyHp: run.enemyHp, depth: run.depth };
    if (!p || p.runNo !== run.runNo) return; // first sight / run switch: no effect
    let next: string | null = null;
    if (run.hp < p.hp) next = "hurt";
    else if (run.hp > p.hp) next = "heal";
    else if (run.enemyHp < p.enemyHp && p.enemyHp > 0) next = "hit";
    else if (run.depth > p.depth) next = "step";
    if (next) setFx(next);
  }, [run?.runNo, run?.hp, run?.enemyHp, run?.depth]);
  useEffect(() => {
    if (!fx) return;
    const id = setTimeout(() => setFx(null), 520);
    return () => clearTimeout(id);
  }, [fx]);
  return fx;
}

function Gauge({ settled, tip }: { settled: number; tip: number }) {
  // The bar flex-grows to fill the row; size its block count to the measured width
  // (one glyph's advance via a hidden probe, like the dungeon map) so it stays full.
  const barRef = useRef<HTMLSpanElement>(null);
  const [n, setN] = useState(24);
  useEffect(() => {
    const el = barRef.current;
    if (!el) return;
    const measure = () => {
      const cs = getComputedStyle(el);
      const probe = document.createElement("span");
      probe.style.cssText = "position:absolute;visibility:hidden;white-space:pre";
      probe.style.fontFamily = cs.fontFamily;
      probe.style.fontSize = cs.fontSize;
      probe.style.fontWeight = cs.fontWeight;
      probe.style.letterSpacing = cs.letterSpacing;
      probe.textContent = "█".repeat(100);
      el.appendChild(probe);
      const charW = probe.getBoundingClientRect().width / 100;
      el.removeChild(probe);
      if (!charW) return;
      const cols = Math.max(8, Math.floor(el.clientWidth / charW) - 2); // -2 for the ▕▏ brackets
      setN((prev) => (prev === cols ? prev : cols));
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);
  const safeTip = Math.max(tip, 1);
  const fill = Math.max(0, Math.min(n, Math.round((settled / safeTip) * n)));
  const settling = settled < tip; // sweep the settled blocks while L1 is still catching up
  return (
    <div className="gauge">
      <span className="bar" ref={barRef}>
        ▕<span className={`fill ${settling ? "settling" : ""}`}>{"█".repeat(fill)}</span>
        <span className="empty">{"░".repeat(Math.max(0, n - fill))}</span>▏
      </span>
      <span
        className="gauge-count"
        data-tooltip="settled / tip — appchain blocks saya has settled onto L1 (piltover), over the appchain's current block. The gap is how far L1 settlement trails; your bank mints once its withdrawal's block is settled."
      >
        <span className="n">{String(Math.max(settled, 0)).padStart(4, "0")}</span> / <span className="n">{String(tip).padStart(4, "0")}</span>
      </span>
    </div>
  );
}

/** Settings modal — currently just a sound-fx toggle (persisted in localStorage). */
function SettingsModal({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const [muted, setMuted] = useState(isSfxMuted());
  const [vol, setVol] = useState(getSfxVolume());
  // Throttle the while-sliding preview blip so dragging tracks the level audibly
  // without machine-gunning a blip per tick.
  const lastBlip = useRef(0);
  const toggleSound = () => {
    const next = !muted;
    setSfxMuted(next);
    setMuted(next);
    if (!next) sfx("switch"); // little confirmation blip when turning sound back on
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-sm" onClick={(e) => e.stopPropagation()}>
        <div className="modal-h">
          <span>⚙ settings</span>
          <button className="modal-x" onClick={onClose} aria-label="close">
            ✕
          </button>
        </div>
        <div className="settings">
          <div className="settings-row">
            <div>
              <div className="settings-label">Sound effects</div>
              <div className="settings-sub">action + callout audio</div>
            </div>
            <button
              className={`toggle ${muted ? "" : "on"}`}
              onClick={toggleSound}
              role="switch"
              aria-checked={!muted}
            >
              {muted ? "Off" : "On"}
            </button>
          </div>
          <div className="settings-row">
            <div>
              <div className="settings-label">Volume</div>
              <div className="settings-sub">sound fx level</div>
            </div>
            <div className="vol-control">
              <input
                type="range"
                className="vol-slider"
                style={{ "--pct": `${Math.round(vol * 100)}%` } as CSSProperties}
                min={0}
                max={1}
                step={0.05}
                value={vol}
                disabled={muted}
                onChange={(e) => {
                  const v = parseFloat(e.target.value);
                  setSfxVolume(v);
                  setVol(v);
                  // preview while sliding, throttled to one blip per ~120ms
                  const now = performance.now();
                  if (!muted && now - lastBlip.current > 120) {
                    lastBlip.current = now;
                    sfx("switch");
                  }
                }}
                onPointerUp={() => {
                  if (!muted) sfx("switch"); // settle blip at the final level
                }}
                aria-label="sound fx volume"
              />
              <span className="vol-pct">{Math.round(vol * 100)}%</span>
            </div>
          </div>
          <div className="settings-row settings-col">
            <div className="settings-label">Networks</div>
            <div className="settings-chips">
              <span className="chip on">
                <span className="led" />
                {chain.SETTLEMENT_NETWORK.toUpperCase()} · settlement
              </span>
              <span className="chip on">
                <span className="led" />
                DUNGEON · appchain
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

/** Detail modal for a clicked run-outcome entry, with a link to its appchain tx. */
function OutcomeModal({ outcome, onClose }: { outcome: chain.OutcomeRow; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const txUrl = chain.explorerTxUrl(chain.APPCHAIN_EXPLORER, outcome.txHash);
  const rows: [string, ReactNode][] = [
    ["run", `#${outcome.runNo}`],
    ["player", outcome.player],
    ["outcome", outcome.died ? "died" : "extracted"],
    ["depth", String(outcome.depth)],
    [outcome.died ? "gold forfeited" : "gold banked", outcome.loot.toLocaleString()],
    ["appchain block", String(outcome.block)],
    [
      "tx hash",
      // The hash itself is the link to the run-ending tx on the appchain explorer.
      <a className="tx-link" href={txUrl} target="_blank" rel="noreferrer" title="view on appchain explorer">
        {outcome.txHash} ↗
      </a>,
    ],
  ];

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-h">
          <span>
            {outcome.died ? "☠" : "✓"} run #{outcome.runNo} {outcome.died ? "died" : "extracted"}
          </span>
          <button className="modal-x" onClick={onClose} aria-label="close">
            ✕
          </button>
        </div>
        <dl className="kv">
          {rows.map(([k, v]) => (
            <div className="kv-row" key={k}>
              <dt>{k}</dt>
              <dd className={k === "tx hash" || k === "player" ? "mono-wrap" : ""}>{v}</dd>
            </div>
          ))}
        </dl>
      </div>
    </div>
  );
}

/** Account modal: shows the currently connected account, and a picker to switch the
 *  signing method — operator account ↔ Cartridge Controller. Opened from the account
 *  chip in the header. */
function WalletModal({ onClose }: { onClose: () => void }) {
  const wallet = useWallet();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const [showMore, setShowMore] = useState(false);
  const connected = wallet.method !== null; // a signer is active
  const isCtrl = wallet.method === "controller";
  const signerName =
    wallet.method === "controller"
      ? "Cartridge Controller"
      : wallet.method === "injected"
        ? wallet.label || "Wallet"
        : "Operator account";
  const pick = (fn: () => Promise<void>) => () => void fn().then(onClose).catch(() => {});

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-h">
          <span>{connected ? "account" : "connect a wallet"}</span>
          <button className="modal-x" onClick={onClose} aria-label="close">
            ✕
          </button>
        </div>
        {connected ? (
          // Connected: just the current account + disconnect — no other-wallet options.
          <>
            <dl className="kv">
              <div className="kv-row">
                <dt>signing as</dt>
                <dd>{signerName}</dd>
              </div>
              {isCtrl && wallet.username ? (
                <div className="kv-row">
                  <dt>username</dt>
                  <dd>{wallet.username}</dd>
                </div>
              ) : null}
              <div className="kv-row">
                <dt>address</dt>
                <dd className="mono-wrap">{wallet.player || "—"}</dd>
              </div>
            </dl>
            <button className="wallet-disconnect" onClick={pick(wallet.disconnect)}>
              disconnect
            </button>
          </>
        ) : (
          // Disconnected: Cartridge Controller is the primary choice; the rest live under "more".
          <div className="wallet-methods">
            <button
              className="wallet-opt primary"
              disabled={!wallet.controllerAvailable || wallet.connecting}
              onClick={pick(wallet.connectController)}
            >
              <span className="wo-title">{wallet.connecting ? "Connecting…" : "Cartridge Controller"}</span>
              <span className="wo-sub">
                {wallet.controllerAvailable ? "passkey wallet · signs both chains" : "unavailable — start the stack first"}
              </span>
            </button>
            <button className="wallet-more" onClick={() => setShowMore((v) => !v)} aria-expanded={showMore}>
              {showMore ? "less" : "more"}
            </button>
            <div className={`wallet-extra ${showMore ? "open" : ""}`}>
              <div className="wallet-extra-inner">
                <button className="wallet-opt" tabIndex={showMore ? 0 : -1} onClick={pick(wallet.useOperator)}>
                  <span className="wo-title">Operator account</span>
                  <span className="wo-sub">
                    prefunded Sepolia dev key · <span className="mono">{chain.shortHex(chain.operatorAccount.address)}</span>
                  </span>
                </button>
                <button
                  className="wallet-opt"
                  tabIndex={showMore ? 0 : -1}
                  disabled={wallet.connecting}
                  onClick={pick(() => wallet.connectInjected("argent"))}
                >
                  <span className="wo-title">Argent X</span>
                  <span className="wo-sub">browser wallet · Sepolia (dev key plays)</span>
                </button>
                <button
                  className="wallet-opt"
                  tabIndex={showMore ? 0 : -1}
                  disabled={wallet.connecting}
                  onClick={pick(() => wallet.connectInjected("braavos"))}
                >
                  <span className="wo-title">Braavos</span>
                  <span className="wo-sub">browser wallet · Sepolia (dev key plays)</span>
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/** Details for one pending withdrawal: the L2 tx that emitted it, its appchain block,
 *  the saya settlement progress, and the L2→L1 message route + the poseidon message
 *  hash piltover registers / `bank` consumes. */
function WithdrawalModal({
  w,
  player,
  settled,
  tip,
  onClose,
}: {
  w: chain.WithdrawalRow;
  player: string;
  settled: number;
  tip: number;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const ready = settled >= w.block;
  const msgHash = chain.withdrawalMessageHash(player, w.amount, w.withdrawNo);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-h">
          <span>withdrawal #{w.withdrawNo}</span>
          <button className="modal-x" onClick={onClose} aria-label="close">
            ✕
          </button>
        </div>
        <dl className="kv">
          <div className="kv-row">
            <dt>status</dt>
            <dd style={{ color: ready ? "var(--green)" : "var(--amber)" }}>
              {ready ? "settled · claimable" : "awaiting saya settlement"}
            </dd>
          </div>
          <div className="kv-row">
            <dt>amount</dt>
            <dd>{w.amount.toLocaleString()} $GOLD</dd>
          </div>
          <div className="kv-row">
            <dt>appchain block</dt>
            <dd>
              {w.block} <small style={{ color: "var(--faint)" }}>· settled {settled} / tip {tip}</small>
            </dd>
          </div>
          <div className="kv-row">
            <dt>L2 tx</dt>
            <dd className="mono-wrap">
              <a
                className="tx-link"
                href={chain.explorerTxUrl(chain.APPCHAIN_EXPLORER, w.txHash)}
                target="_blank"
                rel="noreferrer"
              >
                {w.txHash} ↗
              </a>
            </dd>
          </div>
          <div className="kv-row">
            <dt>L1 msg hash</dt>
            <dd className="mono-wrap">{msgHash}</dd>
          </div>
          <div className="kv-row">
            <dt>from · L2</dt>
            <dd className="mono-wrap">{chain.GAME_SYSTEM}</dd>
          </div>
          <div className="kv-row">
            <dt>to · L1</dt>
            <dd className="mono-wrap">{chain.BANK_SYSTEM}</dd>
          </div>
        </dl>
        <div className="legend">
          the L2→L1 message is hashed (poseidon) to the L1 msg hash that <b>bank</b> consumes on{" "}
          {chain.SETTLEMENT_NAME}.
        </div>
      </div>
    </div>
  );
}

/** Live log viewer: tails a chosen service's .run/*.log over SSE (Vite plugin). */
const LOG_SERVICES = [
  { id: "appchain", label: "Appchain · Katana" },
  { id: "saya", label: "Saya · settle" },
  { id: "torii-game", label: "Torii · game (L2)" },
  { id: "torii-bank", label: "Torii · bank (L1)" },
] as const;

function LogViewer() {
  const [svc, setSvc] = useState<(typeof LOG_SERVICES)[number]["id"]>("saya");
  const [lines, setLines] = useState<string[]>([]);
  const [live, setLive] = useState(false);
  const preRef = useRef<HTMLPreElement>(null);
  const atBottomRef = useRef(true);

  useEffect(() => {
    setLines([]);
    setLive(false);
    atBottomRef.current = true;
    const es = new EventSource(`/api/logs/${svc}/stream`);
    es.onopen = () => setLive(true);
    es.onmessage = (e) => setLines((prev) => [...prev, e.data].slice(-2000));
    es.onerror = () => setLive(false); // EventSource auto-reconnects
    return () => es.close();
  }, [svc]);

  // Stick to the bottom unless the user has scrolled up.
  useEffect(() => {
    const el = preRef.current;
    if (el && atBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <section className="logview">
      <div className="logview-bar">
        <div className="logview-tabs">
          {LOG_SERVICES.map((s) => (
            <button key={s.id} className={`logtab ${svc === s.id ? "on" : ""}`} onClick={() => setSvc(s.id)}>
              {s.label}
            </button>
          ))}
        </div>
        <span className="logview-status">
          <span className={`led ${live ? "on" : ""}`} /> {live ? "streaming" : "connecting…"} · {lines.length} lines
          <button className="logclear" onClick={() => setLines([])}>clear</button>
        </span>
      </div>
      <pre
        className="logout"
        ref={preRef}
        onScroll={() => {
          const el = preRef.current;
          if (el) atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {lines.join("\n")}
      </pre>
    </section>
  );
}

const TX_ICON: Record<chain.TxStatus, string> = { pending: "⏳", ok: "✓", err: "✕" };

/** Live transaction log: every signed write the client submits, L1 + L2, with a
 *  chain badge, status, and a hash that links to the matching explorer. */
function TxLogViewer() {
  const [log, setLog] = useState<chain.TxEntry[]>([]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);

  useEffect(() => chain.subscribeTxLog(setLog), []);

  // Stick to the newest unless the user has scrolled up.
  useEffect(() => {
    const el = bodyRef.current;
    if (el && atBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [log]);

  return (
    <section className="txview">
      <div className="txview-bar">
        <span className="txview-status">
          {log.length} tx · <span className="tx-chain l1">L1</span> settlement ·{" "}
          <span className="tx-chain l2">L2</span> appchain
        </span>
        <button className="logclear" onClick={() => chain.clearTxLog()}>
          clear
        </button>
      </div>
      <div
        className="txout"
        ref={bodyRef}
        onScroll={() => {
          const el = bodyRef.current;
          if (el) atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {log.length === 0 ? (
          <div className="tx-empty">no transactions yet — fund, enter, or play</div>
        ) : (
          log.map((t) => (
            <div className={`txrow ${t.status}`} key={t.id} title={t.error ?? ""}>
              <span className={`tx-chain ${t.chain.toLowerCase()}`}>{t.chain}</span>
              <span className="tx-label">{t.label}</span>
              <span className="tx-st">{TX_ICON[t.status]}</span>
              {t.hash ? (
                <a
                  className="tx-hash"
                  href={chain.explorerTxUrl(t.explorer, t.hash)}
                  target="_blank"
                  rel="noreferrer"
                  title="view on explorer"
                >
                  {chain.shortHex(t.hash)} ↗
                </a>
              ) : (
                <span className="tx-hash dim">—</span>
              )}
            </div>
          ))
        )}
      </div>
    </section>
  );
}

// Shared stacking counter so clicking a window brings it above the others (and above
// the launcher buttons, which sit at z 8001).
let floatingWinZ = 8001;

/** Draggable, min/maximizable, resizable floating window. Hosts arbitrary content. */
function FloatingWindow({
  title,
  onClose,
  initial,
  children,
}: {
  title: string;
  onClose: () => void;
  initial?: { x: number; y: number; w: number; h: number };
  children: ReactNode;
}) {
  const [pos, setPos] = useState({ x: initial?.x ?? 14, y: initial?.y ?? 58 });
  const [size, setSize] = useState({ w: initial?.w ?? Math.min(760, window.innerWidth - 28), h: initial?.h ?? 440 });
  const [min, setMin] = useState(false);
  const [max, setMax] = useState(false);
  const [z, setZ] = useState(() => (floatingWinZ += 1)); // newest window opens on top
  const toFront = () => setZ((floatingWinZ += 1));
  const drag = useRef<{ dx: number; dy: number } | null>(null);
  // resize: which edges are active ("e" = right, "s" = bottom) + start geometry.
  const rz = useRef<{ e: boolean; s: boolean; x: number; y: number; w: number; h: number } | null>(null);

  useEffect(() => {
    const move = (ev: MouseEvent) => {
      if (drag.current) {
        setPos({ x: Math.max(0, ev.clientX - drag.current.dx), y: Math.max(0, ev.clientY - drag.current.dy) });
      } else if (rz.current) {
        const r = rz.current;
        setSize({
          w: r.e ? Math.max(360, r.w + (ev.clientX - r.x)) : r.w,
          h: r.s ? Math.max(180, r.h + (ev.clientY - r.y)) : r.h,
        });
      }
    };
    const up = () => {
      drag.current = null;
      rz.current = null;
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, []);

  const startResize = (e: boolean, s: boolean) => (ev: React.MouseEvent) => {
    ev.preventDefault();
    ev.stopPropagation();
    rz.current = { e, s, x: ev.clientX, y: ev.clientY, w: size.w, h: size.h };
  };

  return (
    <div
      className={`logwin${max ? " max" : ""}${min ? " min" : ""}`}
      onMouseDownCapture={toFront}
      style={max ? { zIndex: z } : { zIndex: z, left: pos.x, top: pos.y, width: size.w, height: min ? undefined : size.h }}
    >
      <div
        className="logwin-bar"
        onMouseDown={(e) => {
          if (!max) drag.current = { dx: e.clientX - pos.x, dy: e.clientY - pos.y };
        }}
        onDoubleClick={() => setMax((m) => !m)}
      >
        <span className="logwin-title">▸ {title}</span>
        <span className="logwin-ctrls" onMouseDown={(e) => e.stopPropagation()}>
          <button onClick={() => setMin((m) => !m)} title={min ? "restore" : "minimize"}>
            {min ? "▢" : "—"}
          </button>
          <button onClick={() => { setMax((m) => !m); setMin(false); }} title={max ? "restore" : "maximize"}>
            {max ? "❐" : "▣"}
          </button>
          <button onClick={onClose} title="close">
            ✕
          </button>
        </span>
      </div>
      {!min && <div className="logwin-body">{children}</div>}
      {!min && !max && (
        <>
          <div className="logwin-rz e" onMouseDown={startResize(true, false)} />
          <div className="logwin-rz s" onMouseDown={startResize(false, true)} />
          <div className="logwin-rz se" onMouseDown={startResize(true, true)} />
        </>
      )}
    </div>
  );
}

/** Deployment configuration: service URLs, contract addresses, saya progress. */
function ConfigPanel() {
  // "empty" = falsy or a zero address; non-hex values (URLs) are never empty.
  const z = (a: string) => {
    if (!a) return true;
    try {
      return BigInt(a) === 0n;
    } catch {
      return false;
    }
  };
  const Field = ({ label, value, href }: { label: string; value: string; href?: string }) => (
    <div className="cfg-row">
      <span className="cfg-k">{label}</span>
      {z(value) ? (
        <span className="cfg-v dim">—</span>
      ) : href ? (
        <a className="cfg-v tx-link" href={href} target="_blank" rel="noreferrer">
          {value} ↗
        </a>
      ) : (
        <span className="cfg-v">{value}</span>
      )}
    </div>
  );
  const l1 = (addr: string) => (z(addr) ? undefined : `${chain.SETTLEMENT_EXPLORER}/contract/${addr}`);

  return (
    <div className="cfg">
      <div className="cfg-sec">Services · {chain.SETTLEMENT_NAME}</div>
      <Field label="Settlement RPC" value={chain.SETTLEMENT_RPC} href={chain.SETTLEMENT_RPC} />
      <Field label="Appchain RPC" value={chain.APPCHAIN_RPC} href={chain.APPCHAIN_RPC} />
      <Field label="Torii · bank (L1)" value={chain.TORII_BANK} href={chain.TORII_BANK} />
      <Field label="Torii · game (L2)" value={chain.TORII_GAME} href={chain.TORII_GAME} />
      <Field label="Settlement explorer" value={chain.SETTLEMENT_EXPLORER} href={chain.SETTLEMENT_EXPLORER} />
      <Field label="Appchain explorer" value={chain.APPCHAIN_EXPLORER} href={chain.APPCHAIN_EXPLORER} />

      <div className="cfg-sec">{chain.SETTLEMENT_NAME} (L1) contracts</div>
      <Field label="piltover core" value={chain.PILTOVER} href={l1(chain.PILTOVER)} />
      <Field label="USDC (external)" value={chain.USDC} href={l1(chain.USDC)} />
      <Field label="GAME token" value={chain.GAME_TOKEN} href={l1(chain.GAME_TOKEN)} />
      <Field label="GOLD token" value={chain.GOLD_TOKEN} href={l1(chain.GOLD_TOKEN)} />
      <Field label="TokenSale" value={chain.TOKEN_SALE} href={l1(chain.TOKEN_SALE)} />
      <Field label="Entry" value={chain.ENTRY} href={l1(chain.ENTRY)} />
      <Field label="bank world" value={chain.BANK_WORLD} href={l1(chain.BANK_WORLD)} />
      <Field label="bank system" value={chain.BANK_SYSTEM} href={l1(chain.BANK_SYSTEM)} />

      <div className="cfg-sec">Appchain (L2) contracts</div>
      <Field label="game world" value={chain.GAME_WORLD} />
      <Field label="game system" value={chain.GAME_SYSTEM} />

      <div className="cfg-sec">Accounts</div>
      <Field label="settlement (operator)" value={chain.operatorAccount.address} href={l1(chain.operatorAccount.address)} />
      <Field label="appchain (dev)" value={chain.appchainAccount.address} />
    </div>
  );
}

export default function App() {
  const wallet = useWallet();
  const player = wallet.player;

  const [runState, setRunState] = useState<chain.RunState | null>(null); // last-loaded run state
  const [runs, setRuns] = useState<chain.RunState[]>([]); // the player's unfinished runs (lobby)
  const [selectedRun, setSelectedRun] = useState<number | null>(null); // run_no being played, or lobby
  // `runState` lags `selectedRun` by one poll on a switch (new game / continue), so only
  // treat it as the current run when it actually matches — otherwise the dungeon view would
  // briefly render the *previous* run's progress before the next poll replaces it.
  const run = runState && runState.runNo === selectedRun ? runState : null;
  const enteringRef = useRef<number | null>(null); // total_runs at "New game" click; auto-selects the mint
  const [stats, setStats] = useState<chain.Stats>({ totalRuns: 0, activeRuns: 0, totalActions: 0, totalBanked: 0 });
  const [feed, setFeed] = useState<chain.OutcomeRow[]>([]);
  const [board, setBoard] = useState<chain.LeaderRow[]>([]);
  // Resolved Cartridge Controller usernames, keyed by canonical address. Players
  // without a Controller simply stay absent (we fall back to the short address).
  const [names, setNames] = useState<Record<string, string>>({});
  const queriedAddrs = useRef<Set<string>>(new Set());
  // Look up usernames for any not-yet-queried addresses, in one batched call.
  const resolveNames = useCallback(async (addrs: string[]) => {
    const fresh = addrs.filter((a) => !queriedAddrs.current.has(addrKey(a)));
    if (!fresh.length) return;
    fresh.forEach((a) => queriedAddrs.current.add(addrKey(a)));
    try {
      const map = await lookupAddresses(fresh);
      if (map.size) {
        setNames((prev) => {
          const next = { ...prev };
          for (const [addr, name] of map) next[addrKey(addr)] = name;
          return next;
        });
      }
    } catch {
      // offline / Cartridge API unreachable — retry these next poll
      fresh.forEach((a) => queriedAddrs.current.delete(addrKey(a)));
    }
  }, []);
  // Seed your own row from the connected Controller, so it shows immediately.
  useEffect(() => {
    if (player && wallet.username) setNames((prev) => ({ ...prev, [addrKey(player)]: wallet.username! }));
  }, [player, wallet.username]);
  const [goldBal, setGoldBal] = useState(0n);
  const [vault, setVault] = useState(0); // accumulated GOLD on L2 awaiting bank
  const [settled, setSettled] = useState(0);
  const [tip, setTip] = useState(0);
  const [unclaimed, setUnclaimed] = useState<chain.WithdrawalRow[]>([]); // withdrawals not yet banked, oldest-first
  const [lastEnded, setLastEnded] = useState<chain.RunEndRow | null>(null);
  const [selected, setSelected] = useState<chain.OutcomeRow | null>(null);
  const [tab, setTab] = useState<"dungeon" | "bank" | "leaderboard">("dungeon"); // dungeon = L2, bank = L1
  const [logsOpen, setLogsOpen] = useState(false); // floating service-logs window
  const [configOpen, setConfigOpen] = useState(false); // floating deployment-config window
  const [txOpen, setTxOpen] = useState(false); // floating transaction-log window
  const [walletOpen, setWalletOpen] = useState(false); // account / connect-method modal
  // Show the guided appchain-mechanics walkthrough on every app load; the titlebar
  // button reopens it after dismissal.
  const [tutorial, setTutorial] = useState(true);
  const closeTutorial = () => setTutorial(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Outcome screen only shows for a run that ends *this session*: baseline the last
  // RunEnded seen at load, then show the veil only when a newer one appears. A reload
  // re-baselines, so a prior run's outcome doesn't reappear.
  const baselineEndNoRef = useRef<number | null>(null);
  const [dismissedEndNo, setDismissedEndNo] = useState(-1); // outcome veil closed by the user
  const [wdDetail, setWdDetail] = useState<chain.WithdrawalRow | null>(null); // withdrawal-details modal
  // Busy is tracked per layer so an in-flight L2 (appchain) action doesn't disable L1
  // (settlement) actions and vice versa — they're separate accounts/chains with their own
  // nonce locks, so they run concurrently.
  const [busyL1, setBusyL1] = useState<string | null>(null);
  const [busyL2, setBusyL2] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const inFlight = useRef(false);

  // Message log scroll-follow: newest renders at the bottom, so "caught up" means
  // scrolled to the bottom. When pinned there we auto-follow new entries; when the
  // user has scrolled up we don't yank them, we just count the unseen ones and show
  // a jump-to-latest button.
  const logRef = useRef<HTMLDivElement>(null);
  const seenEndRef = useRef(-1); // newest endNo the user has caught up to
  const [newLogs, setNewLogs] = useState(0); // unseen entries while scrolled up
  const logAtBottom = () => {
    const el = logRef.current;
    return !el || el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };
  const catchUpLog = () => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    seenEndRef.current = feed.length ? feed[0].endNo : seenEndRef.current;
    setNewLogs(0);
  };

  const ready = DEPLOYED && !!player && BigInt(player || "0x0") !== 0n;
  // The L1 entry buttons (buy / dev-mint / new game / enter again) stay clickable when
  // disconnected so the click opens the login modal (their handlers prompt to connect);
  // once connected, they need the deployments + player to be ready.
  const actReady = ready || wallet.method === null;

  const tick = useCallback(async () => {
    // Gate on deployments, NOT on a connected player: the global feed/leaderboard/stats
    // are world state and load even when nothing is connected (per-player reads below are
    // guarded by `if (player)`).
    if (!DEPLOYED || inFlight.current) return;
    inFlight.current = true;
    try {
      // Global reads (always shown, even on the disconnected starting page): world
      // stats, the run-outcome feed, the leaderboard, and the settled/tip gauge.
      // These come from the game world + RPC.
      const [st, fd, lb, sb, tp] = await Promise.all([
        chain.readStats(),
        chain.getOutcomeFeed(),
        chain.readLeaderboard(),
        chain.settledBlock(),
        chain.appchainBlock(),
      ]);
      setStats(st);
      setFeed(fd);
      setBoard(lb);
      void resolveNames(lb.map((r) => r.player));
      setSettled(sb);
      setTip(tp);

      // Per-player reads only when a wallet is connected. The starting page (empty
      // player) skips them — and the bank-world Torii (see the subscription effect) —
      // so idle work and memory stay down.
      if (player) {
        const [rl, r, gld, vt, wd, bc, le] = await Promise.all([
          chain.listRuns(player),
          selectedRun != null ? chain.readRun(selectedRun) : Promise.resolve(null),
          chain.goldBalance(player),
          chain.readVault(player),
          chain.getWithdrawals(player),
          chain.getBankCount(player),
          chain.getLastRunEnded(player),
        ]);
        setRuns(rl);
        setRunState(r);
        // When the selected run ends (death or extract) we deliberately KEEP it
        // selected so the outcome screen overlays the run page; the lobby transition
        // happens only when that screen is closed. After "New game", auto-select the
        // freshly minted run once it appears.
        if (enteringRef.current != null) {
          const fresh = rl.find((x) => x.runNo > enteringRef.current!);
          if (fresh) {
            setSelectedRun(fresh.runNo);
            enteringRef.current = null;
          }
        }
        setGoldBal(gld);
        setVault(vt);
        // Everything past the banked count is unclaimed (oldest-first). The Claim
        // button banks all of these that have settled, in one multicall.
        setUnclaimed(wd.slice(bc));
        setLastEnded(le);
        if (baselineEndNoRef.current === null) baselineEndNoRef.current = le ? le.endNo : -1;
      } else if (wallet.method === null) {
        // Truly disconnected (the starting page) — clear everything.
        setRuns([]);
        setRunState(null);
        setGoldBal(0n);
        setVault(0);
        setUnclaimed([]);
        setLastEnded(null);
      }
      // else: still connected but `player` is transiently empty — `useAccount()` blips to
      // undefined while the Controller switches chains (it does so on every action), which
      // would otherwise flash the stats/balances to zero until the address resolves. Keep
      // the last values instead of resetting.
      setErr(null);
    } catch (e) {
      setErr(String((e as Error).message || e));
    } finally {
      inFlight.current = false;
    }
  }, [player, selectedRun, wallet.method]);

  // The long-lived subscriptions/interval below always call the latest tick().
  const tickRef = useRef(tick);
  tickRef.current = tick;

  // Game-world live updates + the slow fallback poll. Long-lived (mount → unmount):
  // the game world (leaderboard, feed, runs) is shown even when nothing is connected.
  useEffect(() => {
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    chain
      .subscribeGameTorii(() => tickRef.current())
      .then((c) => {
        // eslint-disable-next-line no-console
        console.log("[torii] game-world subscription connected");
        if (cancelled) c();
        else cleanup = c;
      })
      .catch((e) => {
        // couldn't connect (stack down / not deployed) — the slow poll covers it.
        // eslint-disable-next-line no-console
        console.warn("[torii] game subscription unavailable, falling back to polling:", e);
      });
    const h = setInterval(() => tickRef.current(), 5000);
    return () => {
      cancelled = true;
      clearInterval(h);
      cleanup?.();
    };
  }, []);

  // Bank-world (Sepolia) live updates are per-player — only subscribe while a wallet is
  // connected, so the idle starting page runs a single torii-wasm client.
  useEffect(() => {
    if (!player) return;
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    chain
      .subscribeBankTorii(() => tickRef.current())
      .then((c) => {
        if (cancelled) c();
        else cleanup = c;
      })
      .catch((e) => {
        // eslint-disable-next-line no-console
        console.warn("[torii] bank subscription unavailable, falling back to polling:", e);
      });
    return () => {
      cancelled = true;
      cleanup?.();
    };
  }, [player]);

  // Re-read when the connected player, the selected run, or readiness changes.
  useEffect(() => {
    void tickRef.current();
  }, [player, selectedRun, ready]);

  // React to new log entries: follow if pinned to the bottom, else surface the count.
  useEffect(() => {
    const newest = feed.length ? feed[0].endNo : -1;
    if (newest < 0) return;
    if (seenEndRef.current < 0 || logAtBottom()) {
      // first load or caught up — stay pinned to the newest entry
      seenEndRef.current = newest;
      setNewLogs(0);
      requestAnimationFrame(() => {
        const el = logRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    } else {
      setNewLogs(feed.filter((a) => a.endNo > seenEndRef.current).length);
    }
  }, [feed]);

  const act = (layer: "l1" | "l2", name: string, fn: () => Promise<unknown>) => async () => {
    const setBusy = layer === "l1" ? setBusyL1 : setBusyL2;
    setBusy(name);
    setErr(null);
    try {
      await fn();
      await tick();
    } catch (e) {
      setErr(String((e as Error).message || e));
    } finally {
      setBusy(null);
    }
  };
  // Like act, but needs a connected signer — opens the wallet modal to connect otherwise.
  const actL1 = (name: string, fn: (acc: chain.Signer) => Promise<unknown>) =>
    act("l1", name, () => (wallet.l1Account ? fn(wallet.l1Account) : (setWalletOpen(true), Promise.resolve())));
  const actL2 = (name: string, fn: (acc: chain.Signer) => Promise<unknown>) =>
    act("l2", name, () => (wallet.l2Account ? fn(wallet.l2Account) : (setWalletOpen(true), Promise.resolve())));

  const playing = selectedRun != null;
  const inCombat = !!run && run.enemyHp > 0;
  // Transient combat juice (shake/flash/face), derived from polled run-state diffs.
  const sceneFx = useRunFx(run);
  // Bumped on each Attack click so the raycaster fires the weapon (muzzle flash +
  // recoil), independent of whether the on-chain hit lands.
  const [fireNonce, setFireNonce] = useState(0);
  // Bumped on each Move click so the raycaster plays a walk-forward step with a
  // fade that doubles as the room transition (the new room arrives a poll later).
  const [walkNonce, setWalkNonce] = useState(0);
  // Bumped on each Use click so the raycaster plays the potion-quaff animation.
  const [useNonce, setUseNonce] = useState(0);
  // Bumped on each Loot click so the raycaster plays the treasure-pickup animation.
  const [lootNonce, setLootNonce] = useState(0);
  // Floating "loot obtained" feed (bottom-left of the scene). We diff the *polled*
  // run so it reflects the real on-chain result — e.g. a mimic chest grants no gold,
  // so nothing pops. Each gain is a transient toast that fades after a moment.
  const [toasts, setToasts] = useState<{ id: number; text: string; kind: string }[]>([]);
  const toastId = useRef(0);
  const prevGains = useRef<{ runNo: number; gold: number; potions: number; hp: number; enemyHp: number; depth: number; roomKind: number; alive: boolean } | null>(null);
  const pushToast = useCallback((text: string, kind: string) => {
    const id = ++toastId.current;
    setToasts((t) => [...t, { id, text, kind }]);
    if (KIND_SFX[kind]) sfx(KIND_SFX[kind]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 2600);
  }, []);
  useEffect(() => {
    initSfx();
  }, []);
  useEffect(() => {
    if (!run) {
      prevGains.current = null;
      return;
    }
    const p = prevGains.current;
    prevGains.current = { runNo: run.runNo, gold: run.gold, potions: run.potions, hp: run.hp, enemyHp: run.enemyHp, depth: run.depth, roomKind: run.roomKind, alive: run.alive };
    if (!p || p.runNo !== run.runNo) return; // first sight / run switch: no toasts
    // Run just ended: extract leaves hp > 0 (success), death zeroes it (failure).
    if (p.alive && !run.alive) sfx(run.hp > 0 ? "getpow" : "death");
    if (run.gold > p.gold) pushToast(`+${run.gold - p.gold} $GOLD`, "gold");
    if (run.potions > p.potions) {
      const d = run.potions - p.potions;
      pushToast(`+${d} potion${d > 1 ? "s" : ""}`, "potion");
    }
    if (run.hp > p.hp) pushToast(`+${run.hp - p.hp} HP`, "hp");
    else if (run.hp < p.hp) pushToast(`-${p.hp - run.hp} HP`, "dmg");
    // Failed flee: still in the same monster room (depth + enemy_hp unchanged) but hp
    // dropped — the escape roll failed and the monster got a hit in.
    if (run.hp < p.hp && run.enemyHp > 0 && run.enemyHp === p.enemyHp && run.depth === p.depth) {
      pushToast("✗ FLEE FAILED", "flee");
    }
    // Ambush: advanced into a new room (depth up) that holds a fresh monster — flags a
    // new encounter, so it isn't mistaken for the one we just fled/moved from.
    if (run.depth > p.depth && run.roomKind === 1 && run.enemyHp > 0) {
      pushToast("⚔ AMBUSH!", "ambush");
    }
    // Mimic: looted a treasure (was a TREASURE room) but hp dropped with no advance —
    // the chest bit back instead of paying out.
    if (run.hp < p.hp && p.roomKind === 2 && run.depth === p.depth) {
      pushToast("⚠ MIMIC!", "mimic");
    }
    // Escaped: a flee that worked — we were in combat and advanced a room. Suppressed
    // when the new room is itself a monster, since that already shows AMBUSH.
    if (p.enemyHp > 0 && run.depth > p.depth && !(run.roomKind === 1 && run.enemyHp > 0)) {
      pushToast("✓ ESCAPED", "escaped");
    }
  }, [run?.runNo, run?.gold, run?.potions, run?.hp, run?.enemyHp, run?.depth, run?.roomKind, run?.alive, pushToast]);
  // A run that ended this session (newer than the load-time baseline) — drives the
  // death / extract outcome veils, so they don't reappear on reload.
  const freshOutcome =
    !!lastEnded &&
    baselineEndNoRef.current !== null &&
    lastEnded.endNo > baselineEndNoRef.current &&
    lastEnded.endNo > dismissedEndNo;
  // Closing the outcome screen (the ✕ button) dismisses the veil AND leaves the
  // just-ended run, dropping back to the New Game lobby. The outcome therefore stays
  // on the run page until the user closes it (no auto-dismiss).
  const closeOutcome = useCallback((endNo: number) => {
    setDismissedEndNo(endNo);
    setSelectedRun(null);
  }, []);
  const b = (n: string) => busyL1 === n || busyL2 === n;
  // True from the New-game click until the freshly minted run is selected — covers both
  // the L1 tx (busy "enter") and the L1→L2 relay wait (enteringRef still pending).
  const entering = b("enter") || enteringRef.current != null;
  // Per-layer busy: an in-flight L1 (settlement) action gates only L1 buttons; an L2
  // (appchain) action gates only L2 buttons — so the two layers don't block each other.
  const l1Busy = busyL1 !== null;
  const l2Busy = busyL2 !== null;
  // The selected run has ended (death/extract) but is still on screen.
  const runOver = !!run && !run.alive;
  // Show the outcome screen over the run page once the played run ends, or on the
  // lobby for a just-closed transition. Never over a still-alive run.
  const showOutcome = freshOutcome && !entering && (!playing || runOver);

  // "New game": fire the free, self-funding L1 enter (dev-mint → approve → enter, all
  // in one multicall — see chain.enterDungeon), which sends the L1→L2 mint_run. Remember
  // the run count at click time so the tick can auto-select the freshly minted run once
  // it shows up on the appchain. Owns its own lifecycle (rather than `act`) because the
  // loader is gated on `enteringRef`, which must be cleared if the L1 tx fails — else
  // the lobby stays stuck behind the loader with no way to retry.
  const onNewGame = async () => {
    if (!wallet.l1Account) {
      setWalletOpen(true);
      return;
    }
    enteringRef.current = stats ? stats.totalRuns : 0;
    setBusyL1("enter");
    setErr(null);
    try {
      await chain.enterDungeon(wallet.l1Account);
      await tick(); // may already see the minted run and auto-select it
    } catch (e) {
      enteringRef.current = null; // entry failed → lift the loader, back to the lobby
      setErr(String((e as Error).message || e));
    } finally {
      setBusyL1(null);
    }
  };
  const onContinue = (runNo: number) => setSelectedRun(runNo);
  const onLeave = () => setSelectedRun(null);
  // From the outcome page: dismiss the result and immediately start a fresh dive.
  const onEnterAgain = () => {
    if (lastEnded) setDismissedEndNo(lastEnded.endNo);
    setSelectedRun(null);
    void onNewGame();
  };
  // Play actions are signed by the appchain signer: the dev account, or the connected
  // Controller (switched to the appchain) — see wallet.l2Account.
  const onMove = actL2("move", (acc) => chain.moveRoom(selectedRun!, acc));
  const onAttack = actL2("attack", (acc) => chain.attack(selectedRun!, acc));
  const onLoot = actL2("loot", (acc) => chain.loot(selectedRun!, acc));
  const onUse = actL2("use", (acc) => chain.useItem(selectedRun!, acc));
  const onExtract = actL2("extract", (acc) => chain.extract(selectedRun!, acc));
  // Banking is two explicit actions, one per chain. "Withdraw" empties the L2 vault
  // into an L2→L1 message (signed by the appchain signer). "Claim" banks every settled
  // withdrawal at once — a single L1 multicall that consumes each settled message and
  // mints the GOLD (signed by the L1 signer). It's enabled once the oldest withdrawal
  // has settled; any not-yet-settled ones stay queued for the next Claim.
  const onWithdraw = actL2("withdraw", (acc) => chain.withdraw(player, acc));
  const onClaim = actL1("claim", async (acc) => {
    // Only settled withdrawals can be consumed; an unsettled one would revert the
    // whole multicall, so leave those for a later click.
    const claimable = unclaimed.filter((w) => settled >= w.block);
    if (!claimable.length) return;
    await chain.bankMany(acc, player, claimable);
  });

  const hp = run ? run.hp : 0;
  const maxHp = run ? run.maxHp : 100;
  const hpBar = (() => {
    const n = 11;
    const f = Math.max(0, Math.round((hp / Math.max(maxHp, 1)) * n));
    return (
      <>
        <span className="on">{"█".repeat(f)}</span>
        <span className="off">{"░".repeat(n - f)}</span>
      </>
    );
  })();

  // The oldest unbanked withdrawal drives the gate/label/modal. Because withdraw_no
  // climbs with block height and saya settles in order, the oldest always settles
  // first — so "oldest settled" means at least one is claimable.
  const pending = unclaimed[0] ?? null;
  const claimReady = !!pending && settled >= pending.block;
  // What a Claim click banks right now: every withdrawal already settled on L1. In the
  // common single-withdrawal flow this is just the one pending amount.
  const claimAmount = unclaimed.filter((w) => settled >= w.block).reduce((s, w) => s + w.amount, 0);
  // GOLD that can still be banked: gold sitting in the L2 vault (needs a withdraw)
  // plus everything withdrawn but not yet banked (needs settle + claim on L1).
  const bankable = vault + unclaimed.reduce((s, w) => s + w.amount, 0);

  return (
    <>
      <div className="crt">
        <div className="frame">
          <div className="titlebar">
            <span className="dots">
              <i /><i /><i />
            </span>
            <span className="path">diver@dungeon</span>
            <span>:</span>
            <span>~/run</span>
            <span className="spacer" />
            <button className="tut-launch" onClick={() => setTutorial(true)} title="how the appchain works">
              tutorial
            </button>
            <span>·</span>
            <button className="tut-launch settings-btn" onClick={() => setSettingsOpen(true)} title="settings" aria-label="settings">
              ⚙
            </button>
          </div>

          <div className="banner">
            <div>
              <h1 className="title">
                DUNGEON DUNGEON DUNGEON<span className="cur" />
              </h1>
              <div className="subtitle">push-your-luck roguelite</div>
            </div>
            <div className="chips">
              <button
                className={`chip ${wallet.method !== null ? "on" : ""} acct-chip`}
                onClick={() => setWalletOpen(true)}
                title={wallet.method === null ? "connect a wallet" : "account details"}
              >
                <span
                  className="led"
                  style={wallet.method !== null ? { background: "var(--gold)", boxShadow: "0 0 8px var(--gold)" } : undefined}
                />
                {wallet.connecting ? "connecting…" : wallet.method === null ? "login" : wallet.label}
              </button>
            </div>
          </div>

          <div className="tabs" data-tut="tabs">
            <button className={`tab ${tab === "dungeon" ? "on" : ""}`} onClick={() => setTab("dungeon")}>
              ▸ Dungeon
            </button>
            <button className={`tab ${tab === "bank" ? "on" : ""}`} onClick={() => setTab("bank")}>
              ▸ Bank
              {bankable > 0 && (
                <span
                  className={`tab-badge ${claimReady ? "ready" : "wait"}`}
                  title={claimReady ? `${claimAmount} GOLD ready to bank` : `${bankable} GOLD to bank`}
                >
                  {bankable}
                </span>
              )}
            </button>
            <button className={`tab ${tab === "leaderboard" ? "on" : ""}`} onClick={() => setTab("leaderboard")}>
              ▸ Leaderboard
            </button>
          </div>

          {tab === "dungeon" && (
          <main className={`grid${SHOW_RUNS_PANEL ? "" : " grid-no-right"}`}>
            {/* CENTER: three distinct views — the New Game page (lobby), the Dungeon
                run page, and the Outcome page — never share the screen. */}
            <section className="col-center" data-tut="play">
              {showOutcome && lastEnded ? (
                /* ===== Outcome page: the run result + lobby / re-enter actions ===== */
                <>
                  <div className={`outcome ${lastEnded.died ? "outcome-death" : "outcome-extract"}`}>
                    {lastEnded.died ? (
                      <>
                        <div className="death-skull">☠</div>
                        <div className="death-title">YOU DIED</div>
                        <div>
                          depth <b>{lastEnded.depth}</b> · <b>{lastEnded.loot.toLocaleString()}</b> gold forfeited
                        </div>
                        <div className="death-sub">your haul is lost.</div>
                      </>
                    ) : (
                      <>
                        <div className="extract-mark">✓</div>
                        <div className="extract-title">EXTRACTED</div>
                        <div className="extract-gold">+{lastEnded.loot.toLocaleString()} $GOLD</div>
                        <div className="death-sub">
                          depth <b>{lastEnded.depth}</b> · hp <b>{lastEnded.hp}/{lastEnded.maxHp}</b>
                        </div>
                      </>
                    )}
                  </div>

                  <div className="actions">
                    <button disabled={l2Busy} onClick={() => closeOutcome(lastEnded.endNo)}>
                      Back to lobby
                    </button>
                    <button
                      className="good"
                      disabled={l1Busy || !actReady}
                      onClick={() => {
                        sfx("teleport");
                        void onEnterAgain();
                      }}
                    >
                      Enter again
                    </button>
                  </div>
                </>
              ) : playing ? (
                /* ===== Dungeon run page: stage + vitals + actions ===== */
                <>
                  <div className="arena">
                    <div className="stage">
                      <div className="stage-h">
                        <button className="stage-h-back" disabled={l2Busy} onClick={onLeave} title="back to the New Game page">
                          ←
                        </button>
                        <span className="stage-h-label">
                          DUNGEON · <span className="kind">{run ? chain.roomLabel(run.roomKind) : "— idle —"}</span>
                        </span>
                        <span className="stage-h-active">{stats.activeRuns} active</span>
                      </div>
                      <DoomScene run={run} fx={sceneFx} fireNonce={fireNonce} walkNonce={walkNonce} useNonce={useNonce} lootNonce={lootNonce}>
                        {/* overlays live inside the canvas box so they center on the scene */}
                        {/* numeric gains/losses: small feed, bottom-left */}
                        {toasts.some((t) => !EVENT_KINDS.includes(t.kind)) && (
                          <div className="loot-feed">
                            {toasts
                              .filter((t) => !EVENT_KINDS.includes(t.kind))
                              .map((t) => (
                                <div key={t.id} className={`loot-toast ${t.kind}`}>
                                  {t.text}
                                </div>
                              ))}
                          </div>
                        )}
                        {/* dramatic events (ambush / flee failed): big centered banner */}
                        {toasts.some((t) => EVENT_KINDS.includes(t.kind)) && (
                          <div className="event-banner">
                            {toasts
                              .filter((t) => EVENT_KINDS.includes(t.kind))
                              .map((t) => (
                                <div key={t.id} className={`event-toast ${t.kind}`}>
                                  {t.text}
                                </div>
                              ))}
                          </div>
                        )}
                      </DoomScene>
                    </div>

                    {/* Doom-style status bar: ammo · health + face · level · score */}
                    <div className={`hud ${sceneFx ? "hud-" + sceneFx : ""}`}>
                      <div className="hud-stat hud-ammo">
                        <div className="hud-lbl">POTI</div>
                        <div className="hud-big">{run ? run.potions : "—"}</div>
                      </div>
                      <div className="hud-stat hud-health">
                        <div className="hud-lbl">HEALTH</div>
                        <div className="hud-big">
                          {run ? Math.round((run.hp / Math.max(run.maxHp, 1)) * 100) : "—"}
                          <span className="pct">%</span>
                        </div>
                        <div className="hud-meter">{run ? hpBar : ""}</div>
                      </div>
                      <div className="hud-face">
                        <pre>{(run ? doomFace(run, sceneFx) : FACE_OK).join("\n")}</pre>
                      </div>
                      <div className="hud-stat hud-depth">
                        <div className="hud-lbl">DEPTH</div>
                        <div className="hud-big">{run ? run.depth : "—"}</div>
                      </div>
                      <div className="hud-stat hud-gold">
                        <div className="hud-lbl">GOLD</div>
                        <div className="hud-big">{run ? run.gold.toLocaleString() : "—"}</div>
                      </div>
                    </div>
                  </div>

                  <div className="actions">
                    <button
                      disabled={l2Busy || !run || runOver}
                      onClick={() => {
                        sfx("door");
                        setWalkNonce((n) => n + 1);
                        void onMove();
                      }}
                    >
                      {b("move") ? "…" : inCombat ? "Flee" : "Move"}
                    </button>
                    <button
                      disabled={l2Busy || !inCombat || runOver}
                      onClick={() => {
                        sfx("shotgun");
                        setFireNonce((n) => n + 1);
                        void onAttack();
                      }}
                    >
                      {b("attack") ? "…" : "Attack"}
                    </button>
                    <button
                      disabled={l2Busy || !run || runOver || run.roomKind !== 2}
                      onClick={() => {
                        sfx("switch");
                        setLootNonce((n) => n + 1);
                        void onLoot();
                      }}
                    >
                      {b("loot") ? "…" : "Loot"}
                    </button>
                    <button
                      disabled={l2Busy || !run || runOver || run.potions === 0 || run.hp >= run.maxHp}
                      onClick={() => {
                        sfx("getpow");
                        setUseNonce((n) => n + 1);
                        void onUse();
                      }}
                    >
                      {b("use") ? "…" : "Use"}
                    </button>
                    <button
                      className="danger"
                      disabled={l2Busy || !run || runOver || inCombat}
                      onClick={() => {
                        sfx("teleport");
                        void onExtract();
                      }}
                    >
                      {b("extract") ? "…" : "Extract"}
                    </button>
                  </div>
                </>
              ) : (
                /* ===== New Game page: start a dive or continue an unfinished run ===== */
                <div className="newgame">
                  <div className="newgame-head">
                    <div className="newgame-title">LOBBY</div>
                    <div className="newgame-sub">into the unknown</div>
                  </div>
                  <button
                    className="good newgame-start"
                    disabled={l1Busy || !actReady}
                    onClick={() => {
                      // disconnected → onNewGame just opens the connect modal; save the
                      // enter-the-dungeon sfx for an actual entry
                      if (wallet.method !== null) sfx("teleport");
                      void onNewGame();
                    }}
                  >
                    {wallet.method === null ? "Login" : "+ New Game"}
                  </button>
                  {runs.length > 0 && (
                    <div className="lobby-list">
                      <div className="lobby-h">unfinished runs</div>
                      {runs.map((r) => (
                        <button key={r.runNo} className="lobby-run" onClick={() => onContinue(r.runNo)}>
                          <span className="lr-id">[r{r.runNo}]</span>
                          <span>d{r.depth}</span>
                          <span>hp{r.hp}</span>
                          <span>{r.gold.toLocaleString()}g</span>
                          <span className="lr-go">continue →</span>
                        </button>
                      ))}
                    </div>
                  )}
                  {/* Entering covers the whole lobby with the loader (the new run is being
                      minted on L2 via the L1→L2 message) until it auto-selects. */}
                  {entering && (
                    <div className="veil newgame-loading">
                      <div className="loading">
                        <div className="spinner" aria-hidden />
                        <div className="loading-title">entering the dungeon…</div>
                        <div className="loading-bar" aria-hidden />
                      </div>
                    </div>
                  )}
                </div>
              )}
            </section>

            {/* RIGHT: run-outcome log ("Runs" panel) — hidden via SHOW_RUNS_PANEL (top of file). */}
            {SHOW_RUNS_PANEL && (
            <section className="col-right" data-tut="log">
              <div className="panel-h">
                Runs<span className="rule" />
              </div>
              <div className="log-wrap">
              <div
                className="log"
                ref={logRef}
                onScroll={() => {
                  if (logAtBottom()) {
                    seenEndRef.current = feed.length ? feed[0].endNo : seenEndRef.current;
                    setNewLogs(0);
                  }
                }}
              >
                {feed.length === 0 ? (
                  <p className="sys">
                    <span className="t">--</span>
                    <span className="g">›</span>
                    <span className="m">no runs ended yet — extract or die to land here</span>
                  </p>
                ) : (
                  [...feed].reverse().map((a) => (
                    <p
                      key={a.endNo}
                      className={`logrow ${a.died ? "combat" : "good"}`}
                      onClick={() => setSelected(a)}
                      title="click for details + tx"
                    >
                      <span className="when" title={`run #${a.runNo} · finished ${new Date(a.ts).toLocaleString()}`}>
                        {fmtTime(a.ts)}
                      </span>
                      <span className="who" title={a.player}>{chain.shortAddr(a.player)}</span>
                      <span className="g">{a.died ? "☠" : "✓"}</span>
                      <span className="m">
                        <span className="c-kind">{a.died ? "died" : "extracted"}</span>
                        <span className="c-out">
                          d{a.depth} · {a.loot.toLocaleString()}g{a.died ? " lost" : ""}
                        </span>
                      </span>
                    </p>
                  ))
                )}
              </div>
                {newLogs > 0 && (
                  <button className="log-new" onClick={catchUpLog} title="jump to latest">
                    ↓ {newLogs} new {newLogs === 1 ? "log" : "logs"}
                  </button>
                )}
              </div>
            </section>
            )}
          </main>
          )}

          {tab === "bank" && (
          <main className="bank-page">
            <div className="bank-stack">
            <section className="bank-card" data-tut="bank">
              <div className="bank-chain">
                <span className="chip on">
                  <span className="led" />
                  {chain.SETTLEMENT_NAME.toUpperCase()} · L1
                </span>
              </div>
              <div className="panel-h">
                Bank your dungeon GOLD<span className="rule" />
              </div>
              <p className="bank-intro">
                <b>$GOLD</b> is earned in the dungeon (L2) but minted on <b>{chain.SETTLEMENT_NAME} (L1)</b>.
                Every extract banks a run's gold into your <b>vault</b> on the L2; you bank the whole vault
                to L1 in one go. <b>WITHDRAW</b> publishes a single L2→L1 message, and once <b>saya</b> settles
                it onto the piltover core, <b>mint</b> consumes the message and mints that much <b>$GOLD</b> here.
              </p>

              <div className="bal">
                {/*<span className="k">vault · ready to bank <small>(L2)</small></span>*/}
                <span className="k">vault</span>
                <span className="v">{bankable.toLocaleString()} <small>$GOLD</small></span>
              </div>
              <div className="bal gold">
                <span className="k">account balance</span>
                <span className="v">{chain.fmtToken(goldBal, chain.GOLD_DECIMALS, 0)} <small>$GOLD</small></span>
              </div>

              {bankable > 0 ? (
                <>
                  <div className="row-actions">
                    <button className="good" disabled={l2Busy || vault === 0} onClick={() => void onWithdraw()}>
                      {b("withdraw") ? "withdrawing…" : `Withdraw ${vault.toLocaleString()} $GOLD`}
                    </button>
                    <span className="flow-arrow" aria-hidden>
                      →
                    </span>
                    <button
                      className="good"
                      disabled={l1Busy || !pending || !claimReady}
                      onClick={() => void onClaim()}
                    >
                      {b("claim")
                        ? "claiming…"
                        : pending
                          ? claimReady
                            ? `Claim ${claimAmount.toLocaleString()} $GOLD`
                            : "awaiting saya…"
                          : "Claim"}
                    </button>
                  </div>
                  {unclaimed.length > 0 && (
                    <div className="wd">
                      <div className="wd-head">pending withdrawals</div>
                      <ul className="wd-list">
                        {unclaimed.map((w) => {
                          const ready = settled >= w.block;
                          return (
                            <li
                              key={w.withdrawNo}
                              className={ready ? "ready" : "wait"}
                              role="button"
                              tabIndex={0}
                              title="view withdrawal details"
                              onClick={() => setWdDetail(w)}
                              onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && setWdDetail(w)}
                            >
                              <span className="wd-no">#{w.withdrawNo}</span>
                              <span className="wd-amt">
                                {w.amount.toLocaleString()} <small>$GOLD</small>
                              </span>
                              <span className="wd-status">{ready ? "settled" : "awaiting saya"}</span>
                              <span className="wd-chev" aria-hidden>
                                ›
                              </span>
                            </li>
                          );
                        })}
                      </ul>
                    </div>
                  )}
                </>
              ) : (
                <></>
              )}
            </section>

            <section className="bank-card">
              <div className="panel-h">
                Settlement · saya<span className="rule" />
              </div>
              <Gauge settled={settled} tip={tip} />
              <p className="bank-intro" style={{ marginBottom: 0 }}>
                Your bank mints once <b>saya</b> settles the withdrawal's appchain block onto
                piltover — the lag above is how far L1 settlement trails the appchain tip.
              </p>
            </section>
            </div>
          </main>
          )}

          {tab === "leaderboard" && (
          <main className="board-page">
            <div className="board-stack">
              <section className="panel" data-tut="leaderboard">
                <div className="panel-h">
                  Leaderboard<span className="rule" />
                </div>
                <table>
                  <thead>
                    <tr>
                      <th>#</th>
                      <th>diver</th>
                      <th style={{ textAlign: "right" }}>best</th>
                      <th style={{ textAlign: "right" }}>gold</th>
                    </tr>
                  </thead>
                  <tbody>
                    {board.length === 0 ? (
                      <tr>
                        <td colSpan={4} className="r">
                          no runs yet
                        </td>
                      </tr>
                    ) : (
                      board.map((row, i) => (
                        <tr key={row.player} className={BigInt(row.player) === BigInt(player || "0x0") ? "you" : ""}>
                          <td className="r">{String(i + 1).padStart(2, "0")}</td>
                          {names[addrKey(row.player)] ? (
                            <td className="ctrl-name" title={row.player}>
                              {names[addrKey(row.player)]}
                            </td>
                          ) : (
                            <td title={row.player}>{chain.shortAddr(row.player)}</td>
                          )}
                          <td className="score">{row.bestScore.toLocaleString()}</td>
                          <td className="rw">{row.totalGold.toLocaleString()}</td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </section>
            </div>
          </main>
          )}

          {err && (
            <footer className="statusline">
              <span style={{ color: "var(--red)" }}>{chain.shortHex(err, 48, 0)}</span>
            </footer>
          )}
        </div>
      </div>
      {/* Dev-only debug launchers (service logs / deployment config / tx log). */}
      {import.meta.env.DEV && (
        <div className="launchers" data-tut="windows">
          <button className="launcher" onClick={() => setLogsOpen((o) => !o)} title="service logs">
            ▸ logs
          </button>
          <button className="launcher" onClick={() => setConfigOpen((o) => !o)} title="deployment config">
            ▸ config
          </button>
          <button className="launcher" onClick={() => setTxOpen((o) => !o)} title="transaction log (L1 + L2)">
            ▸ txns
          </button>
        </div>
      )}
      {logsOpen && (
        <FloatingWindow title="service logs" onClose={() => setLogsOpen(false)}>
          <LogViewer />
        </FloatingWindow>
      )}
      {configOpen && (
        <FloatingWindow
          title="deployment"
          onClose={() => setConfigOpen(false)}
          initial={{ x: 90, y: 92, w: Math.min(620, window.innerWidth - 28), h: 500 }}
        >
          <ConfigPanel />
        </FloatingWindow>
      )}
      {txOpen && (
        <FloatingWindow
          title="transactions"
          onClose={() => setTxOpen(false)}
          initial={{ x: 50, y: 76, w: Math.min(440, window.innerWidth - 28), h: 360 }}
        >
          <TxLogViewer />
        </FloatingWindow>
      )}
      {selected && <OutcomeModal outcome={selected} onClose={() => setSelected(null)} />}
      {walletOpen && <WalletModal onClose={() => setWalletOpen(false)} />}
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      {wdDetail && (
        <WithdrawalModal w={wdDetail} player={player} settled={settled} tip={tip} onClose={() => setWdDetail(null)} />
      )}
      {tutorial && <Tutorial onClose={closeTutorial} setTab={setTab} />}
      <div className="scanlines" />
      <div className="vignette" />
    </>
  );
}
