import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Check, ExternalLink, Gamepad2, Loader2, Trophy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import {
  readGameState,
  readScoreState,
  getMintTxHashes,
  purchaseGame,
  syncScore,
  claimScore,
  settledBlock,
  appchainBlock,
  shortHex,
  explorerTxUrl,
  type GameState,
  type ScoreState,
  SETTLEMENT_RPC,
  APPCHAIN_RPC,
  SETTLEMENT_EXPLORER,
  APPCHAIN_EXPLORER,
  PILTOVER,
  SCORE_REGISTRY,
  GAME_MINTER,
  ACHIEVEMENTS,
  BUYER_ADDRESS,
  PLAYER_ADDRESS,
} from "./chain.ts";

const CATALOG = ["Cosmic Drift", "Neon Samurai", "Dungeon of Felt", "Starkfall", "Pixel Raiders", "Cairo Quest"];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function waitUntil(pred: () => Promise<boolean>, timeoutMs: number) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await pred()) return true;
    await sleep(1000);
  }
  return false;
}

type Purchase = { id: number; game: string; gameId: number; l1TxHash?: string; error?: string };
type PurchaseStage = "sending" | "relaying" | "minted";

type SyncStage = "emitting" | "settling" | "claiming" | "claimed" | "error";
type Sync = {
  id: number;
  score: number;
  l2TxHash?: string;
  emitBlock?: number;
  claimTxHash?: string;
  stage: SyncStage;
  error?: string;
};

