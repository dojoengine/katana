import { Fragment, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Coins,
  Dices,
  ExternalLink,
  Info,
  Loader2,
  Settings,
  Trophy,
  Vault,
  Wrench,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { AnimatePresence, motion } from "framer-motion";
import { cn } from "@/lib/utils";
import {
  readGameState,
  readScoreState,
  getPurchaseHistory,
  getPlayHistory,
  purchaseGame,
  playGame,
  claimScore,
  settledBlock,
  appchainBlock,
  shortHex,
  explorerTxUrl,
  explorerAddrUrl,
  type GameState,
  type ScoreState,
  type PurchaseRecord,
  type PlayRecord,
  SETTLEMENT_RPC,
  APPCHAIN_RPC,
  SETTLEMENT_EXPLORER,
  APPCHAIN_EXPLORER,
  PILTOVER,
  SCORE_REGISTRY,
  SCORE_WORLD,
  GAME,
  GAME_WORLD,
  TORII_SCORE,
  TORII_GAME,
  PLAYER_ADDRESS,
} from "./chain.ts";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export default function App() {
  const [online, setOnline] = useState(false);
  const [game, setGame] = useState<GameState | null>(null);
  const [, setScore] = useState<ScoreState | null>(null);
  const [settled, setSettled] = useState(-1);
  const [tip, setTip] = useState(0);
  const [purchases, setPurchases] = useState<PurchaseRecord[]>([]);
  const [plays, setPlays] = useState<PlayRecord[]>([]);

  const [buying, setBuying] = useState(false);
  const [rolling, setRolling] = useState(false);
  const [rollDisplay, setRollDisplay] = useState<number | null>(null);
  const [settling, setSettling] = useState<Set<number>>(new Set());
  const [introOpen, setIntroOpen] = useState(true);
  const [hoodOpen, setHoodOpen] = useState(false);
  const [detail, setDetail] = useState<PlayRecord | null>(null);
  const rollStart = useRef(0); // plays.length when the current roll started
  const rollToken = useRef(0); // invalidates a stale play_game() resolution

  useEffect(() => {
    let active = true;
    const tick = async () => {
      try {
        const [g, sc, ph, plh, sb, tp] = await Promise.all([
          readGameState(),
          readScoreState(),
          getPurchaseHistory(),
          getPlayHistory(),
          settledBlock(),
          appchainBlock(),
        ]);
        if (!active) return;
        setGame(g);
        setScore(sc);
        setPurchases(ph);
        setPlays(plh);
        setSettled(sb);
        setTip(tp);
        setOnline(true);
      } catch {
        if (active) setOnline(false);
      }
    };
    tick();
    const h = setInterval(tick, 1500);
    return () => {
      active = false;
      clearInterval(h);
    };
  }, []);

  // Cycle the die face while rolling.
  useEffect(() => {
    if (!rolling) return;
    const id = setInterval(() => setRollDisplay(1 + Math.floor(Math.random() * 100)), 60);
    return () => clearInterval(id);
  }, [rolling]);

  // Stop rolling as soon as the new roll lands in the feed (its card appears),
  // even if play_game()'s promise hasn't resolved yet.
  useEffect(() => {
    if (rolling && plays.length > rollStart.current) {
      rollToken.current += 1; // invalidate the in-flight play_game() resolution
      setRollDisplay(plays[plays.length - 1].score);
      setRolling(false);
    }
  }, [plays, rolling]);

  const available = game?.available ?? 0;
  const pendingMints = purchases.filter((p) => !p.mintTxHash).length;
  const coinLoading = buying || pendingMints > 0; // submitting on L1 or still minting on L2
  const unbanked = plays.filter((p) => !p.claimTxHash);
  const banked = plays.filter((p) => p.claimTxHash);
  const best = banked.reduce((m, p) => Math.max(m, p.score), 0);
  const totalPoints = banked.reduce((s, p) => s + p.score, 0);

  // Insert coin → buy a credit (L1 -> L2 mint).
  async function onBuy() {
    setBuying(true);
    try {
      await purchaseGame(purchases.length + 1);
    } catch (e) {
      console.error("buy failed", e);
    } finally {
      setBuying(false);
    }
  }

  // Roll the dice (play on L2). Cycle numbers for juice, then land on the real
  // on-chain roll read from the GamePlayed event.
  async function onRoll() {
    if (rolling || available < 1) return;
    const token = ++rollToken.current;
    rollStart.current = plays.length;
    setRolling(true);
    try {
      const { score } = await playGame();
      if (rollToken.current === token) {
        setRollDisplay(score);
        setRolling(false);
      }
    } catch (e) {
      if (rollToken.current === token) setRolling(false);
      console.error("roll failed", e);
    }
  }

  // Bank a roll to the Vault (settle L2 -> L1). Retries until saya has settled.
  async function onBank(seq: number, sc: number) {
    setSettling((s) => new Set(s).add(seq));
    try {
      for (let i = 0; i < 90; i++) {
        try {
          await claimScore(PLAYER_ADDRESS, sc);
          break;
        } catch {
          await sleep(2000);
        }
      }
    } finally {
      setSettling((s) => {
        const n = new Set(s);
        n.delete(seq);
        return n;
      });
    }
  }

  const sayaCaughtUp = settled >= tip;

  return (
    <TooltipProvider>
      <IntroDialog open={introOpen} onOpenChange={setIntroOpen} />
      <PlayDetailDialog play={detail} settled={settled} onOpenChange={(o) => !o && setDetail(null)} />
      <HoodDialog open={hoodOpen} onOpenChange={setHoodOpen} online={online} />

      <div className="flex h-screen min-h-[44rem] flex-col bg-background bg-[radial-gradient(1100px_560px_at_85%_-12%,oklch(0.72_0.13_285/0.16),transparent_58%)] text-foreground">
        <div className="mx-auto flex w-full max-w-5xl min-h-0 flex-1 flex-col px-5 py-7">
          {/* HUD */}
          <header className="mb-6 flex shrink-0 flex-wrap items-center justify-between gap-3">
            <div className="flex items-center gap-3">
              <div className="grid size-11 shrink-0 place-items-center rounded-xl bg-primary/15 text-primary">
                <Dices className="size-6" />
              </div>
              <div>
                <h1 className="text-xl font-bold tracking-tight">Cross-Chain Dice</h1>
                <p className="text-xs text-muted-foreground">Play on the appchain · bank your score to L1</p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              <Hud icon={<Coins className="size-3.5 text-primary" />} label="Credits" value={online ? available : "…"} />
              <Hud icon={<Trophy className="size-3.5 text-green-600" />} label="Best" value={online ? best : "…"} tone="green" />
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button variant="ghost" size="icon" className="size-10 rounded-full" onClick={() => setHoodOpen(true)} aria-label="Under the hood" />
                  }
                >
                  <Settings className="size-5 transition-transform duration-300 group-hover/button:rotate-90" />
                </TooltipTrigger>
                <TooltipContent>Under the hood</TooltipContent>
              </Tooltip>
            </div>
          </header>

          {/* Game area */}
          <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[1.35fr_1fr]">
            {/* Arcade (L2) */}
            <Card className="relative overflow-hidden">
              <CardContent className="flex min-h-0 flex-1 flex-col gap-5 p-5">
                <div className="flex items-center gap-2">
                  <span className="text-lg">🕹️</span>
                  <h2 className="font-semibold">Arcade</h2>
                  <Badge variant="secondary" className="text-[10px]">appchain · play free</Badge>
                  <InfoButton title="Buying & playing" className="ml-auto">
                    <BuyInfo />
                    <PlayInfo />
                  </InfoButton>
                </div>

                {/* Dice + roll */}
                <div className="flex flex-col items-center gap-4 py-2">
                  <Die value={rollDisplay} rolling={rolling} idle={!rolling && rollDisplay == null} />
                  <Button size="lg" className="h-12 w-48 text-base" onClick={onRoll} disabled={rolling || available < 1 || !online}>
                    {rolling ? <Loader2 className="size-5 animate-spin" /> : <Dices className="size-5" />}
                    {rolling ? "Rolling…" : available < 1 ? "No credits" : "Roll"}
                  </Button>
                  <div className="flex flex-wrap items-center justify-center gap-2 text-xs text-muted-foreground">
                    <span className="inline-flex items-center gap-1">
                      <Coins className="size-3.5" /> {available} credit{available === 1 ? "" : "s"}
                    </span>
                    <Button
                      variant="outline"
                      size="sm"
                      className="ml-1 h-7 gap-1 px-2 text-xs"
                      onClick={onBuy}
                      disabled={coinLoading || !online}
                    >
                      {coinLoading ? <Loader2 className="size-3.5 animate-spin" /> : <Coins className="size-3.5" />}
                      Insert coin
                    </Button>
                  </div>
                </div>

                {/* Unbanked rolls */}
                <div className="flex min-h-0 flex-1 flex-col">
                  <div className="mb-2 flex shrink-0 items-center justify-between text-xs text-muted-foreground">
                    <span>Rolls to bank</span>
                    {unbanked.length > 0 && <span>{unbanked.length} waiting</span>}
                  </div>
                  {unbanked.length === 0 ? (
                    <div className="flex flex-1 items-center justify-center rounded-lg border border-dashed p-4 text-center text-sm text-muted-foreground">
                      Roll to win points, then bank them to the Vault →
                    </div>
                  ) : (
                    <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto py-px pr-1">
                      <AnimatePresence initial={false}>
                        {[...unbanked].reverse().map((p) => (
                          <motion.div
                            key={p.seq}
                            layout
                            initial={{ opacity: 0, y: -10, scale: 0.97 }}
                            animate={{ opacity: 1, y: 0, scale: 1 }}
                            exit={{ opacity: 0, scale: 0.9 }}
                            transition={{ duration: 0.25, ease: "easeOut" }}
                          >
                            <UnbankedRoll
                              play={p}
                              settled={settled}
                              settling={settling.has(p.seq)}
                              onBank={() => onBank(p.seq, p.score)}
                              onInspect={() => setDetail(p)}
                            />
                          </motion.div>
                        ))}
                      </AnimatePresence>
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>

            {/* Vault (L1) */}
            <Card className="relative overflow-hidden border-green-600/30 bg-green-600/5">
              <CardContent className="flex min-h-0 flex-1 flex-col gap-5 p-5">
                <div className="flex items-center gap-2">
                  <Vault className="size-5 text-green-600" />
                  <h2 className="font-semibold">Vault</h2>
                  <Badge className="bg-green-600 text-[10px] text-white">L1 · permanent</Badge>
                  <InfoButton title="Banking to L1" className="ml-auto">
                    <PublishInfo />
                  </InfoButton>
                </div>

                <div className="text-center">
                  <div className="text-xs tracking-wide text-muted-foreground uppercase">Best banked score</div>
                  <div className="text-5xl font-bold tabular-nums text-green-600">{online ? best : "…"}</div>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <VaultStat label="Runs banked" value={online ? banked.length : "…"} />
                  <VaultStat label="Total points" value={online ? totalPoints : "…"} />
                </div>

                <div className="flex min-h-0 flex-1 flex-col">
                  <div className="mb-2 text-xs text-muted-foreground">Banked runs</div>
                  {banked.length === 0 ? (
                    <div className="flex flex-1 items-center justify-center rounded-lg border border-dashed p-4 text-center text-sm text-muted-foreground">
                      Nothing banked yet — settle a roll to lock it in on L1.
                    </div>
                  ) : (
                    <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto py-px pr-1">
                      <AnimatePresence initial={false}>
                        {[...banked].reverse().map((p) => (
                          <motion.div
                            key={p.seq}
                            layout
                            initial={{ opacity: 0, scale: 0.9 }}
                            animate={{ opacity: 1, scale: 1 }}
                            transition={{ duration: 0.3, ease: "easeOut" }}
                          >
                            <BankedRow play={p} best={best} onInspect={() => setDetail(p)} />
                          </motion.div>
                        ))}
                      </AnimatePresence>
                    </div>
                  )}
                </div>
              </CardContent>
            </Card>
          </div>

          {/* saya status */}
          <div className="mt-4 flex shrink-0 items-center justify-center">{online && <SayaGauge settled={settled} tip={tip} caughtUp={sayaCaughtUp} />}</div>
        </div>
      </div>
    </TooltipProvider>
  );
}

// --- HUD + game pieces ---

function Hud(props: { icon: React.ReactNode; label: string; value: React.ReactNode; tone?: "green" }) {
  return (
    <div className="flex h-10 items-center gap-2 rounded-full border bg-card px-4">
      {props.icon}
      <div className="flex items-baseline gap-1.5 leading-none">
        <span className="text-[11px] text-muted-foreground">{props.label}</span>
        <span className={cn("text-sm font-bold tabular-nums", props.tone === "green" && "text-green-600")}>{props.value}</span>
      </div>
    </div>
  );
}

function Die({ value, rolling, idle }: { value: number | null; rolling: boolean; idle: boolean }) {
  return (
    <motion.div
      animate={rolling ? { rotate: [0, -7, 7, -5, 5, 0] } : { rotate: 0, scale: [1.1, 1] }}
      transition={{ duration: rolling ? 0.5 : 0.3, repeat: rolling ? Infinity : 0, ease: "easeInOut" }}
      className={cn(
        "grid size-28 place-items-center rounded-3xl border-2 shadow-sm",
        rolling ? "border-primary/40 bg-primary/10" : "border-primary/30 bg-primary/5",
      )}
    >
      {idle ? <Dices className="size-12 text-primary/50" /> : <span className="text-5xl font-bold tabular-nums text-primary">{value ?? "—"}</span>}
    </motion.div>
  );
}

function UnbankedRoll({
  play,
  settled,
  settling,
  onBank,
  onInspect,
}: {
  play: PlayRecord;
  settled: number;
  settling: boolean;
  onBank: () => void;
  onInspect: () => void;
}) {
  const sayaReady = settled >= play.block;
  return (
    <div className="flex items-center gap-3 rounded-lg border bg-card px-3 py-2">
      <span className="grid size-8 shrink-0 place-items-center rounded-md bg-primary/10 text-sm font-bold tabular-nums text-primary">
        {play.score}
      </span>
      <button onClick={onInspect} className="flex-1 cursor-pointer text-left text-xs text-muted-foreground hover:text-foreground">
        {settling ? "banking…" : sayaReady ? "ready to bank" : "settling on saya…"}{" "}
        <span className="underline decoration-dotted">details</span>
      </button>
      <Button
        size="sm"
        className="h-8 shrink-0 gap-1.5 bg-green-600 text-xs text-white hover:bg-green-600/90"
        disabled={settling || !sayaReady}
        onClick={onBank}
      >
        {settling ? <Loader2 className="size-3.5 animate-spin" /> : <Vault className="size-3.5" />}
        {settling ? "Banking" : "Bank"}
      </Button>
    </div>
  );
}

function BankedRow({ play, best, onInspect }: { play: PlayRecord; best: number; onInspect: () => void }) {
  return (
    <button
      onClick={onInspect}
      className="flex w-full cursor-pointer items-center gap-2.5 rounded-lg border border-green-600/20 bg-card px-3 py-1.5 text-left transition-colors hover:bg-green-600/10"
    >
      <Check className="size-3.5 shrink-0 text-green-600" />
      <span className="text-sm font-semibold tabular-nums">{play.score}</span>
      {play.score === best && best > 0 && <Badge className="bg-green-600 text-[9px] text-white">BEST</Badge>}
      <span className="ml-auto text-[11px] text-muted-foreground">on L1 ↗</span>
    </button>
  );
}

function VaultStat(props: { label: string; value: React.ReactNode }) {
  return (
    <div className="rounded-lg border bg-card p-2.5 text-center">
      <div className="text-xl font-bold tabular-nums">{props.value}</div>
      <div className="text-[11px] text-muted-foreground">{props.label}</div>
    </div>
  );
}

function SayaGauge({ settled, tip, caughtUp }: { settled: number; tip: number; caughtUp: boolean }) {
  return (
    <Tooltip>
      <TooltipTrigger className="flex cursor-help items-center gap-1.5 rounded-full border bg-card px-3 py-1 text-xs text-muted-foreground decoration-dotted underline-offset-2 hover:underline">
        {caughtUp ? <Check className="size-3.5 text-green-600" /> : <Loader2 className="size-3.5 animate-spin text-primary" />}
        saya: settled <span className="font-mono text-foreground">{Math.max(settled, 0)}</span> / tip{" "}
        <span className="font-mono text-foreground">{tip}</span>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs">
        <div className="space-y-1.5 text-left leading-snug">
          <p>
            <b>saya</b> proves each appchain block and settles it onto L1. A roll can only be <b>banked</b> once its block
            is settled.
          </p>
          <p>
            <b>settled = tip</b> means saya is fully caught up — every roll is ready to bank.
          </p>
        </div>
      </TooltipContent>
    </Tooltip>
  );
}

// --- Under the hood (inspect layer) ---

function HoodDialog({ open, onOpenChange, online }: { open: boolean; onOpenChange: (o: boolean) => void; online: boolean }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Wrench className="size-4 text-muted-foreground" /> Under the hood
          </DialogTitle>
          <DialogDescription>The cross-chain plumbing this game runs on.</DialogDescription>
        </DialogHeader>
        <div className="grid items-stretch gap-3 md:grid-cols-[1fr_auto_1fr]">
          <ChainCard
            tag="Settlement · “L1”"
            rpc={SETTLEMENT_RPC}
            torii={TORII_SCORE}
            explorer={SETTLEMENT_EXPLORER}
            contracts={[
              { label: "piltover core", addr: PILTOVER },
              { label: "score world", addr: SCORE_WORLD },
              { label: "score system", addr: SCORE_REGISTRY },
            ]}
          />
          <div className="flex flex-col items-center justify-center gap-2 px-2 text-center text-muted-foreground">
            <span className="font-mono text-[11px] text-primary">buy → mint</span>
            <ArrowRight className="size-5 text-primary" />
            <ArrowLeft className="size-5 text-green-600" />
            <span className="font-mono text-[11px] text-green-600">bank + saya</span>
          </div>
          <ChainCard
            tag="Appchain · “L2”"
            rpc={APPCHAIN_RPC}
            torii={TORII_GAME}
            explorer={APPCHAIN_EXPLORER}
            contracts={[
              { label: "game world", addr: GAME_WORLD },
              { label: "game system", addr: GAME },
            ]}
          />
        </div>
        {!online && <p className="text-center text-xs text-amber-600">Not connected — start the stack with ./up.sh</p>}
      </DialogContent>
    </Dialog>
  );
}

function ChainCard(props: {
  tag: string;
  rpc: string;
  torii: string;
  explorer: string;
  contracts: { label: string; addr: string }[];
}) {
  return (
    <Card className="gap-0 py-4">
      <CardContent className="px-4">
        <div className="mb-2 flex items-center justify-between">
          <Badge variant="secondary" className="text-[10px] tracking-wide uppercase">{props.tag}</Badge>
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
          <dt className="mt-2 text-[11px] tracking-wide text-muted-foreground uppercase">Torii (indexer)</dt>
          <dd className="font-mono text-[13px] break-all">
            <a
              href={`${props.torii}/sql`}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 text-primary hover:underline"
            >
              {props.torii} <ExternalLink className="size-3" />
            </a>
          </dd>
          {props.contracts.map((c) => (
            <div key={c.label}>
              <dt className="mt-2 text-[11px] tracking-wide text-muted-foreground uppercase">{c.label}</dt>
              <dd className="font-mono text-[13px] break-all">
                {c.addr ? (
                  <a
                    href={explorerAddrUrl(props.explorer, c.addr)}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1 text-primary hover:underline"
                  >
                    {shortHex(c.addr, 10, 6)} <ExternalLink className="size-3" />
                  </a>
                ) : (
                  "—"
                )}
              </dd>
            </div>
          ))}
        </dl>
      </CardContent>
    </Card>
  );
}

// --- Detail + info dialogs ---

function Code({ children }: { children: React.ReactNode }) {
  return <code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">{children}</code>;
}

function withCode(text: string) {
  return text.split("`").map((part, i) => (i % 2 === 1 ? <Code key={i}>{part}</Code> : <Fragment key={i}>{part}</Fragment>));
}

function TxLink(props: { label: string; href: string; hash: string; tone: "l1" | "l2" }) {
  return (
    <a
      href={props.href}
      target="_blank"
      rel="noreferrer"
      title={props.hash}
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-xs transition-colors",
        props.tone === "l1"
          ? "border-primary/30 bg-primary/10 text-primary hover:bg-primary/20"
          : "border-green-600/30 bg-green-600/10 text-green-600 hover:bg-green-600/20",
      )}
    >
      {props.label} {shortHex(props.hash)} <ExternalLink className="size-3" />
    </a>
  );
}

