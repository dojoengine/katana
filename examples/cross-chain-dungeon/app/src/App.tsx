import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import * as chain from "./chain.ts";
import { useWallet } from "./wallet.tsx";
import { Tutorial } from "./tutorial.tsx";

// Demo amounts. Buy 1 USDC worth of GAME; dev-mint 500 GAME.
const BUY_USDC = 10n ** BigInt(chain.USDC_DECIMALS); // 1 USDC
const DEV_MINT = 500n * 10n ** BigInt(chain.GAME_DECIMALS); // 500 GAME

const ROOM_GLYPH: Record<number, { ch: string; cls: string }> = {
  1: { ch: "M", cls: "mob" },
  2: { ch: "$", cls: "loot" },
  3: { ch: "^", cls: "trap" },
  4: { ch: "+", cls: "shrine" },
};
const KIND_GLYPH: Record<string, string> = {
  move: "»",
  attack: "⚔",
  loot: "$",
  use_item: "+",
};
const KIND_CLASS: Record<string, string> = {
  move: "move",
  attack: "attack",
  loot: "loot",
  use_item: "good",
};

/** Render a small ASCII room from the run's current room kind. */
// The dungeon room keeps its original 7-row height but stretches to fill the
// container's width: we measure the <pre> and its character cell, then derive the
// column count so the walls span the full stage and reflow on resize. (`.map` is
// a block <pre>, so its width is container-driven — growing the grid never feeds
// back into the observer.)
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