export default function App() {
  const [online, setOnline] = useState(false);
  const [game, setGame] = useState<GameState | null>(null);
  const [score, setScore] = useState<ScoreState | null>(null);
  const [settled, setSettled] = useState<number>(-1);
  const [tip, setTip] = useState<number>(0);

  const [purchases, setPurchases] = useState<Purchase[]>([]);
  const [mintTxs, setMintTxs] = useState<string[]>([]);
  const mintBaseline = useRef<number | null>(null);
  const [buying, setBuying] = useState(false);
  const nextPurchase = useRef(1);

  const [syncs, setSyncs] = useState<Sync[]>([]);
  const nextSync = useRef(1);
  const syncing = syncs.some((s) => s.stage !== "claimed" && s.stage !== "error");

  useEffect(() => {
    let active = true;
    const tick = async () => {
      try {
        const [g, sc, txs, sb, tp] = await Promise.all([
          readGameState(),
          readScoreState(),
          getMintTxHashes(),
          settledBlock(),
          appchainBlock(),
        ]);
        if (!active) return;
        if (mintBaseline.current === null) mintBaseline.current = txs.length;
        setGame(g);
        setScore(sc);
        setMintTxs(txs);
        setSettled(sb);
        setTip(tp);
        setOnline(true);
      } catch {
        if (active) setOnline(false);
      }
    };
    tick();
    const h = setInterval(tick, 1000);
    return () => {
      active = false;
      clearInterval(h);
    };
  }, []);

  // --- L1 -> L2 purchase ---
  const mintBase = mintBaseline.current ?? 0;
  const confirmed = Math.max(0, mintTxs.length - mintBase);
  const purchaseStage = (i: number, p: Purchase): PurchaseStage =>
    i < confirmed ? "minted" : p.l1TxHash ? "relaying" : "sending";
  const l2HashFor = (i: number) => (i < confirmed ? mintTxs[mintBase + i] : undefined);
  const pendingPurchases = purchases.length - confirmed;

  async function onPurchase() {
    const game = CATALOG[(nextPurchase.current - 1) % CATALOG.length];
    const id = nextPurchase.current;
    nextPurchase.current += 1;
    setPurchases((p) => [...p, { id, game, gameId: id }]);
    setBuying(true);
    try {
      const tx = await purchaseGame(id);
      setPurchases((p) => p.map((x) => (x.id === id ? { ...x, l1TxHash: tx } : x)));
    } catch (err) {
      setPurchases((p) =>
        p.map((x) => (x.id === id ? { ...x, error: err instanceof Error ? err.message : String(err) } : x)),
      );
    } finally {
      setBuying(false);
    }
  }

  // --- L2 -> L1 score sync ---
  const updateSync = (id: number, patch: Partial<Sync>) =>
    setSyncs((s) => s.map((x) => (x.id === id ? { ...x, ...patch } : x)));

  async function onSync() {
    const id = nextSync.current;
    nextSync.current += 1;
    const value = 1000 + Math.floor(Math.random() * 9000);
    setSyncs((s) => [...s, { id, score: value, stage: "emitting" }]);
    try {
      const { txHash, block } = await syncScore(value);
      updateSync(id, { l2TxHash: txHash, emitBlock: block, stage: "settling" });

      const ok = await waitUntil(async () => (await settledBlock()) >= block, 180_000);
      if (!ok) throw new Error("timed out waiting for saya to settle the block");
      updateSync(id, { stage: "claiming" });

      // The message is registered by the settled state update; claim may need a
      // retry if the settlement tx is still being indexed.
      let claimTx = "";
      for (let attempt = 0; attempt < 5 && !claimTx; attempt++) {
        try {
          claimTx = await claimScore(PLAYER_ADDRESS, value);
        } catch (e) {
          if (attempt === 4) throw e;
          await sleep(1500);
        }
      }
      updateSync(id, { claimTxHash: claimTx, stage: "claimed" });
    } catch (err) {
      updateSync(id, { stage: "error", error: err instanceof Error ? err.message : String(err) });
    }
  }

  const sayaCaughtUp = settled >= tip;

  return (
    <div className="min-h-screen bg-background bg-[radial-gradient(1200px_600px_at_80%_-10%,oklch(0.3_0.08_280/0.25),transparent_55%)] text-foreground">
      <div className="mx-auto max-w-6xl px-5 py-8 pb-16">
        <header className="mb-6 flex flex-wrap items-start justify-between gap-4">
          <div className="flex items-center gap-4">
            <div className="grid size-12 shrink-0 place-items-center rounded-xl bg-primary/15 text-primary">
              <Gamepad2 className="size-6" />
            </div>
            <div>
              <h1 className="text-2xl font-semibold tracking-tight">Cross-Chain Game Store</h1>
              <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
                Two-way Katana messaging: buy a game (L1&nbsp;→&nbsp;L2) and sync your score
                (L2&nbsp;→&nbsp;L1, settled by <b className="text-foreground">saya</b>).
              </p>
            </div>
          </div>
          <div className="flex flex-col items-end gap-2">
            <Badge variant="outline" className="gap-1.5 py-1">
              <span className={cn("size-2 rounded-full", online ? "bg-green-500" : "bg-amber-500")} />
              {online ? "Connected" : "Connecting…"}
            </Badge>
            <SayaIndicator settled={settled} tip={tip} caughtUp={sayaCaughtUp} online={online} />
          </div>
        </header>

        <section className="mb-6 grid items-stretch gap-3 md:grid-cols-[1fr_auto_1fr]">
          <ChainCard
            tag="Settlement · “L1”"
            rpc={SETTLEMENT_RPC}
            explorer={SETTLEMENT_EXPLORER}
            contracts={[
              { label: "piltover core", addr: PILTOVER },
              { label: "score_registry", addr: SCORE_REGISTRY },
            ]}
          />
          <div className="flex flex-col items-center justify-center gap-2 px-2 text-center text-muted-foreground">
            <span className="font-mono text-[11px] text-primary">send_message_to_appchain</span>
            <ArrowRight className="size-5 text-primary" />
            <ArrowLeft className="size-5 text-green-500" />
            <span className="font-mono text-[11px] text-green-500">send_message_to_l1 + saya</span>
          </div>
          <ChainCard
            tag="Appchain · “L2”"
            rpc={APPCHAIN_RPC}
            explorer={APPCHAIN_EXPLORER}
            contracts={[
              { label: "game_minter", addr: GAME_MINTER },
              { label: "achievements", addr: ACHIEVEMENTS },
            ]}
          />
        </section>

        <section className="grid gap-4 lg:grid-cols-2">
          {/* L1 -> L2 */}
          <Card className="py-5">
            <CardContent className="px-5">
              <FlowHeader tone="l1" title="Buy a game" subtitle="L1 → L2 message" />
              <div className="mb-4 grid grid-cols-2 gap-3">
                <Stat label="Total minted" value={game ? game.totalMinted : "…"} highlight tone="l1" />
                <Stat label="Your games" value={game ? game.mintedByYou : "…"} />
              </div>
              <Button className="w-full" onClick={onPurchase} disabled={buying || !online}>
                {buying ? <Loader2 className="size-4 animate-spin" /> : <Gamepad2 className="size-4" />}
                {buying ? "Submitting on L1…" : `Purchase “${CATALOG[(nextPurchase.current - 1) % CATALOG.length]}”`}
              </Button>
              <p className="mt-2 text-center text-xs text-muted-foreground">
                buyer <code className="font-mono">{shortHex(BUYER_ADDRESS)}</code>
                {pendingPurchases > 0 && ` · ${pendingPurchases} awaiting relay`}
              </p>
              <Feed empty={purchases.length === 0} emptyText="No purchases yet.">
                {purchases
                  .map((p, i) => ({ p, stage: purchaseStage(i, p), l2: l2HashFor(i) }))
                  .reverse()
                  .map(({ p, stage, l2 }) => (
                    <PurchaseRow key={p.id} purchase={p} stage={stage} l2TxHash={l2} />
                  ))}
              </Feed>
            </CardContent>
          </Card>

          {/* L2 -> L1 */}
          <Card className="py-5">
            <CardContent className="px-5">
              <FlowHeader tone="l2" title="Sync your score" subtitle="L2 → L1 message · settled by saya" />
              <div className="mb-4 grid grid-cols-2 gap-3">
                <Stat label="Last synced score" value={score ? score.lastScore : "…"} highlight tone="l2" />
                <Stat label="Total syncs" value={score ? score.totalSynced : "…"} />
              </div>
              <Button
                className="w-full bg-green-600 text-white hover:bg-green-600/90"
                onClick={onSync}
                disabled={syncing || !online}
              >
                {syncing ? <Loader2 className="size-4 animate-spin" /> : <Trophy className="size-4" />}
                {syncing ? "Syncing…" : "Sync a random score to L1"}
              </Button>
              <p className="mt-2 text-center text-xs text-muted-foreground">
                player <code className="font-mono">{shortHex(PLAYER_ADDRESS)}</code>
                {score && score.yourScore > 0 && ` · your score on L1: ${score.yourScore}`}
              </p>
              <Feed empty={syncs.length === 0} emptyText="No syncs yet.">
                {[...syncs].reverse().map((s) => (
                  <SyncRow key={s.id} sync={s} settled={settled} />
                ))}
              </Feed>
            </CardContent>
          </Card>
        </section>
      </div>
    </div>
  );
}

