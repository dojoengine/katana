import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import * as chain from "./chain.ts";
import { useWallet } from "./wallet.tsx";

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
function DungeonMap({ run }: { run: chain.RunState | null }) {
  const W = 30;
  const H = 7;
  const lines: ReactNode[] = [];
  const feature = run ? ROOM_GLYPH[run.roomKind] : undefined;
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
      if (run && y === (H >> 1) && x === 8) {
        ch = "@";
        cls = "me";
      }
      if (run && feature && y === (H >> 1) && x === 19) {
        ch = feature.ch;
        cls = feature.cls;
      }
      cells.push(
        <span key={x} className={cls}>
          {ch}
        </span>,
      );
    }
    lines.push(
      <div key={y}>{cells}</div>,
    );
  }
  return <pre className="map">{lines}</pre>;
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
              <dd className={k === "tx hash" ? "mono-wrap" : ""}>{v}</dd>
            </div>
          ))}
        </dl>
      </div>
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
  const [usdcBal, setUsdcBal] = useState(0n);
  const [settled, setSettled] = useState(0);
  const [tip, setTip] = useState(0);
  const [pending, setPending] = useState<chain.ExtractRow | null>(null);
  const [lastEnded, setLastEnded] = useState<chain.RunEndRow | null>(null);
  const [selected, setSelected] = useState<chain.ActionRow | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const inFlight = useRef(false);

  const ready = !!player && BigInt(player || "0x0") !== 0n && BigInt(chain.GAME_SYSTEM) !== 0n;

  const tick = useCallback(async () => {
    if (!ready || inFlight.current) return;
    inFlight.current = true;
    try {
      const [r, st, fd, lb, gb, ub, sb, tp, ex, bc, le] = await Promise.all([
        chain.readRun(player),
        chain.readStats(),
        chain.getActionFeed(),
        chain.readLeaderboard(),
        chain.gameBalance(player),
        chain.usdcBalance(player),
        chain.settledBlock(),
        chain.appchainBlock(),
        chain.getExtracts(player),
        chain.getBankCount(player),
        chain.getLastRunEnded(player),
      ]);
      setRun(r);
      setStats(st);
      setFeed(fd);
      setBoard(lb);
      setGameBal(gb);
      setUsdcBal(ub);
      setSettled(sb);
      setTip(tp);
      setPending(ex.length > bc ? ex[bc] : null);
      setLastEnded(le);
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
  const b = (n: string) => busy === n;
  const anyBusy = busy !== null;

  const onBuy = act("buy", () => chain.buyGame(wallet.l1Account, BUY_USDC));
  const onMint = act("mint", () => chain.devMint(wallet.l1Account, DEV_MINT));
  const onEnter = act("enter", () => chain.enterDungeon(wallet.l1Account));
  const onMove = act("move", () => chain.moveRoom(player));
  const onAttack = act("attack", () => chain.attack(player));
  const onLoot = act("loot", () => chain.loot(player));
  const onUse = act("use", () => chain.useItem(player));
  const onExtract = act("extract", () => chain.extract(player));
  const onClaim = act("claim", () => (pending ? chain.claimRun(wallet.l1Account, player, pending.score, pending.loot) : Promise.resolve()));

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

  const claimReady = pending && settled >= pending.block;

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
          </div>

          <div className="banner">
            <div>
              <h1 className="title">
                CROSS<span className="x">-</span>CHAIN DUNGEON<span className="cur" />
              </h1>
              <div className="subtitle">
                push-your-luck roguelite · play on <b>DUNGEON</b> appchain · settle on <b>STARKNET SEPOLIA</b>
              </div>
            </div>
            <div className="chips">
              <span className="chip on">
                <span className="led" />
                SEPOLIA · settlement
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

          <Gauge settled={settled} tip={tip} />

          <main className="grid">
            {/* LEFT: funding + leaderboard */}
            <section className="col-left">
              <div className="panel">
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
                  <span className="k">GAME_TOKEN</span>
                  <span className="v">
                    {chain.fmtToken(gameBal, chain.GAME_DECIMALS, 0)} <small>DGOLD</small>
                  </span>
                </div>
                <div className="row-actions">
                  <button disabled={anyBusy || !ready} onClick={onBuy}>
                    {b("buy") ? "…" : "Buy GME"}
                  </button>
                  <button disabled={anyBusy || !ready} onClick={onMint}>
                    {b("mint") ? "…" : "Dev-mint"}
                  </button>
                </div>
                <div className="legend">
                  buy uses real USDC · dev-mint is the no-USDC faucet
                </div>
              </div>

              <div className="panel">
                <div className="panel-h">
                  Leaderboard · Sepolia<span className="rule" />
                </div>
                <table>
                  <thead>
                    <tr>
                      <th>#</th>
                      <th>diver</th>
                      <th style={{ textAlign: "right" }}>best</th>
                      <th style={{ textAlign: "right" }}>reward</th>
                    </tr>
                  </thead>
                  <tbody>
                    {board.length === 0 ? (
                      <tr>
                        <td colSpan={4} className="r">
                          no banked runs yet
                        </td>
                      </tr>
                    ) : (
                      board.map((row, i) => (
                        <tr key={row.player} className={BigInt(row.player) === BigInt(player || "0x0") ? "you" : ""}>
                          <td className="r">{String(i + 1).padStart(2, "0")}</td>
                          <td>{chain.shortHex(row.player)}</td>
                          <td className="score">{row.bestScore.toLocaleString()}</td>
                          <td className="rw">{chain.fmtToken(row.totalReward, chain.GAME_DECIMALS, 0)}</td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
                <div className="legend">banked runs only · deaths never settle</div>
              </div>
            </section>

            {/* CENTER: dungeon */}
            <section className="col-center">
              <div className="stage">
                <div className="stage-h">
                  <span>
                    DUNGEON · <span className="kind">{run ? chain.roomLabel(run.roomKind) : "— idle —"}</span>
                  </span>
                  <span>{stats.activeRuns} active</span>
                </div>
                <DungeonMap run={run} />
                {!run && (
                  <div className={`veil${b("enter") ? "" : lastEnded?.died ? " veil-death" : ""}`}>
                    {b("enter") ? (
                      <div>entering… (relaying L1→L2 mint_run)</div>
                    ) : lastEnded?.died ? (
                      <>
                        <div className="death-skull">☠</div>
                        <div className="death-title">YOU DIED</div>
                        <div>
                          depth <b>{lastEnded.depth}</b> · <b>{lastEnded.loot.toLocaleString()}</b> gold forfeited
                        </div>
                        <div className="death-sub">
                          nothing settled to L1 — the haul is lost. <b>ENTER DUNGEON</b> to try again.
                        </div>
                      </>
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

              <div className="actions">
                {!run ? (
                  <button className="good" disabled={anyBusy || !ready} onClick={onEnter}>
                    {b("enter") ? "entering…" : "Enter Dungeon"}
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

              {pending && (
                <div className="panel" style={{ marginTop: 16 }}>
                  <div className="panel-h">
                    Bank a settled run<span className="rule" />
                  </div>
                  <div className="bal">
                    <span className="k">
                      extracted · score {pending.score.toLocaleString()} · loot {pending.loot.toLocaleString()} · block {pending.block}
                    </span>
                  </div>
                  <div className="row-actions">
                    <button className="good" disabled={anyBusy || !claimReady} onClick={onClaim}>
                      {b("claim") ? "banking…" : claimReady ? "Bank → mint reward" : "awaiting saya…"}
                    </button>
                  </div>
                  <div className="legend">
                    consumes the L2→L1 message on Sepolia once settled, mints the GAME reward
                  </div>
                </div>
              )}
            </section>

            {/* RIGHT: message log */}
            <section className="col-right">
              <div className="panel-h">
                Message Log<span className="rule" />
              </div>
              <div className="log">
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
                      <span className="run" title={`run #${a.runNo}`}>r{a.runNo}</span>
                      <span className="t">d{a.depth}</span>
                      <span className="g">{KIND_GLYPH[a.kind] ?? "·"}</span>
                      <span className="m">
                        <span className="c-kind">{a.kind}</span>
                        <span className="c-out">{a.outcome}</span>
                        <span className="c-hp">hp {a.hp}</span>
                        <span className="c-gold">gold {a.gold}</span>
                      </span>
                    </p>
                  ))
                )}
              </div>
            </section>
          </main>

          <footer className="statusline">
            <span className="keys">
              runs <b>{stats.totalRuns}</b> · actions <b>{stats.totalActions}</b> · banked <b>{stats.totalBanked}</b>
            </span>
            <span className="spacer" />
            {err ? <span style={{ color: "var(--red)" }}>{chain.shortHex(err, 48, 0)}</span> : <span>dungeon$ ready</span>}
          </footer>
        </div>
      </div>
      {selected && <ActionModal action={selected} onClose={() => setSelected(null)} />}
      <div className="scanlines" />
      <div className="vignette" />
    </>
  );
}