function PlayDetailDialog({
  play,
  settled,
  onOpenChange,
}: {
  play: PlayRecord | null;
  settled: number;
  onOpenChange: (open: boolean) => void;
}) {
  const banked = !!play?.claimTxHash;
  const sayaReady = !!play && settled >= play.block;
  const steps = play
    ? [
        {
          label: "Rolled on L2",
          done: true,
          desc: "`play_game` rolled the score on the appchain and emitted it to L1 via `send_message_to_l1`.",
          tx: { label: "L2 tx", tone: "l2" as const, hash: play.l2TxHash, href: explorerTxUrl(APPCHAIN_EXPLORER, play.l2TxHash) },
        },
        {
          label: "Settled by saya",
          done: banked || sayaReady,
          desc: "saya proves the appchain block and submits `update_state` to the piltover core, registering the message.",
        },
        {
          label: "Banked to L1",
          done: banked,
          desc: "You bank it: `score_registry` consumes the message via `consume_message_from_appchain` and records the score.",
          tx: play.claimTxHash
            ? { label: "L1 tx", tone: "l1" as const, hash: play.claimTxHash, href: explorerTxUrl(SETTLEMENT_EXPLORER, play.claimTxHash) }
            : undefined,
        },
      ]
    : [];
  return (
    <Dialog open={!!play} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            🎲 Score {play?.score}
            <Badge variant="secondary" className="text-[10px]">L2 → L1</Badge>
          </DialogTitle>
          <DialogDescription>
            <span className={banked ? "text-green-600" : "text-amber-600"}>{banked ? "Banked on L1" : "Not yet banked"}</span>
          </DialogDescription>
        </DialogHeader>
        <ol className="flex flex-col">
          {steps.map((s, i) => (
            <li key={s.label} className="flex gap-3">
              <div className="flex flex-col items-center">
                <span
                  className={cn(
                    "grid size-6 shrink-0 place-items-center rounded-full border text-[10px]",
                    s.done ? "border-green-600 bg-green-600 text-white" : "border-border text-muted-foreground",
                  )}
                >
                  {s.done ? <Check className="size-3.5" /> : i + 1}
                </span>
                {i < steps.length - 1 && <span className={cn("my-1 w-px flex-1", s.done ? "bg-green-600" : "bg-border")} />}
              </div>
              <div className="flex-1 pb-4">
                <div className="text-sm font-medium">{s.label}</div>
                <div className="mt-0.5 text-xs text-muted-foreground">{withCode(s.desc)}</div>
                {s.tx && (
                  <div className="mt-2">
                    <TxLink {...s.tx} />
                  </div>
                )}
              </div>
            </li>
          ))}
        </ol>
      </DialogContent>
    </Dialog>
  );
}