function SayaIndicator(props: { settled: number; tip: number; caughtUp: boolean; online: boolean }) {
  if (!props.online) return null;
  return (
    <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
      {props.caughtUp ? (
        <Check className="size-3.5 text-green-500" />
      ) : (
        <Loader2 className="size-3.5 animate-spin text-primary" />
      )}
      saya: settled block <span className="font-mono text-foreground">{Math.max(props.settled, 0)}</span> / appchain tip{" "}
      <span className="font-mono text-foreground">{props.tip}</span>
    </div>
  );
}

function ChainCard(props: {
  tag: string;
  rpc: string;
  explorer: string;
  contracts: { label: string; addr: string }[];
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
        <dl className="grid gap-0.5 text-sm">
          <dt className="text-[11px] tracking-wide text-muted-foreground uppercase">RPC</dt>
          <dd className="font-mono text-[13px] break-all">{props.rpc}</dd>
          {props.contracts.map((c) => (
            <div key={c.label}>
              <dt className="mt-2 text-[11px] tracking-wide text-muted-foreground uppercase">{c.label}</dt>
              <dd className="font-mono text-[13px] break-all">{shortHex(c.addr, 10, 6)}</dd>
            </div>
          ))}
        </dl>
      </CardContent>
    </Card>
  );
}

function FlowHeader(props: { tone: "l1" | "l2"; title: string; subtitle: string }) {
  return (
    <div className="mb-4 flex items-center gap-2">
      <Badge className={cn(props.tone === "l2" && "bg-green-600 text-white")}>{props.subtitle}</Badge>
      <h2 className="text-base font-semibold">{props.title}</h2>
    </div>
  );
}

function Stat(props: { label: string; value: React.ReactNode; highlight?: boolean; tone?: "l1" | "l2" }) {
  return (
    <div
      className={cn(
        "rounded-lg border p-3 text-center",
        props.highlight && props.tone === "l1" && "border-primary/40 bg-primary/10",
        props.highlight && props.tone === "l2" && "border-green-600/40 bg-green-600/10",
      )}
    >
      <div
        className={cn(
          "text-2xl font-bold tracking-tight tabular-nums",
          props.highlight && props.tone === "l1" && "text-primary",
          props.highlight && props.tone === "l2" && "text-green-500",
        )}
      >
        {props.value}
      </div>
      <div className="mt-0.5 text-xs text-muted-foreground">{props.label}</div>
    </div>
  );
}

