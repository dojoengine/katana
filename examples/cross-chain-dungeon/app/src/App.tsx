import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import * as chain from "./chain.ts";
import { useWallet } from "./wallet.tsx";
import { Tutorial } from "./tutorial.tsx";

// Demo amounts. Buy 1 USDC worth of GAME; dev-mint 500 GAME.
const BUY_USDC = 10n ** BigInt(chain.USDC_DECIMALS); // 1 USDC
const DEV_MINT = 500n * 10n ** BigInt(chain.GAME_DECIMALS); // 500 GAME

// The game world is deployed (deployments.json carries a real GAME_SYSTEM). The global
// run-outcome feed / leaderboard / stats are appchain-world state, not per-user, so they
// load whenever this is true — even with no wallet connected.
const DEPLOYED = BigInt(chain.GAME_SYSTEM) !== 0n;

const ROOM_GLYPH: Record<number, { ch: string; cls: string }> = {
  1: { ch: "M", cls: "mob" },
  2: { ch: "$", cls: "loot" },
  3: { ch: "^", cls: "trap" },
  4: { ch: "+", cls: "shrine" },
};
/** Render a small ASCII room from the run's current room kind. */
// The dungeon room keeps its original 7-row height but stretches to fill the
// container's width: we measure the <pre> and its character cell, then derive the
// column count so the walls span the full stage and reflow on resize. (`.map` is
// a block <pre>, so its width is container-driven — growing the grid never feeds
// back into the observer.)
// Run-finish time for the outcome log — local HH:MM:SS (24h).
const fmtTime = (ms: number): string =>
  Number.isFinite(ms)
    ? new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false })
    : "--:--:--";

const MAP_ROWS = 7;
function DungeonMap({ run }: { run: chain.RunState | null }) {
  const H = MAP_ROWS;
  const ref = useRef<HTMLPreElement>(null);
  const [W, setW] = useState(30);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const cs = getComputedStyle(el);
      const padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
      // Measure one character's advance (font + letter-spacing) via a hidden probe.
      const probe = document.createElement("span");
      probe.style.cssText = "position:absolute;visibility:hidden;white-space:pre";
      probe.style.fontFamily = cs.fontFamily;
      probe.style.fontSize = cs.fontSize;
      probe.style.fontWeight = cs.fontWeight;
      probe.style.letterSpacing = cs.letterSpacing;
      probe.textContent = "0".repeat(100);
      el.appendChild(probe);
      const charW = probe.getBoundingClientRect().width / 100;
      el.removeChild(probe);
      if (!charW) return;
      const w = Math.max(16, Math.floor((el.clientWidth - padX) / charW));
      setW((prev) => (prev === w ? prev : w));
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);

  const feature = run ? ROOM_GLYPH[run.roomKind] : undefined;
  const midY = H >> 1;
  const meX = Math.round(W * 0.27); // player ~quarter in
  const featX = Math.round(W * 0.63); // room feature past center
  const lines: ReactNode[] = [];
  for (let y = 0; y < H; y++) {
    const cells: ReactNode[] = [];
    for (let x = 0; x < W; x++) {
      const edge = x === 0 || x === W - 1 || y === 0 || y === H - 1;
      let ch = "·";
      let cls = "floor";
      if (edge) {
        cls = "wall";
        ch = x === 0 || x === W - 1 ? "║" : "═";
        if (y === 0 && x === 0) ch = "╔";
        if (y === 0 && x === W - 1) ch = "╗";
        if (y === H - 1 && x === 0) ch = "╚";
        if (y === H - 1 && x === W - 1) ch = "╝";
      }
      if (run && y === midY && x === meX) {
        ch = "@";
        cls = "me";
      }
      if (run && feature && y === midY && x === featX) {
        ch = feature.ch;
        cls = feature.cls;
      }
      cells.push(
        <span key={x} className={cls}>
          {ch}
        </span>,
      );
    }
    lines.push(<div key={y}>{cells}</div>);
  }
  return (
    <pre className="map" ref={ref}>
      {lines}
    </pre>
  );
}