function InfoButton({ title, className, children }: { title: string; className?: string; children: React.ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label={`How ${title.toLowerCase()} works`}
        className={cn("inline-flex cursor-pointer text-muted-foreground transition-colors hover:text-foreground", className)}
      >
        <Info className="size-4" />
      </button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-w-lg sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>How this step works on chain.</DialogDescription>
          </DialogHeader>
          <div className="space-y-5">{children}</div>
        </DialogContent>
      </Dialog>
    </>
  );
}

function StepInfo(props: { heading: string; services: string[]; steps: React.ReactNode[]; note: React.ReactNode }) {
  return (
    <div className="space-y-3 text-sm">
      <div className="text-xs font-semibold tracking-wide text-primary uppercase">{props.heading}</div>
      <div>
        <div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">Services</div>
        <div className="flex flex-wrap gap-1.5">
          {props.services.map((s) => (
            <Badge key={s} variant="secondary" className="font-mono text-[11px]">{s}</Badge>
          ))}
        </div>
      </div>
      <ol className="space-y-2">
        {props.steps.map((s, i) => (
          <li key={i} className="flex gap-2.5">
            <span className="grid size-5 shrink-0 place-items-center rounded-full bg-primary/15 text-[10px] font-bold text-primary">{i + 1}</span>
            <div className="flex-1 leading-snug">{s}</div>
          </li>
        ))}
      </ol>
      <p className="rounded-md bg-muted/60 p-2.5 text-xs text-muted-foreground">{props.note}</p>
    </div>
  );
}