function Feed(props: { empty: boolean; emptyText: string; children: React.ReactNode }) {
  return (
    <div className="mt-4">
      {props.empty ? (
        <p className="text-sm text-muted-foreground">{props.emptyText}</p>
      ) : (
        <div className="flex flex-col gap-2">{props.children}</div>
      )}
    </div>
  );
}

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

function Step(props: { done: boolean; active: boolean; label: string }) {
  return (
    <div className={cn("flex flex-1 items-center gap-1.5 text-xs", props.done || props.active ? "text-foreground" : "text-muted-foreground")}>
      <span
        className={cn(
          "grid size-5 shrink-0 place-items-center rounded-full border text-[10px]",
          props.done && "border-green-500 bg-green-500 text-white",
          props.active && "border-primary text-primary",
        )}
      >
        {props.done ? <Check className="size-3" /> : props.active ? <Loader2 className="size-3 animate-spin" /> : null}
      </span>
      {props.label}
    </div>
  );
}

function PurchaseRow({ purchase, stage, l2TxHash }: { purchase: Purchase; stage: PurchaseStage; l2TxHash?: string }) {
  const order: PurchaseStage[] = ["sending", "relaying", "minted"];
  const current = order.indexOf(stage);
  const steps = ["L1 message sent", "Katana relaying", "Minted on L2"];
  return (
    <Card className={cn("gap-0 border-l-4 py-3", stage === "minted" ? "border-l-green-500" : "border-l-amber-500")}>
      <CardContent className="px-3">
        <div className="mb-2.5 flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold">{purchase.game}</span>
          <span className="font-mono text-xs text-muted-foreground">#{purchase.gameId}</span>
          <div className="ml-auto flex items-center gap-3">
            {purchase.l1TxHash && (
              <TxLink tone="l1" label="L1 tx" hash={purchase.l1TxHash} href={explorerTxUrl(SETTLEMENT_EXPLORER, purchase.l1TxHash)} />
            )}
            {l2TxHash && <TxLink tone="l2" label="L2 tx" hash={l2TxHash} href={explorerTxUrl(APPCHAIN_EXPLORER, l2TxHash)} />}
          </div>
        </div>
        {purchase.error ? (
          <div className="font-mono text-xs text-destructive">⚠ {purchase.error}</div>
        ) : (
          <div className="flex gap-2">
            {steps.map((label, idx) => (
              <Step key={label} label={label} done={idx < current || stage === "minted"} active={idx === current && stage !== "minted"} />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function SyncRow({ sync, settled }: { sync: Sync; settled: number }) {
  const done = sync.stage === "claimed";
  const settlingLabel =
    sync.stage === "settling" && sync.emitBlock !== undefined
      ? `saya settling (block ${Math.max(settled, 0)}/${sync.emitBlock})`
      : "Settled by saya";
  const steps = ["Emitted on L2", settlingLabel, "Claimed on L1"];
  // map 4-stage model onto 3 visible steps: emitting->0, settling->1, claiming/claimed->2
  const visibleCurrent = sync.stage === "emitting" ? 0 : sync.stage === "settling" ? 1 : 2;
  return (
    <Card className={cn("gap-0 border-l-4 py-3", done ? "border-l-green-500" : sync.stage === "error" ? "border-l-destructive" : "border-l-amber-500")}>
      <CardContent className="px-3">
        <div className="mb-2.5 flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold">Score {sync.score}</span>
          <div className="ml-auto flex items-center gap-3">
            {sync.l2TxHash && <TxLink tone="l2" label="L2 tx" hash={sync.l2TxHash} href={explorerTxUrl(APPCHAIN_EXPLORER, sync.l2TxHash)} />}
            {sync.claimTxHash && <TxLink tone="l1" label="L1 tx" hash={sync.claimTxHash} href={explorerTxUrl(SETTLEMENT_EXPLORER, sync.claimTxHash)} />}
          </div>
        </div>
        {sync.stage === "error" ? (
          <div className="font-mono text-xs text-destructive">⚠ {sync.error}</div>
        ) : (
          <div className="flex gap-2">
            {steps.map((label, idx) => (
              <Step key={idx} label={label} done={idx < visibleCurrent || done} active={idx === visibleCurrent && !done} />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}