function Gauge({ settled, tip }: { settled: number; tip: number }) {
  const n = 18;
  const safeTip = Math.max(tip, 1);
  const fill = Math.max(0, Math.min(n, Math.round((settled / safeTip) * n)));
  const lag = Math.max(0, tip - settled);
  return (
    <div className="gauge">
      <span className="lbl">SAYA SETTLEMENT</span>
      <span>
        settled <span className="n">{String(Math.max(settled, 0)).padStart(4, "0")}</span>
      </span>
      <span className="bar">
        ▕<span className="fill">{"█".repeat(fill)}</span>
        <span className="empty">{"░".repeat(n - fill)}</span>▏
      </span>
      <span>
        tip <span className="n">{String(tip).padStart(4, "0")}</span>
      </span>
      <span className="lag">{lag > 0 ? `${lag} block${lag > 1 ? "s" : ""} unsettled` : "fully settled"}</span>
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

/** Live progress for a withdraw → settle → mint bank, shown while it's in flight. */
function BankModal({
  phase,
  amount,
  withdrawNo,
  block,
  settled,
  tip,
  withdrawTx,
  mintTx,
  onClose,
}: {
  phase: "withdraw" | "settle" | "mint" | "done";
  amount: number;
  withdrawNo?: number;
  block?: number;
  settled: number;
  tip: number;
  withdrawTx?: string;
  mintTx?: string;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const order = ["withdraw", "settle", "mint"] as const;
  const idx = phase === "done" ? 3 : order.indexOf(phase);
  const steps = [
    { title: "Withdraw vault · L2", detail: `send ${amount.toLocaleString()} $GOLD as one L2→L1 message${withdrawNo != null ? ` (#${withdrawNo})` : ""}`, tx: withdrawTx, explorer: chain.APPCHAIN_EXPLORER },
    { title: `Settle · saya → ${chain.SETTLEMENT_NAME}`, detail: block != null ? `prove + settle appchain block ${block} · settled ${settled} / tip ${tip}` : "saya proves the block and settles it onto the piltover core", tx: undefined as string | undefined, explorer: undefined as string | undefined },
    { title: "Mint $GOLD · L1", detail: `consume the message and mint ${amount.toLocaleString()} $GOLD on ${chain.SETTLEMENT_NAME}`, tx: mintTx, explorer: chain.SETTLEMENT_EXPLORER },
  ];

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-h">
          <span>banking {amount.toLocaleString()} gold → GOLD</span>
          <button className="modal-x" onClick={onClose} aria-label="close">
            ✕
          </button>
        </div>
        <ol className="steps">
          {steps.map((s, i) => {
            const state = i < idx ? "done" : i === idx ? "active" : "todo";
            return (
              <li key={s.title} className={`step ${state}`}>
                <span className="step-mark">
                  {state === "done" ? "✓" : state === "active" ? <span className="spinner" aria-hidden /> : "·"}
                </span>
                <span className="step-body">
                  <span className="step-title">{s.title}</span>
                  <span className="step-detail">{s.detail}</span>
                  {s.tx && s.explorer && (
                    <a className="step-tx tx-link" href={chain.explorerTxUrl(s.explorer, s.tx)} target="_blank" rel="noreferrer">
                      {chain.shortHex(s.tx, 10, 8)} ↗
                    </a>
                  )}
                </span>
              </li>
            );
          })}
        </ol>
        <div className="legend">
          {phase === "done"
            ? `done — $GOLD minted on ${chain.SETTLEMENT_NAME}`
            : "this completes on its own — you can close this and keep playing"}
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
function ConfigPanel({ settled, tip }: { settled: number; tip: number }) {
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

      <div className="cfg-sec">Settlement · saya</div>
      <Gauge settled={settled} tip={tip} />
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
  const [gameBal, setGameBal] = useState(0n);
  const [goldBal, setGoldBal] = useState(0n);
  const [usdcBal, setUsdcBal] = useState(0n);
  const [vault, setVault] = useState(0); // accumulated GOLD on L2 awaiting bank
  const [fee, setFee] = useState(0n); // GAME entry fee
  const [settled, setSettled] = useState(0);
  const [tip, setTip] = useState(0);
  const [pending, setPending] = useState<chain.WithdrawalRow | null>(null); // unbanked withdrawal
  const [lastEnded, setLastEnded] = useState<chain.RunEndRow | null>(null);
  const [selected, setSelected] = useState<chain.OutcomeRow | null>(null);
  const [tab, setTab] = useState<"dungeon" | "bank">("dungeon"); // dungeon = L2, bank = L1
  const [logsOpen, setLogsOpen] = useState(false); // floating service-logs window
  const [configOpen, setConfigOpen] = useState(false); // floating deployment-config window
  const [txOpen, setTxOpen] = useState(false); // floating transaction-log window
  const [walletOpen, setWalletOpen] = useState(false); // account / connect-method modal
  // Show the guided appchain-mechanics walkthrough on every app load; the titlebar
  // button reopens it after dismissal.
  const [tutorial, setTutorial] = useState(true);
  const closeTutorial = () => setTutorial(false);
  const [minting, setMinting] = useState(false); // auto-mint (L1) in flight after a withdraw
  const mintingRef = useRef(false);
  // Outcome screen only shows for a run that ends *this session*: baseline the last
  // RunEnded seen at load, then show the veil only when a newer one appears. A reload
  // re-baselines, so a prior run's outcome doesn't reappear.
  const baselineEndNoRef = useRef<number | null>(null);
  const [dismissedEndNo, setDismissedEndNo] = useState(-1); // outcome veil closed by the user
  const [bankModal, setBankModal] = useState(false); // the withdraw/bank progress modal
  const [bankAmount, setBankAmount] = useState(0); // gold being banked (for the modal/done state)
  const [withdrawTx, setWithdrawTx] = useState<string | undefined>(); // L2 withdraw tx hash
  const [mintTx, setMintTx] = useState<string | undefined>(); // L1 mint tx hash
  const [busy, setBusy] = useState<string | null>(null);
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
      // stats, the run-outcome feed, the leaderboard, the entry fee, and the
      // settled/tip gauge. These come from the game world + RPC.
      const [st, fd, lb, ef, sb, tp] = await Promise.all([
        chain.readStats(),
        chain.getOutcomeFeed(),
        chain.readLeaderboard(),
        chain.entryFee(),
        chain.settledBlock(),
        chain.appchainBlock(),
      ]);
      setStats(st);
      setFeed(fd);
      setBoard(lb);
      setFee(ef);
      setSettled(sb);
      setTip(tp);

      // Per-player reads only when a wallet is connected. The starting page (empty
      // player) skips them — and the bank-world Torii (see the subscription effect) —
      // so idle work and memory stay down.
      if (player) {
        const [rl, r, gb, gld, ub, vt, wd, bc, le] = await Promise.all([
          chain.listRuns(player),
          selectedRun != null ? chain.readRun(selectedRun) : Promise.resolve(null),
          chain.gameBalance(player),
          chain.goldBalance(player),
          chain.usdcBalance(player),
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
        setGameBal(gb);
        setGoldBal(gld);
        setUsdcBal(ub);
        setVault(vt);
        setPending(wd.length > bc ? wd[bc] : null);
        setLastEnded(le);
        if (baselineEndNoRef.current === null) baselineEndNoRef.current = le ? le.endNo : -1;
      } else {
        setRuns([]);
        setRunState(null);
        setGameBal(0n);
        setGoldBal(0n);
        setUsdcBal(0n);
        setVault(0);
        setPending(null);
        setLastEnded(null);
      }
      setErr(null);
    } catch (e) {
      setErr(String((e as Error).message || e));
    } finally {
      inFlight.current = false;
    }
  }, [player, selectedRun]);

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

  const act = (name: string, fn: () => Promise<unknown>) => async () => {
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
    act(name, () => (wallet.l1Account ? fn(wallet.l1Account) : (setWalletOpen(true), Promise.resolve())));
  const actL2 = (name: string, fn: (acc: chain.Signer) => Promise<unknown>) =>
    act(name, () => (wallet.l2Account ? fn(wallet.l2Account) : (setWalletOpen(true), Promise.resolve())));

  const playing = selectedRun != null;
  const inCombat = !!run && run.enemyHp > 0;
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
  const b = (n: string) => busy === n;
  // True from the New-game click until the freshly minted run is selected — covers both
  // the L1 tx (busy "enter") and the L1→L2 relay wait (enteringRef still pending).
  const entering = b("enter") || enteringRef.current != null;
  const anyBusy = busy !== null;
  // The selected run has ended (death/extract) but is still on screen.
  const runOver = !!run && !run.alive;
  // Show the outcome screen over the run page once the played run ends, or on the
  // lobby for a just-closed transition. Never over a still-alive run.
  const showOutcome = freshOutcome && !entering && (!playing || runOver);

  const onBuy = actL1("buy", (acc) => chain.buyGame(acc, BUY_USDC));
  const onMint = actL1("mint", (acc) => chain.devMint(acc, DEV_MINT));
  // "New game": charge $GAME on L1, fire the L1→L2 mint_run, and remember the run
  // count at click time so the tick can auto-select the freshly minted run once it
  // shows up on the appchain. Owns its own lifecycle (rather than `act`) because the
  // loader is gated on `enteringRef`, which must be cleared if the L1 tx fails — else
  // the lobby stays stuck behind the loader with no way to retry.
  const onNewGame = async () => {
    if (!wallet.l1Account) {
      setWalletOpen(true);
      return;
    }
    if (gameBal < fee) {
      setErr(`insufficient $GAME (need ${chain.fmtToken(fee, chain.GAME_DECIMALS, 0)}) — buy or dev-mint`);
      return;
    }
    enteringRef.current = stats ? stats.totalRuns : 0;
    setBusy("enter");
    setErr(null);
    try {
      await chain.enterDungeon(wallet.l1Account);
      await tick(); // may already see the minted run and auto-select it
    } catch (e) {
      enteringRef.current = null; // entry failed → lift the loader, back to the lobby
      setErr(String((e as Error).message || e));
    } finally {
      setBusy(null);
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
  // Banking is one user action ("Withdraw"): the withdraw empties the vault into an
  // L2→L1 message; minting the GOLD on L1 then happens automatically once saya has
  // settled it (see the auto-mint effect below), so the user never presses a second
  // button. The mint runs off the poll loop, not a blocking await, so play isn't
  // blocked while settlement catches up.
  const onWithdraw = actL2("withdraw", async (acc) => setWithdrawTx(await chain.withdraw(player, acc)));

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

  const claimReady = !!pending && settled >= pending.block;
  // GOLD that can still be banked: gold sitting in the L2 vault (needs a withdraw)
  // plus any withdrawn-but-not-yet-banked amount (needs settle + bank on L1).
  const bankable = vault + (pending?.amount ?? 0);
  // A bank is mid-flight from withdraw through auto-mint; the button becomes a
  // "view progress" trigger and the modal shows which phase we're in.
  const bankInProgress = b("withdraw") || !!pending || minting;
  const bankPhase: "withdraw" | "settle" | "mint" | "done" = b("withdraw")
    ? "withdraw"
    : minting || (pending && claimReady)
      ? "mint"
      : pending
        ? "settle"
        : "done";

  // Auto-mint: once a withdrawal has settled on L1, consume it and mint the GOLD
  // without a second click. Runs off the poll loop (not a blocking await), so the
  // user can keep playing while settlement catches up. The ref guards against
  // double-firing across re-renders; on reload it also resumes any pending bank.
  useEffect(() => {
    const acc = wallet.l1Account;
    if (!pending || !claimReady || mintingRef.current || !acc) return;
    mintingRef.current = true;
    setMinting(true);
    chain
      .bankRun(acc, player, pending.amount, pending.withdrawNo)
      .then((tx) => {
        setMintTx(tx);
        return tick();
      })
      .catch((e) => setErr(String((e as Error).message || e)))
      .finally(() => {
        mintingRef.current = false;
        setMinting(false);
      });
  }, [pending, claimReady, wallet.l1Account, player, tick]);

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
            <span>tee: mock</span>
            <span>·</span>
            <span>saya: live</span>
            <span>·</span>
            <button className="tut-launch" onClick={() => setTutorial(true)} title="how the appchain works">
              tutorial
            </button>
          </div>

          <div className="banner">
            <div>
              <h1 className="title">
                CROSS<span className="x">-</span>CHAIN DUNGEON<span className="cur" />
              </h1>
              <div className="subtitle">
                push-your-luck roguelite · play on <b>DUNGEON</b> appchain · settle on <b>{chain.SETTLEMENT_NAME.toUpperCase()}</b>
              </div>
            </div>
            <div className="chips">
              <span className="chip on">
                <span className="led" />
                {chain.SETTLEMENT_NETWORK.toUpperCase()} · settlement
              </span>
              <span className="chip on">
                <span className="led" />
                DUNGEON · appchain
              </span>
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
                  title={claimReady ? `${pending?.amount} GOLD ready to bank` : `${bankable} GOLD to bank`}
                >
                  {bankable}
                </span>
              )}
            </button>
          </div>

          {tab === "dungeon" && (
          <main className="grid">
            {/* LEFT: funding + leaderboard */}
            <section className="col-left">
              <div className="panel" data-tut="fund">
                <div className="panel-h">
                  Funding<span className="rule" />
                </div>
                <div className="bal usdc">
                  <span className="k">USDC</span>
                  <span className="v">
                    {chain.fmtToken(usdcBal, chain.USDC_DECIMALS)} <small>USDC</small>
                  </span>
                </div>
                <div className="bal game">
                  <span className="k">GAME <small>entry credit</small></span>
                  <span className="v">
                    {chain.fmtToken(gameBal, chain.GAME_DECIMALS, 0)} <small>GAME</small>
                  </span>
                </div>
                <div className="bal gold">
                  <span className="k">GOLD <small>winnings · L1</small></span>
                  <span className="v">
                    {chain.fmtToken(goldBal, chain.GOLD_DECIMALS, 0)} <small>GOLD</small>
                  </span>
                </div>
                <div className="row-actions">
                  <button disabled={anyBusy || !actReady} onClick={onBuy}>
                    {b("buy") ? "…" : "Buy GAME"}
                  </button>
                  <button disabled={anyBusy || !actReady} onClick={onMint}>
                    {b("mint") ? "…" : "Dev-mint"}
                  </button>
                </div>
                <div className="legend">
                  spend <b>$GAME</b> to enter · earn <b>$GOLD</b> by banking · buy uses real <b>$USDC</b>
                </div>
              </div>

              <div className="panel" data-tut="leaderboard">
                <div className="panel-h">
                  Leaderboard · Appchain<span className="rule" />
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
                          <td>{chain.shortAddr(row.player)}</td>
                          <td className="score">{row.bestScore.toLocaleString()}</td>
                          <td className="rw">{row.totalGold.toLocaleString()}</td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
                <div className="legend">best run score per player · lives on L2</div>
              </div>
            </section>

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
                    <button disabled={anyBusy} onClick={() => closeOutcome(lastEnded.endNo)}>
                      Back to lobby
                    </button>
                    <button className="good" disabled={anyBusy || !actReady} onClick={onEnterAgain}>
                      {player && gameBal < fee
                        ? "insufficient $GAME"
                        : `Enter again · ${chain.fmtToken(fee, chain.GAME_DECIMALS, 0)} $GAME`}
                    </button>
                  </div>
                </>
              ) : playing ? (
                /* ===== Dungeon run page: stage + vitals + actions ===== */
                <>
                  <div className="arena">
                    <div className="stage">
                      <div className="stage-h">
                        <span>
                          DUNGEON · <span className="kind">{run ? chain.roomLabel(run.roomKind) : "— idle —"}</span>
                        </span>
                        <span>{stats.activeRuns} active</span>
                      </div>
                      <DungeonMap run={run} />
                    </div>

                    <div className="vitals">
                      <div className="vital hp">
                        <div className="k">HP</div>
                        <div className="v">{run ? `${run.hp}/${run.maxHp}` : "—"}</div>
                        <div className="meter">{run ? hpBar : ""}</div>
                      </div>
                      <div className="vital gold">
                        <div className="k">Gold</div>
                        <div className="v">{run ? run.gold.toLocaleString() : "—"}</div>
                        <div className="meter">haul on L2</div>
                      </div>
                      <div className="vital depth">
                        <div className="k">Depth</div>
                        <div className="v">{run ? run.depth : "—"}</div>
                        <div className="meter">rooms down</div>
                      </div>
                      <div className="vital pot">
                        <div className="k">Potions</div>
                        <div className="v">{run ? run.potions : "—"}</div>
                        <div className="meter">heals +35</div>
                      </div>
                    </div>
                  </div>

                  <div className="actions">
                    <button className="ghost" disabled={anyBusy} onClick={onLeave} title="back to the New Game page">
                      ←
                    </button>
                    <button disabled={anyBusy || !run || runOver} onClick={onMove}>
                      {b("move") ? "…" : inCombat ? "Flee" : "Move"}
                    </button>
                    <button disabled={anyBusy || !inCombat || runOver} onClick={onAttack}>
                      {b("attack") ? "…" : "Attack"}
                    </button>
                    <button disabled={anyBusy || !run || runOver || run.roomKind !== 2} onClick={onLoot}>
                      {b("loot") ? "…" : "Loot"}
                    </button>
                    <button disabled={anyBusy || !run || runOver || run.potions === 0 || run.hp >= run.maxHp} onClick={onUse}>
                      {b("use") ? "…" : "Use"}
                    </button>
                    <button className="danger" disabled={anyBusy || !run || runOver || inCombat} onClick={onExtract}>
                      {b("extract") ? "…" : "Extract"}
                    </button>
                  </div>
                </>
              ) : (
                /* ===== New Game page: start a dive or continue an unfinished run ===== */
                <div className="newgame">
                  <div className="newgame-head">
                    <div className="newgame-title">DUNGEON LOBBY</div>
                    <div className="newgame-sub">start a fresh dive — or continue an unfinished run</div>
                  </div>
                  <button className="good newgame-start" disabled={anyBusy || !actReady} onClick={onNewGame}>
                    {player && gameBal < fee
                      ? "insufficient $GAME"
                      : `+ New Game · ${chain.fmtToken(fee, chain.GAME_DECIMALS, 0)} $GAME`}
                  </button>
                  <div className="lobby-list">
                    <div className="lobby-h">unfinished runs</div>
                    {runs.length > 0 ? (
                      runs.map((r) => (
                        <button key={r.runNo} className="lobby-run" onClick={() => onContinue(r.runNo)}>
                          <span className="lr-id">[r{r.runNo}]</span>
                          <span>d{r.depth}</span>
                          <span>hp{r.hp}</span>
                          <span>{r.gold.toLocaleString()}g</span>
                          <span className="lr-go">continue →</span>
                        </button>
                      ))
                    ) : (
                      <div className="lobby-empty">none yet · start a new dive above</div>
                    )}
                  </div>
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

            {/* RIGHT: run-outcome log — every run's ending, by every player */}
            <section className="col-right" data-tut="log">
              <div className="panel-h">
                Run Outcomes<span className="rule" />
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
          </main>
          )}

          {tab === "bank" && (
          <main className="bank-page">
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
                    <button
                      className="good"
                      disabled={!bankInProgress && anyBusy}
                      onClick={() => {
                        if (bankInProgress) {
                          setBankModal(true);
                          return;
                        }
                        setBankAmount(vault);
                        setWithdrawTx(undefined);
                        setMintTx(undefined);
                        void onWithdraw();
                      }}
                    >
                      {b("withdraw")
                        ? "withdrawing…"
                        : minting || (pending && claimReady)
                          ? "minting GOLD…"
                          : pending
                            ? "awaiting saya…"
                            : `Withdraw ${vault.toLocaleString()} $GOLD`}
                    </button>
                  </div>
                  {/*<div className="legend">
                    {bankInProgress
                      ? "banking in progress — click for the live phase breakdown"
                      : "one button: withdraws the whole vault, then auto-mints the GOLD on L1 once saya settles it"}
                  </div>*/}
                </>
              ) : (
                <></>
              )}
            </section>
          </main>
          )}

          <footer className="statusline">
            <span className="keys">
              runs <b>{stats.totalRuns}</b> · actions <b>{stats.totalActions}</b> · banked <b>{stats.totalBanked}</b>
            </span>
            <span className="spacer" />
            {err ? <span style={{ color: "var(--red)" }}>{chain.shortHex(err, 48, 0)}</span> : <span>dungeon$ ready</span>}
          </footer>
        </div>
      </div>
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
          <ConfigPanel settled={settled} tip={tip} />
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
      {bankModal && (
        <BankModal
          phase={bankPhase}
          amount={pending?.amount ?? bankAmount}
          withdrawNo={pending?.withdrawNo}
          block={pending?.block}
          settled={settled}
          tip={tip}
          withdrawTx={withdrawTx}
          mintTx={mintTx}
          onClose={() => setBankModal(false)}
        />
      )}
      {tutorial && <Tutorial onClose={closeTutorial} setTab={setTab} />}
      <div className="scanlines" />
      <div className="vignette" />
    </>
  );
}