function BuyInfo() {
  return (
    <StepInfo
      heading="Insert coin · L1 → L2"
      services={["settlement katana (L1)", "appchain katana — messaging"]}
      steps={[
        <>
          The buyer calls <Code>send_message_to_appchain(game, mint_game, [id])</Code> on the <b>piltover core</b> (L1),
          emitting <Code>MessageSent</Code>.
        </>,
        <>The appchain’s messaging service relays it as an <b>L1-handler</b> tx.</>,
        <>
          <Code>mint_game</Code> runs on the <b>game</b> contract (L2), adding a credit.
        </>,
      ]}
      note="A credit is minted once the appchain runs the L1 handler — relayed by Katana, no prover needed."
    />
  );
}

function PlayInfo() {
  return (
    <StepInfo
      heading="Roll · L2"
      services={["appchain katana (L2)"]}
      steps={[
        <>
          You call <Code>play_game()</Code> on the <b>game</b> contract.
        </>,
        <>
          It spends a credit and rolls on chain — <Code>poseidon(block_timestamp, play_no) % 100 + 1</Code> — emitting{" "}
          <Code>GamePlayed</Code> and a <Code>send_message_to_l1</Code> with your score.
        </>,
      ]}
      note="The roll is instant and free on L2. Your score becomes an unbanked message waiting to settle to L1."
    />
  );
}