/** Detail modal for a clicked message-log entry, with a link to its appchain tx. */
function ActionModal({ action, onClose }: { action: chain.ActionRow; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const txUrl = chain.explorerTxUrl(chain.APPCHAIN_EXPLORER, action.txHash);
  const rows: [string, ReactNode][] = [
    ["run", `#${action.runNo}`],
    ["action", `#${action.actionNo}`],
    ["player", action.player],
    ["kind", action.kind],
    ["outcome", action.outcome],
    ["depth", String(action.depth)],
    ["hp", String(action.hp)],
    ["gold", String(action.gold)],
    ["appchain block", String(action.block)],
    [
      "tx hash",
      // The hash itself is the link to the action's tx on the appchain explorer.
      <a className="tx-link" href={txUrl} target="_blank" rel="noreferrer" title="view on appchain explorer">
        {action.txHash} ↗
      </a>,
    ],
  ];

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-h">
          <span>
            {KIND_GLYPH[action.kind] ?? "·"} action #{action.actionNo}
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

  const [run, setRun] = useState<chain.RunState | null>(null);
  const [stats, setStats] = useState<chain.Stats>({ totalRuns: 0, activeRuns: 0, totalActions: 0, totalBanked: 0 });
  const [feed, setFeed] = useState<chain.ActionRow[]>([]);
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
  const [selected, setSelected] = useState<chain.ActionRow | null>(null);
  const [tab, setTab] = useState<"dungeon" | "bank">("dungeon"); // dungeon = L2, bank = L1
  const [logsOpen, setLogsOpen] = useState(false); // floating service-logs window
  const [configOpen, setConfigOpen] = useState(false); // floating deployment-config window
  const [tutorial, setTutorial] = useState(false); // guided appchain-mechanics walkthrough
  // Auto-open the tutorial on a first visit (per browser); the titlebar button reopens it.
  useEffect(() => {
    try {
      if (!localStorage.getItem("ccd_tutorial_seen")) setTutorial(true);
    } catch {
      /* storage unavailable (private mode) — skip auto-open */
    }
  }, []);
  const closeTutorial = () => {
    setTutorial(false);
    try {
      localStorage.setItem("ccd_tutorial_seen", "1");
    } catch {
      /* ignore */
    }
  };
  const [minting, setMinting] = useState(false); // auto-mint (L1) in flight after a withdraw
  const mintingRef = useRef(false);
  // Outcome screen only shows for a run that ends *this session*: baseline the last
  // RunEnded seen at load, then show the veil only when a newer one appears. A reload
  // re-baselines, so a prior run's outcome doesn't reappear.
  const baselineEndNoRef = useRef<number | null>(null);
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
  const seenActionRef = useRef(-1); // newest actionNo the user has caught up to
  const [newLogs, setNewLogs] = useState(0); // unseen entries while scrolled up
  const logAtBottom = () => {
    const el = logRef.current;
    return !el || el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };
  const catchUpLog = () => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    seenActionRef.current = feed.length ? feed[0].actionNo : seenActionRef.current;
    setNewLogs(0);
  };

  const ready = !!player && BigInt(player || "0x0") !== 0n && BigInt(chain.GAME_SYSTEM) !== 0n;

  const tick = useCallback(async () => {
    if (!ready || inFlight.current) return;
    inFlight.current = true;
    try {
      const [r, st, fd, lb, gb, gld, ub, vt, ef, sb, tp, wd, bc, le] = await Promise.all([
        chain.readRun(player),
        chain.readStats(),
        chain.getActionFeed(),
        chain.readLeaderboard(),
        chain.gameBalance(player),
        chain.goldBalance(player),
        chain.usdcBalance(player),
        chain.readVault(player),
        chain.entryFee(),
        chain.settledBlock(),
        chain.appchainBlock(),
        chain.getWithdrawals(player),
        chain.getBankCount(player),
        chain.getLastRunEnded(player),
      ]);
      setRun(r);
      setStats(st);
      setFeed(fd);
      setBoard(lb);
      setGameBal(gb);
      setGoldBal(gld);
      setUsdcBal(ub);
      setVault(vt);
      setFee(ef);
      setSettled(sb);
      setTip(tp);
      setPending(wd.length > bc ? wd[bc] : null);
      setLastEnded(le);
      if (baselineEndNoRef.current === null) baselineEndNoRef.current = le ? le.endNo : -1;
      setErr(null);
    } catch (e) {
      setErr(String((e as Error).message || e));
    } finally {
      inFlight.current = false;
    }
  }, [player, ready]);

  useEffect(() => {
    tick();
    // Push updates: refetch the instant a model/event changes in either world.
    let cleanup: (() => void) | undefined;
    let cancelled = false;
    chain
      .subscribeToriiUpdates(() => tick())
      .then((c) => {
        // eslint-disable-next-line no-console
        console.log("[torii] live subscriptions connected (game + score worlds)");
        if (cancelled) c();
        else cleanup = c;
      })
      .catch((e) => {
        // couldn't connect (stack down / not deployed) — the slow poll covers it.
        // eslint-disable-next-line no-console
        console.warn("[torii] subscription unavailable, falling back to polling:", e);
      });
    // Slow fallback + the RPC-only facts (token balances, settled height, tip)
    // that have no Torii subscription.
    const h = setInterval(tick, 5000);
    return () => {
      cancelled = true;
      clearInterval(h);
      cleanup?.();
    };
  }, [tick]);

  // React to new log entries: follow if pinned to the bottom, else surface the count.
  useEffect(() => {
    const newest = feed.length ? feed[0].actionNo : -1;
    if (newest < 0) return;
    if (seenActionRef.current < 0 || logAtBottom()) {
      // first load or caught up — stay pinned to the newest entry
      seenActionRef.current = newest;
      setNewLogs(0);
      requestAnimationFrame(() => {
        const el = logRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });
    } else {
      setNewLogs(feed.filter((a) => a.actionNo > seenActionRef.current).length);
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

  const inCombat = !!run && run.enemyHp > 0;
  // A run that ended this session (newer than the load-time baseline) — drives the
  // death / extract outcome veils, so they don't reappear on reload.
  const freshOutcome = !!lastEnded && baselineEndNoRef.current !== null && lastEnded.endNo > baselineEndNoRef.current;
  const b = (n: string) => busy === n;
  const anyBusy = busy !== null;

  const onBuy = act("buy", () => chain.buyGame(wallet.l1Account, BUY_USDC));
  const onMint = act("mint", () => chain.devMint(wallet.l1Account, DEV_MINT));
  const enterRun = act("enter", () => chain.enterDungeon(wallet.l1Account));
  const onEnter = () => {
    if (gameBal < fee) {
      setErr(`insufficient $GAME (need ${chain.fmtToken(fee, chain.GAME_DECIMALS, 0)}) — buy or dev-mint`);
      return;
    }
    void enterRun();
  };
  const onMove = act("move", () => chain.moveRoom(player));
  const onAttack = act("attack", () => chain.attack(player));
  const onLoot = act("loot", () => chain.loot(player));
  const onUse = act("use", () => chain.useItem(player));
  const onExtract = act("extract", () => chain.extract(player));
  // Banking is one user action ("Withdraw"): the withdraw empties the vault into an
  // L2→L1 message; minting the GOLD on L1 then happens automatically once saya has
  // settled it (see the auto-mint effect below), so the user never presses a second
  // button. The mint runs off the poll loop, not a blocking await, so play isn't
  // blocked while settlement catches up.
  const onWithdraw = act("withdraw", async () => setWithdrawTx(await chain.withdraw(player)));

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
    if (!pending || !claimReady || mintingRef.current) return;
    mintingRef.current = true;
    setMinting(true);
    chain
      .bankRun(wallet.l1Account, player, pending.amount, pending.withdrawNo)
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
              <span className="chip on">
                <span className="led" style={{ background: "var(--gold)", boxShadow: "0 0 8px var(--gold)" }} />
                {wallet.label}
                {wallet.method === "operator" && wallet.controllerAvailable ? (
                  <button style={{ flex: "none", padding: "0 6px" }} disabled={wallet.connecting} onClick={() => void wallet.connectController()}>
                    {wallet.connecting ? "…" : "login"}
                  </button>
                ) : wallet.method === "controller" ? (
                  <button style={{ flex: "none", padding: "0 6px" }} onClick={() => void wallet.useOperator()}>
                    logout
                  </button>
                ) : null}
              </span>
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
                  <button disabled={anyBusy || !ready} onClick={onBuy}>
                    {b("buy") ? "…" : "Buy GAME"}
                  </button>
                  <button disabled={anyBusy || !ready} onClick={onMint}>
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

            {/* CENTER: dungeon */}
            <section className="col-center">
              <div className="arena">
                <div className="stage">
                  <div className="stage-h">
                    <span>
                      DUNGEON · <span className="kind">{run ? chain.roomLabel(run.roomKind) : "— idle —"}</span>
                    </span>
                    <span>{stats.activeRuns} active</span>
                  </div>
                  <DungeonMap run={run} />
                  {!run && (b("enter") || !freshOutcome) && (
                    <div className="veil">
                      {b("enter") ? (
                        <div className="loading">
                          <div className="spinner" aria-hidden />
                          <div className="loading-title">entering the dungeon…</div>
                          <div className="loading-bar" aria-hidden />
                        </div>
                      ) : (
                        <>
                          <div>no active run</div>
                          <div>
                            get <b>GME</b>, then <b>ENTER DUNGEON</b> to descend
                          </div>
                        </>
                      )}
                    </div>
                  )}
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

                {!run && !b("enter") && freshOutcome && lastEnded?.died && (
                  <div className="veil veil-death arena-veil">
                    <div className="death-skull">☠</div>
                    <div className="death-title">YOU DIED</div>
                    <div>
                      depth <b>{lastEnded.depth}</b> · <b>{lastEnded.loot.toLocaleString()}</b> gold forfeited
                    </div>
                    <div className="death-sub">
                      the haul is lost. <b>ENTER DUNGEON</b> to try again.
                    </div>
                  </div>
                )}

                {!run && !b("enter") && freshOutcome && lastEnded && !lastEnded.died && (
                  <div className="veil veil-extract arena-veil">
                    <div className="extract-mark">✓</div>
                    <div className="extract-title">EXTRACTED</div>
                    <div className="extract-gold">+{lastEnded.loot.toLocaleString()} $GOLD</div>
                    <div className="death-sub">
                      depth <b>{lastEnded.depth}</b> · hp <b>{lastEnded.hp}/{lastEnded.maxHp}</b>
                    </div>
                  </div>
                )}
              </div>

              <div className="actions" data-tut="play">
                {!run ? (
                  <button className="good" disabled={anyBusy || !ready} onClick={onEnter}>
                    {b("enter")
                      ? "entering…"
                      : gameBal < fee
                        ? "insufficient $GAME"
                        : `Enter Dungeon · ${chain.fmtToken(fee, chain.GAME_DECIMALS, 0)} $GAME`}
                  </button>
                ) : (
                  <>
                    <button disabled={anyBusy} onClick={onMove}>
                      {b("move") ? "…" : inCombat ? "Flee" : "Move"}
                    </button>
                    <button disabled={anyBusy || !inCombat} onClick={onAttack}>
                      {b("attack") ? "…" : "Attack"}
                    </button>
                    <button disabled={anyBusy || run.roomKind !== 2} onClick={onLoot}>
                      {b("loot") ? "…" : "Loot"}
                    </button>
                    <button disabled={anyBusy || run.potions === 0 || run.hp >= run.maxHp} onClick={onUse}>
                      {b("use") ? "…" : "Use"}
                    </button>
                    <button className="danger" disabled={anyBusy || inCombat} onClick={onExtract}>
                      {b("extract") ? "…" : "Extract"}
                    </button>
                  </>
                )}
              </div>
            </section>

            {/* RIGHT: message log */}
            <section className="col-right" data-tut="log">
              <div className="panel-h">
                Message Log<span className="rule" />
              </div>
              <div className="log-wrap">
              <div
                className="log"
                ref={logRef}
                onScroll={() => {
                  if (logAtBottom()) {
                    seenActionRef.current = feed.length ? feed[0].actionNo : seenActionRef.current;
                    setNewLogs(0);
                  }
                }}
              >
                {feed.length === 0 ? (
                  <p className="sys">
                    <span className="t">--</span>
                    <span className="g">›</span>
                    <span className="m">no actions yet — enter the dungeon</span>
                  </p>
                ) : (
                  [...feed].reverse().map((a) => (
                    <p
                      key={a.actionNo}
                      className={`logrow ${KIND_CLASS[a.kind] ?? "sys"}`}
                      onClick={() => setSelected(a)}
                      title="click for details + tx"
                    >
                      <span className="run" title={`run #${a.runNo}`}>[r{a.runNo}]</span>
                      <span className="who" title={a.player}>{chain.shortAddr(a.player)}</span>
                      <span className="g">{KIND_GLYPH[a.kind] ?? "·"}</span>
                      <span className="m">
                        <span className="c-kind">{a.kind}</span>
                        <span className="c-out">{a.outcome}</span>
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
      {selected && <ActionModal action={selected} onClose={() => setSelected(null)} />}
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
