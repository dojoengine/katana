// Guided tutorial: a stepped, anchored walkthrough that highlights UI and explains
// what happens *behind the dungeon* — the two chains, the cross-chain messages, and
// saya's settlement. It deliberately ignores game strategy and focuses on the
// appchain mechanics.
import { useEffect, useLayoutEffect, useState, type ReactNode } from "react";
import * as chain from "./chain.ts";

type Tab = "dungeon" | "bank" | "leaderboard";
type Step = { tab?: Tab; target?: string; side?: "left" | "right"; title: string; body: ReactNode };

const L1 = chain.SETTLEMENT_NAME; // "Starknet Sepolia" | "Starknet Mainnet"

const STEPS: Step[] = [
  {
    title: "Behind the dungeon",
    body: (
      <>
        New to appchains? This walks through what happens <b>behind the stage</b> — the two chains, the
        messages between them, and the settlement that moves value. The dungeon is just an excuse; the
        point is the <b>cross-chain plumbing</b>. Use <b>next</b> / <b>back</b> (or ←/→), <b>skip</b> to exit.
      </>
    ),
  },
  {
    tab: "dungeon",
    target: "tabs",
    title: "Two chains",
    body: (
      <>
        You <b>play on the DUNGEON appchain (L2)</b> — its own Katana: cheap, instant, disposable. You
        <b> own on {L1} (L1)</b> — the durable, public record. Every step below is one of these chains
        talking to the other through the <b>piltover</b> mailbox.
      </>
    ),
  },
  {
    tab: "dungeon",
    target: "play",
    title: "Enter · L1 → L2",
    body: (
      <>
        Entering is <b>free</b> — a one-click L1 tx (it dev-mints the entry credit for you) that calls
        piltover's <b>send_message_to_appchain</b>. The appchain's <b>messaging service</b> relays it into
        the <b>mint_run</b> <code>#[l1_handler]</code> — no prover needed — which starts your run on L2.
        That's the instant L1→L2 direction.
      </>
    ),
  },
  {
    tab: "dungeon",
    target: "log",
    side: "left",
    title: "Play · on L2",
    body: (
      <>
        Each action is <b>one appchain transaction</b> — instant and feeless, never touching L1. The
        <b> run-outcome log</b> here is the appchain's <b>event feed</b> (RunEnded, indexed by Torii): every
        run's death or extract, by <b>every player</b>; click any line for its L2 tx. This is why play lives
        on an appchain: high-frequency, throwaway state.
      </>
    ),
  },
  {
    tab: "dungeon",
    target: "play",
    title: "Extract · into the L2 vault",
    body: (
      <>
        Leaving alive banks the run's gold into your <b>vault on L2</b> — still provisional, nothing has
        crossed to L1 yet. Death forfeits the in-progress haul. Value on the appchain isn't real until
        it's committed to the settlement layer.
      </>
    ),
  },
  {
    tab: "bank",
    target: "bank",
    title: "Bank · L2 → L1, settled",
    body: (
      <>
        <b>Withdraw</b> sends one L2→L1 message (<b>send_message_to_l1</b>) carrying the whole vault. Then
        <b> saya</b> proves the appchain block and submits <b>update_state</b> to piltover on L1. Only once
        that block <b>settles</b> can L1 consume the message and <b>mint $GOLD</b> — which the app then does
        automatically. The settled-vs-tip gap is the whole lesson.
      </>
    ),
  },
  {
    tab: "leaderboard",
    target: "leaderboard",
    title: "Leaderboard · on L2",
    body: (
      <>
        The leaderboard is kept <b>on the appchain itself</b> (best run score per player, read straight
        from its Torii). L1 only holds the minted $GOLD. Worlds can live on whichever chain fits — here,
        scores on L2, money on L1.
      </>
    ),
  },
  {
    tab: "dungeon",
    target: "windows",
    title: "Look behind the curtain",
    body: (
      <>
        <b>config</b> lists every service URL + contract address for the network and shows saya's
        <b> settled vs tip</b>. <b>logs</b> streams the appchain, saya, and Torii output live — watch the
        L1-handler relays and <b>update_state</b> calls as they happen. <b>txns</b> logs every transaction
        the client signs, tagged <b>L1</b> or <b>L2</b>, each linking to its explorer. That's the appchain,
        end to end.
      </>
    ),
  },
];

export function Tutorial({ onClose, setTab }: { onClose: () => void; setTab: (t: Tab) => void }) {
  const [i, setI] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const step = STEPS[i];
  const last = i >= STEPS.length - 1;
  const next = () => (last ? onClose() : setI((p) => p + 1));
  const back = () => setI((p) => Math.max(0, p - 1));

  // Switch to the tab a step needs so its target is on screen.
  useEffect(() => {
    if (step.tab) setTab(step.tab);
  }, [i, step.tab, setTab]);

  // Measure the spotlight target after layout settles (tab switch / reflow / resize).
  useLayoutEffect(() => {
    let raf = 0;
    const measure = () => {
      if (!step.target) return setRect(null);
      const el = document.querySelector(`[data-tut="${step.target}"]`);
      setRect(el ? el.getBoundingClientRect() : null);
    };
    raf = requestAnimationFrame(() => requestAnimationFrame(measure));
    window.addEventListener("resize", measure);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", measure);
    };
  }, [i, step.target]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowRight" || e.key === "Enter") next();
      else if (e.key === "ArrowLeft") back();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Position the card: beside the spotlight if there's room, else centered.
  const CARD_W = 360;
  const pad = 14;
  let cardStyle: React.CSSProperties = {
    left: "50%",
    top: "50%",
    transform: "translate(-50%, -50%)",
  };
  if (rect) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const top = Math.min(Math.max(rect.top, 16), vh - 260); // align with the target, clamped
    if (step.side === "left") {
      cardStyle = { left: Math.max(16, rect.left - pad - CARD_W), top };
    } else if (step.side === "right") {
      cardStyle = { left: Math.min(rect.right + pad, vw - CARD_W - 16), top };
    } else {
      const left = Math.min(Math.max(rect.left, 16), vw - CARD_W - 16);
      const below = vh - rect.bottom > 240;
      cardStyle = below
        ? { left, top: rect.bottom + pad }
        : rect.top > 240
          ? { left, bottom: vh - rect.top + pad }
          : { left, top: Math.min(rect.bottom + pad, vh - 240) };
    }
  }

  return (
    <div className="tut">
      {rect ? (
        <div
          className="tut-hole"
          style={{ left: rect.left - 6, top: rect.top - 6, width: rect.width + 12, height: rect.height + 12 }}
        />
      ) : (
        <div className="tut-dim" />
      )}
      <div className="tut-card" style={cardStyle}>
        <div className="tut-step">
          tutorial · step {i + 1}/{STEPS.length}
        </div>
        <div className="tut-title">{step.title}</div>
        <div className="tut-body">{step.body}</div>
        <div className="tut-actions">
          <button className="tut-skip" onClick={onClose}>
            skip
          </button>
          <span className="spacer" />
          {i > 0 && <button onClick={back}>back</button>}
          <button className="good" onClick={next}>
            {last ? "done" : "next →"}
          </button>
        </div>
      </div>
    </div>
  );
}