function PublishInfo() {
  return (
    <StepInfo
      heading="Bank · L2 → L1"
      services={["saya-tee — prover / settler", "settlement katana (L1)"]}
      steps={[
        <>
          <b>saya</b> proves the appchain block and submits <Code>update_state</Code> to the <b>piltover core</b>,
          registering the message.
        </>,
        <>
          Once settled, you click <b>Bank</b> → <Code>claim_score</Code> on <b>score_registry</b> calls{" "}
          <Code>consume_message_from_appchain</Code> and stores the score, emitting <Code>ScoreClaimed</Code>.
        </>,
      ]}
      note="Banking is your explicit step — a score only counts in the Vault once it's settled and consumed on L1."
    />
  );
}

function IntroDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Dices className="size-5 text-primary" /> Cross-Chain Dice
          </DialogTitle>
          <DialogDescription>A tiny game that shows how a Starknet appchain talks to L1.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4 text-sm">
          <p>
            You play in the <b>Arcade</b> (a Katana <b>appchain</b>, “L2”) and bank your scores into the <b>Vault</b> (the
            settlement layer, “L1”) — the permanent record, secured by <b>saya</b>.
          </p>
          <div className="space-y-2.5">
            <div className="flex items-start gap-2.5">
              <Coins className="mt-0.5 size-4 shrink-0 text-primary" />
              <p>
                <b className="text-primary">Insert coin</b> — buying a credit on L1 sends a message that mints it on the
                appchain (L1 → L2).
              </p>
            </div>
            <div className="flex items-start gap-2.5">
              <Dices className="mt-0.5 size-4 shrink-0 text-primary" />
              <p>
                <b className="text-primary">Roll</b> — play instantly and freely on the appchain to win points.
              </p>
            </div>
            <div className="flex items-start gap-2.5">
              <Vault className="mt-0.5 size-4 shrink-0 text-green-600" />
              <p>
                <b className="text-green-600">Bank</b> — settle a score to L1 (L2 → L1, via saya) to lock it into the Vault
                for good.
              </p>
            </div>
          </div>
          <p className="text-muted-foreground">
            Curious how it works? Hit <b>Under the hood</b> or the ⓘ icons to see the contracts, messages, and transactions
            behind every action.
          </p>
        </div>
        <Button className="w-full cursor-pointer" onClick={() => onOpenChange(false)}>
          Let’s play
        </Button>
      </DialogContent>
    </Dialog>
  );
}
