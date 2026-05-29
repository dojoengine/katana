import { useEffect, useRef, useState } from "react";
import { ArrowRight, Check, ExternalLink, Gamepad2, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import {
  readAppchainState,
  getMintTxHashes,
  purchaseGame,
  shortHex,
  explorerTxUrl,
  type AppchainState,
  SETTLEMENT_RPC,
  APPCHAIN_RPC,
  MESSAGING_CONTRACT,
  GAME_CONTRACT,
  BUYER_ADDRESS,
  SETTLEMENT_EXPLORER,
  APPCHAIN_EXPLORER,
} from "./chain.ts";

const CATALOG = [
  "Cosmic Drift",
  "Neon Samurai",
  "Dungeon of Felt",
  "Starkfall",
  "Pixel Raiders",
  "Cairo Quest",
];

type Purchase = {
  id: number;
  game: string;
  gameId: number;
  l1TxHash?: string;
  error?: string;
};

type Stage = "sending" | "relaying" | "minted";

export default function App() {
  const [state, setState] = useState<AppchainState | null>(null);
  const [online, setOnline] = useState(false);
  const [purchases, setPurchases] = useState<Purchase[]>([]);
  const [mintTxs, setMintTxs] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const nextId = useRef(1);
  // Mint events that existed before this session, so purchases map 1:1 onto the
  // events they produce (messages relay in order).
  const eventsBaseline = useRef<number | null>(null);

  // Poll the appchain — this is how the UI "reacts" to L2 contract state mutated
  // by the relayed L1 -> L2 message. We read the aggregate counters and the
  // per-mint L2 tx hashes (to deep-link each into the appchain explorer).
  useEffect(() => {
    let active = true;
    const tick = async () => {
      try {
        const [s, txs] = await Promise.all([readAppchainState(), getMintTxHashes()]);
        if (!active) return;
        if (eventsBaseline.current === null) eventsBaseline.current = txs.length;
        setState(s);
        setMintTxs(txs);
        setOnline(true);
      } catch {
        if (active) setOnline(false);
      }
    };
    tick();
    const handle = setInterval(tick, 1000);
    return () => {
      active = false;
      clearInterval(handle);
    };
  }, []);

  const base = eventsBaseline.current ?? 0;
  const confirmedCount = Math.max(0, mintTxs.length - base);
  const stageOf = (index: number, p: Purchase): Stage =>
    index < confirmedCount ? "minted" : p.l1TxHash ? "relaying" : "sending";
  const l2HashFor = (index: number) => (index < confirmedCount ? mintTxs[base + index] : undefined);

  async function onPurchase() {
    const game = CATALOG[(nextId.current - 1) % CATALOG.length];
    const id = nextId.current;
    nextId.current += 1;
    setPurchases((prev) => [...prev, { id, game, gameId: id }]);
    setBusy(true);
    try {
      const txHash = await purchaseGame(id);
      setPurchases((prev) => prev.map((p) => (p.id === id ? { ...p, l1TxHash: txHash } : p)));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setPurchases((prev) => prev.map((p) => (p.id === id ? { ...p, error: msg } : p)));
    } finally {
      setBusy(false);
    }
  }

  const pendingCount = purchases.length - confirmedCount;
  const nextGame = CATALOG[(nextId.current - 1) % CATALOG.length];

  return (
    <div className="min-h-screen bg-background bg-[radial-gradient(1200px_600px_at_80%_-10%,oklch(0.3_0.08_280/0.25),transparent_55%)] text-foreground">
      <div className="mx-auto max-w-5xl px-5 py-8 pb-16">
        {/* Hero */}
        <header className="mb-7 flex items-start justify-between gap-4">
          <div className="flex items-center gap-4">
            <div className="grid size-12 shrink-0 place-items-center rounded-xl bg-primary/15 text-primary">
              <Gamepad2 className="size-6" />
            </div>
            <div>
              <h1 className="text-2xl font-semibold tracking-tight">Cross-Chain Game Store</h1>
              <p className="mt-1 max-w-xl text-sm text-muted-foreground">
                Purchase on the <b className="text-foreground">settlement layer</b>, mint on the{" "}
                <b className="text-foreground">appchain</b> — powered by Katana L1&nbsp;→&nbsp;L2
                messaging.
              </p>
            </div>
          </div>
          <Badge variant="outline" className="gap-1.5 py-1">
            <span
              className={cn(
                "size-2 rounded-full",
                online ? "bg-green-500 shadow-[0_0_8px_var(--color-green-500)]" : "bg-amber-500",
              )}
            />
            {online ? "Appchain connected" : "Connecting…"}
          </Badge>
        </header>

        {/* Chains */}
        <section className="mb-6 grid items-stretch gap-3 md:grid-cols-[1fr_auto_1fr]">
          <ChainCard
            role="Settlement layer · “L1”"
            tag="Where you act"
            rpc={SETTLEMENT_RPC}
            contractLabel="Messaging contract"
            contract={MESSAGING_CONTRACT}
            explorer={SETTLEMENT_EXPLORER}
          />
          <div className="flex flex-col items-center justify-center gap-1.5 px-2 text-center text-muted-foreground max-md:flex-row">
            <span className="font-mono text-[11px] text-primary">send_message_to_appchain</span>
            <ArrowRight className="size-6 text-primary max-md:rotate-90" />
            <small className="text-[11px]">Katana relays as an L1 handler tx</small>
          </div>
          <ChainCard
            role="Appchain · “L2”"
            tag="Where state updates"
            rpc={APPCHAIN_RPC}
            contractLabel="game_minter contract"
            contract={GAME_CONTRACT}
            explorer={APPCHAIN_EXPLORER}
          />
        </section>

        {/* Stats */}
        <section className="mb-6 grid grid-cols-2 gap-3 md:grid-cols-4">
          <Stat label="Total games minted" value={state ? state.totalMinted : "…"} highlight />
          <Stat label="Your games" value={state ? state.mintedByYou : "…"} />
          <Stat label="Last buyer" value={state ? shortHex(state.lastBuyer) : "…"} mono />
          <Stat label="Awaiting relay" value={pendingCount} />
        </section>

        {/* Action */}
        <section className="mb-8 text-center">
          <Button size="lg" className="h-12 px-8 text-base" onClick={onPurchase} disabled={busy || !online}>
            {busy ? (
              <>
                <Loader2 className="size-4 animate-spin" /> Submitting on L1…
              </>
            ) : (
              <>
                <Gamepad2 className="size-4" /> Purchase “{nextGame}”
              </>
            )}
          </Button>
          <p className="mt-3 text-sm text-muted-foreground">
            Buyer{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
              {shortHex(BUYER_ADDRESS)}
            </code>{" "}
            · each purchase sends one L1&nbsp;→&nbsp;L2 message
          </p>
        </section>

        {/* Feed */}
        <section>
          <h2 className="mb-3 text-base font-semibold">Purchases</h2>
          {purchases.length === 0 && (
            <p className="text-sm text-muted-foreground">No purchases yet. Buy a game above.</p>
          )}
          <div className="flex flex-col gap-2.5">
            {purchases
              .map((p, i) => ({ p, stage: stageOf(i, p), l2: l2HashFor(i) }))
              .reverse()
              .map(({ p, stage, l2 }) => (
                <PurchaseRow key={p.id} purchase={p} stage={stage} l2TxHash={l2} />
              ))}
          </div>
        </section>
      </div>
    </div>
  );
}

function ChainCard(props: {
  role: string;
  tag: string;
  rpc: string;
  contractLabel: string;
  contract: string;
  explorer: string;
}) {
  return (
    <Card className="gap-0 py-4">
      <CardContent className="px-4">
        <div className="mb-2 flex items-center justify-between">
          <Badge variant="secondary" className="text-[10px] tracking-wide uppercase">
            {props.tag}
          </Badge>
          <a
            href={props.explorer}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-[11px] text-muted-foreground transition-colors hover:border-primary hover:text-foreground"
          >
            Explorer <ExternalLink className="size-3" />
          </a>
        </div>
        <h3 className="mb-3 text-[15px] font-semibold">{props.role}</h3>
        <dl className="grid gap-0.5 text-sm">
          <dt className="mt-1 text-[11px] tracking-wide text-muted-foreground uppercase">RPC</dt>
          <dd className="font-mono break-all">{props.rpc}</dd>
          <dt className="mt-2 text-[11px] tracking-wide text-muted-foreground uppercase">
            {props.contractLabel}
          </dt>
          <dd className="font-mono break-all">{shortHex(props.contract, 10, 6)}</dd>
        </dl>
      </CardContent>
    </Card>
  );
}

function Stat(props: { label: string; value: React.ReactNode; highlight?: boolean; mono?: boolean }) {
  return (
    <Card
      className={cn(
        "py-5 text-center",
        props.highlight && "border-primary/40 bg-gradient-to-b from-primary/10 to-transparent",
      )}
    >
      <CardContent className="px-4">
        <div
          className={cn(
            "text-3xl font-bold tracking-tight tabular-nums",
            props.highlight && "text-primary",
            props.mono && "font-mono text-2xl",
          )}
        >
          {props.value}
        </div>
        <div className="mt-1 text-xs text-muted-foreground">{props.label}</div>
      </CardContent>
    </Card>
  );
}

const STEPS: { key: Stage; label: string }[] = [
  { key: "sending", label: "L1 message sent" },
  { key: "relaying", label: "Katana relaying" },
  { key: "minted", label: "Minted on L2" },
];

function TxLink(props: { label: string; href: string; hash: string; tone: "l1" | "l2" }) {
  return (
    <a
      href={props.href}
      target="_blank"
      rel="noreferrer"
      title={props.hash}
      className={cn(
        "inline-flex items-center gap-1 font-mono text-xs hover:underline",
        props.tone === "l1" ? "text-primary" : "text-green-500",
      )}
    >
      {props.label} {shortHex(props.hash)} <ExternalLink className="size-3" />
    </a>
  );
}

function PurchaseRow({
  purchase,
  stage,
  l2TxHash,
}: {
  purchase: Purchase;
  stage: Stage;
  l2TxHash?: string;
}) {
  const order: Stage[] = ["sending", "relaying", "minted"];
  const current = order.indexOf(stage);
  return (
    <Card
      className={cn(
        "gap-0 border-l-4 py-3.5",
        stage === "minted" ? "border-l-green-500" : "border-l-amber-500",
      )}
    >
      <CardContent className="px-4">
        <div className="mb-3 flex flex-wrap items-center gap-2.5">
          <span className="font-semibold">{purchase.game}</span>
          <span className="font-mono text-xs text-muted-foreground">#{purchase.gameId}</span>
          <div className="ml-auto flex items-center gap-3">
            {purchase.l1TxHash && (
              <TxLink
                tone="l1"
                label="L1 tx"
                hash={purchase.l1TxHash}
                href={explorerTxUrl(SETTLEMENT_EXPLORER, purchase.l1TxHash)}
              />
            )}
            {l2TxHash && (
              <TxLink
                tone="l2"
                label="L2 tx"
                hash={l2TxHash}
                href={explorerTxUrl(APPCHAIN_EXPLORER, l2TxHash)}
              />
            )}
          </div>
        </div>
        {purchase.error ? (
          <div className="font-mono text-sm text-destructive">⚠ {purchase.error}</div>
        ) : (
          <div className="flex gap-2">
            {STEPS.map((s, idx) => {
              const done = idx < current || stage === "minted";
              const active = idx === current && stage !== "minted";
              return (
                <div
                  key={s.key}
                  className={cn(
                    "flex flex-1 items-center gap-1.5 text-xs",
                    done || active ? "text-foreground" : "text-muted-foreground",
                  )}
                >
                  <span
                    className={cn(
                      "grid size-5 shrink-0 place-items-center rounded-full border text-[10px]",
                      done && "border-green-500 bg-green-500 text-white",
                      active && "border-primary text-primary",
                    )}
                  >
                    {done ? (
                      <Check className="size-3" />
                    ) : active ? (
                      <Loader2 className="size-3 animate-spin" />
                    ) : null}
                  </span>
                  {s.label}
                </div>
              );
            })}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
