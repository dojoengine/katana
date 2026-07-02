import { useCallback, useEffect, useState } from "react";
import {
  APPCHAIN_EXPLORER,
  fetchState,
  playAll,
  SETTLEMENT_EXPLORER,
  type ArcadeState,
} from "./chain.ts";

const short = (a: string) => (a && a !== "0x0" ? `${a.slice(0, 6)}…${a.slice(-4)}` : "—");

export function App() {
  const [state, setState] = useState<ArcadeState | null>(null);
  const [busy, setBusy] = useState(false);
  const [lastTx, setLastTx] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [flash, setFlash] = useState<Record<string, number>>({});

  const refresh = useCallback(async () => {
    try {
      const next = await fetchState();
      setState((prev) => {
        if (prev) {
          // Flash any machine whose coin count just went up.
          const bumped: Record<string, number> = {};
          for (const m of next.machines) {
            const before = prev.machines.find((x) => x.address === m.address);
            if (before && m.coins > before.coins) bumped[m.address] = Date.now();
          }
          if (Object.keys(bumped).length) setFlash((f) => ({ ...f, ...bumped }));
        }
        return next;
      });
    } catch (e) {
      // transient during boot
    }
  }, []);

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 1500);
    return () => clearInterval(id);
  }, [refresh]);

  const onPlay = async () => {
    setBusy(true);
    setError(null);
    try {
      const tx = await playAll();
      setLastTx(tx);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const machines = state?.machines ?? [];
  const pending = state?.pending ?? 0;

  return (
    <div className="wrap">
      <header>
        <h1>🕹️ Cross-Chain Arcade</h1>
        <p className="tag">
          One L1 transaction → an L1→L2 message to <b>every</b> machine (each a
          distinct L2 contract). Validates katana{" "}
          <a href="https://github.com/dojoengine/katana/pull/623" target="_blank" rel="noreferrer">
            PR #623
          </a>
          : before the fix, only the first machine would ever get its coin.
        </p>
      </header>

      <div className="controls">
        <button onClick={onPlay} disabled={busy}>
          {busy ? "Dispensing…" : "🪙 Insert Coins (Play All)"}
        </button>
        <div className="stats">
          <span>
            sent <b>{state?.sent ?? 0}</b>
          </span>
          <span>
            landed <b>{state?.landed ?? 0}</b>
          </span>
          <span className={pending ? "pending on" : "pending"}>
            {pending ? `⏳ ${pending} relaying…` : "✓ all relayed"}
          </span>
        </div>
      </div>

      {lastTx && (
        <p className="txline">
          L1 tx:{" "}
          <a href={`${SETTLEMENT_EXPLORER}/tx/${lastTx}`} target="_blank" rel="noreferrer">
            {short(lastTx)}
          </a>{" "}
          — fanned out to {machines.length} machines.
        </p>
      )}
      {error && <p className="error">{error}</p>}

      <div className="grid">
        {machines.map((m) => {
          const lit = flash[m.address] && Date.now() - flash[m.address] < 1200;
          return (
            <div className={lit ? "card lit" : "card"} key={m.address}>
              <div className="cardhead">
                <span className="name">{m.name}</span>
                <a
                  href={`${APPCHAIN_EXPLORER}/contract/${m.address}`}
                  target="_blank"
                  rel="noreferrer"
                  className="addr"
                >
                  {short(m.address)}
                </a>
              </div>
              <div className="coins">{m.coins}</div>
              <div className="sub">coins received</div>
              <div className="last">last player {short(m.lastPlayer)}</div>
            </div>
          );
        })}
      </div>

      <footer>
        <span>settlement (L1) :5050</span>
        <span>appchain (L2) :5051</span>
        <span>reads via RPC — no indexer</span>
      </footer>
    </div>
  );
}
