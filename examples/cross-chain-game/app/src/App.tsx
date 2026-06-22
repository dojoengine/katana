import { Fragment, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Coins,
  Compass,
  Dices,
  ExternalLink,
  Info,
  Loader2,
  LogIn,
  LogOut,
  MousePointerClick,
  PlugZap,
  Settings,
  ShieldCheck,
  Trophy,
  Vault,
  Wallet,
  Workflow,
  Wrench,
  X,
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
  readCredits,
  getPurchaseHistory,
  getPlayHistory,
  probeServices,
  type ServiceId,
  subscribeToriiUpdates,
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
  STORE,
  STORE_WORLD,
  GAME,
  GAME_WORLD,
  TORII_SCORE,
  TORII_GAME,
  BUYER_ADDRESS,
} from "./chain.ts";
import { sourceUrl } from "./source.ts";
import { useWallet } from "./wallet.tsx";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// A pulsing placeholder shown in place of a value while the first read is in flight.
function Skeleton({ className }: { className?: string }) {
  return <span className={cn("inline-block animate-pulse rounded bg-muted-foreground/20 align-middle", className)} />;
}

// A stat reflects connection status: the real value when online, a skeleton while
// the first read is loading, an em dash once we know the stack is unreachable.
function stat(online: boolean, loading: boolean, value: React.ReactNode, skeleton = "h-3.5 w-6"): React.ReactNode {
  if (online) return value;
  if (loading) return <Skeleton className={skeleton} />;
  return "—";
}

// Slim banner while the very first read is in flight (the stack may be booting).
function ConnectingBanner() {
  return (
    <div className="mb-4 flex shrink-0 items-center gap-2.5 rounded-xl border bg-card px-4 py-2.5 text-sm text-muted-foreground">
      <Loader2 className="size-4 animate-spin text-primary" />
      Connecting to the local stack…
    </div>
  );
}

// --- Per-service status indicators ---

type SvcStatus = "up" | "down" | "settling" | "stalled" | "unknown";
type SvcItem = { label: string; detail: string; status: SvcStatus };

const SVC_DOT: Record<SvcStatus, string> = {
  up: "bg-green-500",
  settling: "bg-amber-500 animate-pulse",
  stalled: "bg-red-500",
  down: "bg-red-500",
  unknown: "bg-muted-foreground/40",
};
const SVC_WORD: Record<SvcStatus, string> = {
  up: "running",
  settling: "settling",
  stalled: "stalled",
  down: "not running",
  unknown: "unknown",
};

function ServiceDot({ status }: { status: SvcStatus }) {
  return <span className={cn("size-2 shrink-0 rounded-full", SVC_DOT[status])} />;
}

// The four directly-probeable network services. `services === null` (first load)
// shows them as unknown.
function networkServiceItems(services: Record<ServiceId, boolean> | null): SvcItem[] {
  const st = (up: boolean | undefined): SvcStatus => (services == null ? "unknown" : up ? "up" : "down");
  return [
    { label: "Settlement node", detail: ":5050", status: st(services?.settlement) },
    { label: "Appchain node", detail: ":5051", status: st(services?.appchain) },
    { label: "Torii · score", detail: ":8081", status: st(services?.toriiScore) },
    { label: "Torii · game", detail: ":8082", status: st(services?.toriiGame) },
  ];
}

// Compact always-visible row (bottom of the page). the settler is shown by <SettlerGauge>.
function ServiceStatusBar({ items }: { items: SvcItem[] }) {
  return (
    <div className="flex flex-wrap items-center justify-center gap-x-3.5 gap-y-1.5 text-xs text-muted-foreground">
      {items.map((s) => (
        <span key={s.label} className="flex items-center gap-1.5" title={`${s.detail} · ${SVC_WORD[s.status]}`}>
          <ServiceDot status={s.status} />
          {s.label}
        </span>
      ))}
    </div>
  );
}

// Detailed list for the offline modal: one row per directly-probeable service.
function ServiceStatusList({ items }: { items: SvcItem[] }) {
  return (
    <div className="space-y-1.5">
      {items.map((s) => (
        <div key={s.label} className="flex items-center justify-between rounded-md border px-2.5 py-1.5 text-xs">
          <span className="flex items-center gap-2 font-medium">
            <ServiceDot status={s.status} /> {s.label}
          </span>
          <span className="font-mono text-muted-foreground">
            {s.detail} · {SVC_WORD[s.status]}
          </span>
        </div>
      ))}
    </div>
  );
}

// Shown once a read has failed: the services aren't up. Stands in for the intro
// modal. Non-dismissible (no close button, ignores Escape/backdrop) and bound to
// `open={offline}`, so it clears itself the moment the poll reconnects.
function OfflineDialog({ open, services }: { open: boolean; services: SvcItem[] }) {
  return (
    <Dialog open={open} onOpenChange={() => {}}>
      <DialogContent showCloseButton={false} className="max-w-md sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <PlugZap className="size-5 text-amber-600" /> Can't reach the local stack
          </DialogTitle>
          <DialogDescription>
            Some services aren't responding (see below). Start the stack and this dialog closes on its own.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <ServiceStatusList items={services} />
          <pre className="rounded-lg border bg-muted px-3 py-2 font-mono text-xs">
            cd examples/cross-chain-game{"\n"}./up.sh
          </pre>
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="size-3.5 animate-spin text-primary" /> Waiting for services — reconnects automatically.
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function App() {
  const [online, setOnline] = useState(false);
  const [loading, setLoading] = useState(true); // true until the first fetch settles (ok or fail)
  const [services, setServices] = useState<Record<ServiceId, boolean> | null>(null);
  const [, setGame] = useState<GameState | null>(null); // global stats read but credits are now per-player
  const [, setScore] = useState<ScoreState | null>(null);
  const [settled, setSettled] = useState(-1);
  const [tip, setTip] = useState(0);
  const [purchases, setPurchases] = useState<PurchaseRecord[]>([]);
  const [plays, setPlays] = useState<PlayRecord[]>([]);
  const [credits, setCredits] = useState<Map<string, number>>(new Map()); // per-player game credits

  const [buying, setBuying] = useState(false);
  const [rolling, setRolling] = useState(false);
  const [rollDisplay, setRollDisplay] = useState<number | null>(null);
  const [settling, setSettling] = useState<Set<number>>(new Set());
  const [introOpen, setIntroOpen] = useState(true);
  const [tourStep, setTourStep] = useState(-1); // -1 = tour closed
  const [hoodOpen, setHoodOpen] = useState(false);
  const [coinsOpen, setCoinsOpen] = useState(false);
  const [flowOpen, setFlowOpen] = useState(false);
  const [walletOpen, setWalletOpen] = useState(false);
  const wallet = useWallet();
  const [detail, setDetail] = useState<PlayRecord | null>(null);
  const rollStart = useRef(0); // plays.length when the current roll started
  const rollToken = useRef(0); // invalidates a stale play_game() resolution
  const refetch = useRef<() => void>(() => {}); // nudge the data layer after a write
  const settlerProgress = useRef({ last: -1, stalls: 0 }); // detect a stalled settler

  useEffect(() => {
    let active = true;
    const tick = async () => {
      // Probe each service independently first, so the UI can show exactly which
      // one is down rather than a single all-or-nothing flag.
      const health = await probeServices();
      if (!active) return;
      setServices(health);
      const reachable = health.settlement && health.appchain && health.toriiScore && health.toriiGame;
      try {
        if (!reachable) throw new Error("offline");
        const [g, sc, ph, plh, sb, tp, cr] = await Promise.all([
          readGameState(),
          readScoreState(),
          getPurchaseHistory(),
          getPlayHistory(),
          settledBlock(),
          appchainBlock(),
          readCredits(),
        ]);
        if (!active) return;
        setGame(g);
        setScore(sc);
        setPurchases(ph);
        setPlays(plh);
        setSettled(sb);
        setTip(tp);
        setCredits(cr);
        // Track settler progress to tell "settling" (advancing) from "stalled"
        // (behind and not moving — the settler likely down).
        const p = settlerProgress.current;
        if (sb > p.last) p.stalls = 0;
        else if (tp > sb) p.stalls += 1;
        p.last = sb;
        setOnline(true);
      } catch {
        if (active) setOnline(false);
      } finally {
        if (active) setLoading(false);
      }
    };

    // Coalesce bursts of subscription pushes (a single tx fires both an entity
    // and an event-message update) into one refetch.
    let debounce: ReturnType<typeof setTimeout> | null = null;
    const ping = () => {
      if (debounce) clearTimeout(debounce);
      debounce = setTimeout(() => active && tick(), 120);
    };
    refetch.current = ping; // let write handlers nudge a refetch (RPC-only states)

    tick(); // initial load

    // Torii entity/event subscriptions drive the snappy gameplay state. A slow
    // poll stays as a safety net and keeps the RPC-only reads fresh (the settler-settled
    // block + appchain tip have no Torii subscription to push them).
    const slow = setInterval(tick, 4000);
    let cleanupSub: (() => void) | undefined;
    subscribeToriiUpdates(ping)
      .then((cleanup) => {
        if (active) cleanupSub = cleanup;
        else cleanup();
      })
      .catch((e) => console.error("torii subscribe failed; falling back to poll", e));

    return () => {
      active = false;
      if (debounce) clearTimeout(debounce);
      clearInterval(slow);
      cleanupSub?.();
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

  const offline = !loading && !online; // first read failed → services aren't up

  // Per-service status. The four network services come straight from the probe;
  // the settler has no endpoint, so it's inferred: caught up → running, behind but
  // advancing → settling, behind and stuck → stalled (likely not running).
  const settlerStatus: SvcStatus = !online
    ? "unknown"
    : settled >= 0 && settled >= tip
      ? "up"
      : settlerProgress.current.stalls >= 3
        ? "stalled"
        : "settling";
  const networkItems = networkServiceItems(services);

  const pendingMints = purchases.filter((p) => !p.mintTxHash).length;
  const coinLoading = buying || pendingMints > 0; // submitting on L1 or still minting on L2
  // The connected wallet's own credits + rolls. Credits are per-player; a roll is
  // keyed to whoever made it (GamePlayed.player), and you can only bank your own
  // (claim_score consumes the message for that player). Compare by value — Torii
  // pads the address.
  let myPlayer: bigint | null = null;
  try {
    myPlayer = BigInt(wallet.playerAddress);
  } catch {
    myPlayer = null;
  }
  // Credits the connected player owns (0 when disconnected, or none yet).
  const available = wallet.connected && myPlayer !== null ? (credits.get(myPlayer.toString()) ?? 0) : 0;
  const mine =
    myPlayer === null
      ? []
      : plays.filter((p) => {
          try {
            return BigInt(p.player) === myPlayer;
          } catch {
            return false;
          }
        });
  const unbanked = mine.filter((p) => !p.claimTxHash);
  const banked = mine.filter((p) => p.claimTxHash);
  const best = banked.reduce((m, p) => Math.max(m, p.score), 0);
  const totalPoints = banked.reduce((s, p) => s + p.score, 0);

  // Insert coin → buy a credit (L1 -> L2 mint).
  async function onBuy() {
    const acc = wallet.l1Account;
    if (!acc) return setWalletOpen(true); // no wallet → prompt to connect
    setBuying(true);
    try {
      await purchaseGame(acc, wallet.playerAddress, purchases.length + 1);
      refetch.current(); // surface the pending mint (piltover MessageSent is RPC-only)
    } catch (e) {
      console.error("buy failed", e);
    } finally {
      setBuying(false);
    }
  }

  // Roll the dice (play on L2). Cycle numbers for juice, then land on the real
  // on-chain roll read from the GamePlayed event.
  async function onRoll() {
    const l2 = wallet.l2Account;
    if (!l2) return setWalletOpen(true); // no wallet → prompt to connect
    if (rolling || available < 1) return;
    const token = ++rollToken.current;
    rollStart.current = plays.length;
    setRolling(true);
    try {
      const { score } = await playGame(l2);
      if (rollToken.current === token) {
        setRollDisplay(score);
        setRolling(false);
      }
    } catch (e) {
      if (rollToken.current === token) setRolling(false);
      console.error("roll failed", e);
    }
  }

  // Bank a roll to the Vault (settle L2 -> L1). Retries until the settler has settled.
  // `player` is the roll's own player (GamePlayed.player), not the connected
  // wallet: claim_score consumes the L2→L1 message keyed to whoever rolled, and
  // its consume isn't caller-restricted, so any connected L1 signer can settle it.
  async function onBank(seq: number, sc: number, player: string) {
    const acc = wallet.l1Account;
    if (!acc) return setWalletOpen(true); // no wallet → prompt to connect
    setSettling((s) => new Set(s).add(seq));
    try {
      for (let i = 0; i < 90; i++) {
        try {
          await claimScore(acc, player, sc);
          refetch.current(); // ScoreClaimed will also push, but don't wait on it
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

  const settlerCaughtUp = settled >= tip;

  return (
    <TooltipProvider>
      {/* When the stack is down, the offline modal stands in for the intro. The
          intro only opens once we're connected, so the two never collide. */}
      <OfflineDialog open={offline} services={networkItems} />
      <IntroDialog
        open={introOpen && online}
        onSkip={() => setIntroOpen(false)}
        onStartTutorial={() => {
          setIntroOpen(false);
          setTourStep(0);
        }}
      />
      <Tour
        step={tourStep}
        onStep={setTourStep}
        state={{ online, available, playsLen: plays.length, bankedLen: banked.length }}
      />
      <CoinMessagesDialog open={coinsOpen} onOpenChange={setCoinsOpen} purchases={purchases} />
      <PlayDetailDialog play={detail} settled={settled} onOpenChange={(o) => !o && setDetail(null)} />
      <HoodDialog open={hoodOpen} onOpenChange={setHoodOpen} online={online} />
      <FlowDialog open={flowOpen} onOpenChange={setFlowOpen} />
      <WalletDialog open={walletOpen} onOpenChange={setWalletOpen} />

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
              <Hud icon={<Coins className="size-3.5 text-primary" />} label="Credits" value={stat(online, loading, available)} />
              <Hud icon={<Trophy className="size-3.5 text-green-600" />} label="Best" value={stat(online, loading, best)} tone="green" />
              <Button
                variant="outline"
                className="h-10 gap-1.5 rounded-full px-3 text-xs"
                onClick={() => setWalletOpen(true)}
                aria-label={wallet.connected ? "Account" : "Login"}
              >
                {wallet.connected ? (
                  wallet.method === "controller" ? (
                    <Wallet className="size-4 text-primary" />
                  ) : (
                    <Coins className="size-4 text-primary" />
                  )
                ) : (
                  <LogIn className="size-4" />
                )}
                <span className="max-w-28 truncate">{wallet.connected ? wallet.label : "Login"}</span>
              </Button>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button variant="ghost" size="icon" className="size-10 rounded-full" onClick={() => setFlowOpen(true)} aria-label="Contract flow" />
                  }
                >
                  <Workflow className="size-5" />
                </TooltipTrigger>
                <TooltipContent>Contract flow</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button data-tour="hood" variant="ghost" size="icon" className="size-10 rounded-full" onClick={() => setHoodOpen(true)} aria-label="Under the hood" />
                  }
                >
                  <Settings className="size-5 transition-transform duration-300 group-hover/button:rotate-90" />
                </TooltipTrigger>
                <TooltipContent>Under the hood</TooltipContent>
              </Tooltip>
            </div>
          </header>

          {/* Connecting hint while the first read is in flight. The offline state
              is handled by <OfflineDialog> (a blocking modal), not a banner. */}
          {loading && <ConnectingBanner />}

          {/* Game area */}
          <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-[1.35fr_1fr]">
            {/* Arcade (L2) */}
            <Card data-tour="arcade" className="relative overflow-hidden">
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
                  <Button data-tour="roll" size="lg" className="h-12 w-48 text-base" onClick={onRoll} disabled={rolling || available < 1 || !online}>
                    {rolling ? <Loader2 className="size-5 animate-spin" /> : <Dices className="size-5" />}
                    {rolling ? "Rolling…" : available < 1 ? "No credits" : "Roll"}
                  </Button>
                  <div className="flex flex-wrap items-center justify-center gap-2 text-xs text-muted-foreground">
                    <button
                      type="button"
                      onClick={() => setCoinsOpen(true)}
                      title="View the L1 → L2 messages behind your credits"
                      className="inline-flex cursor-pointer items-center gap-1 rounded-md decoration-dotted underline-offset-2 transition-colors hover:text-foreground hover:underline"
                    >
                      <Coins className="size-3.5" /> {available} credit{available === 1 ? "" : "s"}
                    </button>
                    <Button
                      data-tour="coin"
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
                <div data-tour="rolls" className="flex min-h-0 flex-1 flex-col">
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
                              onBank={() => onBank(p.seq, p.score, p.player)}
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
            <Card data-tour="vault" className="relative overflow-hidden border-green-600/30 bg-green-600/5">
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
                  <div className="text-5xl font-bold tabular-nums text-green-600">{stat(online, loading, best, "h-10 w-20")}</div>
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <VaultStat label="Runs banked" value={stat(online, loading, banked.length, "h-5 w-8")} />
                  <VaultStat label="Total points" value={stat(online, loading, totalPoints, "h-5 w-8")} />
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

          {/* System status: per-service dots + the settler gauge */}
          <div data-tour="settler" className="mt-4 flex shrink-0 flex-wrap items-center justify-center gap-x-4 gap-y-2">
            <ServiceStatusBar items={networkItems} />
            <span className="hidden h-4 w-px bg-border sm:block" />
            {online ? (
              <SettlerGauge settled={settled} tip={tip} caughtUp={settlerCaughtUp} stalled={settlerStatus === "stalled"} />
            ) : (
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <ServiceDot status={settlerStatus} /> settler: {loading ? "connecting…" : "offline"}
              </span>
            )}
          </div>
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
  const settlerReady = settled >= play.block;
  return (
    <div className="flex items-center gap-3 rounded-lg border bg-card px-3 py-2">
      <span className="grid size-8 shrink-0 place-items-center rounded-md bg-primary/10 text-sm font-bold tabular-nums text-primary">
        {play.score}
      </span>
      <button onClick={onInspect} className="flex-1 cursor-pointer text-left text-xs text-muted-foreground hover:text-foreground">
        {settling ? "banking…" : settlerReady ? "ready to bank" : "settling…"}{" "}
        <span className="underline decoration-dotted">details</span>
      </button>
      <Button
        size="sm"
        data-tour="bank"
        className="h-8 shrink-0 gap-1.5 bg-green-600 text-xs text-white hover:bg-green-600/90"
        disabled={settling || !settlerReady}
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

function SettlerGauge({ settled, tip, caughtUp, stalled }: { settled: number; tip: number; caughtUp: boolean; stalled?: boolean }) {
  return (
    <Tooltip>
      <TooltipTrigger
        className={cn(
          "flex cursor-help items-center gap-1.5 rounded-full border bg-card px-3 py-1 text-xs decoration-dotted underline-offset-2 hover:underline",
          stalled ? "border-red-500/40 text-red-600" : "text-muted-foreground",
        )}
      >
        {stalled ? (
          <PlugZap className="size-3.5 text-red-600" />
        ) : caughtUp ? (
          <Check className="size-3.5 text-green-600" />
        ) : (
          <Loader2 className="size-3.5 animate-spin text-primary" />
        )}
        settler: settled <span className="font-mono text-foreground">{Math.max(settled, 0)}</span> / tip{" "}
        <span className="font-mono text-foreground">{tip}</span>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs">
        <div className="space-y-1.5 text-left leading-snug">
          <p>
            The <b>settler</b> proves each appchain block and settles it onto L1. A roll can only be <b>banked</b> once its block
            is settled.
          </p>
          {stalled ? (
            <p>
              <b className="text-red-600">Stalled</b> — the settled height is stuck below the tip. The settler may have stopped;
              check its log.
            </p>
          ) : (
            <p>
              <b>settled = tip</b> means the settler is fully caught up — every roll is ready to bank.
            </p>
          )}
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
              { label: "store world", addr: STORE_WORLD },
              { label: "store system", addr: STORE },
              { label: "score world", addr: SCORE_WORLD },
              { label: "score system", addr: SCORE_REGISTRY },
            ]}
          />
          <div className="flex flex-col items-center justify-center gap-2 px-2 text-center text-muted-foreground">
            <span className="font-mono text-[11px] text-primary">buy → mint</span>
            <ArrowRight className="size-5 text-primary" />
            <ArrowLeft className="size-5 text-green-600" />
            <span className="font-mono text-[11px] text-green-600">bank + settle</span>
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

// A box for one contract inside a chain zone of the flow diagram.
function FlowBox(props: { name: string; fns?: string[]; desc?: string; tone: "green" | "primary" }) {
  const ring = props.tone === "green" ? "border-green-600/30 bg-green-600/5" : "border-primary/30 bg-primary/5";
  return (
    <div className={cn("rounded-lg border px-3 py-2", ring)}>
      <div className="font-mono text-xs font-semibold">{props.name}</div>
      {props.fns && (
        <div className="mt-1 flex flex-wrap gap-1">
          {props.fns.map((f) => (
            <Code key={f}>{f}</Code>
          ))}
        </div>
      )}
      {props.desc && <div className="mt-0.5 text-[11px] text-muted-foreground">{props.desc}</div>}
    </div>
  );
}

// One cross-chain message direction in the flow diagram.
function FlowHop(props: { tag: string; tone: "green" | "primary"; dir: "right" | "left"; children: React.ReactNode }) {
  const tagCls = props.tone === "green" ? "bg-green-600/10 text-green-600" : "bg-primary/10 text-primary";
  const Arrow = props.dir === "right" ? ArrowRight : ArrowLeft;
  return (
    <div className="rounded-lg border bg-card p-2.5 text-center">
      <div className="mb-1 flex items-center justify-center gap-1.5">
        {props.dir === "left" && <Arrow className={cn("size-4", props.tone === "green" ? "text-green-600" : "text-primary")} />}
        <span className={cn("rounded-full px-2 py-0.5 font-mono text-[10px] font-medium", tagCls)}>{props.tag}</span>
        {props.dir === "right" && <Arrow className={cn("size-4", props.tone === "green" ? "text-green-600" : "text-primary")} />}
      </div>
      <div className="text-left text-[11px] leading-snug text-muted-foreground [&_b]:text-foreground">{props.children}</div>
    </div>
  );
}

function FlowDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Workflow className="size-5 text-primary" /> Contract flow
          </DialogTitle>
          <DialogDescription>
            Two chains, four contracts, two message directions — how one round trip moves between them. Tap any function
            to open its source.
          </DialogDescription>
        </DialogHeader>

        <div className="grid items-stretch gap-3 md:grid-cols-[1fr_1.25fr_1fr]">
          {/* Settlement (L1) */}
          <div className="flex flex-col gap-2 rounded-xl border border-green-600/30 bg-green-600/[0.04] p-3">
            <div className="text-center text-xs font-semibold tracking-wide text-green-600 uppercase">Settlement · L1</div>
            <FlowBox tone="green" name="store" fns={["buy_game"]} />
            <FlowBox tone="green" name="piltover core" desc="mailbox + settled state" />
            <FlowBox tone="green" name="score" fns={["claim_score"]} />
          </div>

          {/* Messages between the chains */}
          <div className="flex flex-col justify-center gap-3">
            <FlowHop tag="L1 → L2" tone="primary" dir="right">
              <b>store</b> runs its rules, then calls <Code>send_message_to_appchain</Code> on the piltover core. Katana
              relays it into <Code>mint_game</Code> — instant, no prover.
            </FlowHop>
            <FlowHop tag="L2 → L1" tone="green" dir="left">
              <Code>play_game</Code> emits <Code>send_message_to_l1</Code>. The <b>settler</b> proves the block and{" "}
              <Code>update_state</Code>s piltover; then <Code>claim_score</Code> calls{" "}
              <Code>consume_message_from_appchain</Code> — settled.
            </FlowHop>
          </div>

          {/* Appchain (L2) */}
          <div className="flex flex-col gap-2 rounded-xl border border-primary/30 bg-primary/[0.04] p-3">
            <div className="text-center text-xs font-semibold tracking-wide text-primary uppercase">Appchain · L2</div>
            <FlowBox tone="primary" name="game" fns={["mint_game", "play_game"]} desc="mint_game is an l1_handler" />
          </div>
        </div>

        <div className="flex items-start gap-2.5 rounded-lg border bg-muted/50 p-3 text-xs text-muted-foreground [&_b]:text-foreground">
          <ShieldCheck className="mt-0.5 size-4 shrink-0 text-green-600" />
          <p>
            The appchain's embedded <b>settler</b> bridges L2 → L1: it proves each appchain block and submits <Code>update_state</Code>, which is what
            lets the settlement layer consume a message. A <b>Torii</b> indexer per chain mirrors the worlds so this UI can
            read them. (Proving runs in mock mode for this demo.)
          </p>
        </div>
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
  const cls = "rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]";
  // If the snippet names one of our contract symbols, link it to the source.
  const url = typeof children === "string" ? sourceUrl(children) : null;
  if (url) {
    return (
      <a
        href={url}
        target="_blank"
        rel="noreferrer"
        title="View source on GitHub"
        className={cn(cls, "text-primary underline decoration-dotted underline-offset-2 hover:bg-primary/10")}
      >
        {children}
      </a>
    );
  }
  return <code className={cls}>{children}</code>;
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
  const settlerReady = !!play && settled >= play.block;
  const steps = play
    ? [
        {
          label: "Rolled on L2",
          done: true,
          desc: "`play_game` rolled the score on the appchain and emitted it to L1 via `send_message_to_l1`.",
          tx: { label: "L2 tx", tone: "l2" as const, hash: play.l2TxHash, href: explorerTxUrl(APPCHAIN_EXPLORER, play.l2TxHash) },
        },
        {
          label: "Settled by the settler",
          done: banked || settlerReady,
          desc: "The settler proves the appchain block and submits `update_state` to the piltover core, registering the message.",
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

// One dialog, two faces: when connected it shows the account profile (with a log
// out button); when disconnected it shows the connect picker (dev / Controller).
function WalletDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const wallet = useWallet();

  if (wallet.connected) {
    const isCtrl = wallet.method === "controller";
    return (
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="max-w-md sm:max-w-md">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <Wallet className="size-5 text-primary" /> Account
            </DialogTitle>
            <DialogDescription>Connected wallet — signs buy, roll, and bank.</DialogDescription>
          </DialogHeader>
          <div className="flex items-center gap-3 rounded-lg border p-3">
            <div className="grid size-10 shrink-0 place-items-center rounded-full bg-primary/10">
              {isCtrl ? <Wallet className="size-5 text-primary" /> : <Coins className="size-5 text-muted-foreground" />}
            </div>
            <div className="min-w-0 flex-1">
              <div className="truncate font-medium">{isCtrl ? (wallet.username ?? "Controller") : "Dev account"}</div>
              <div className="truncate font-mono text-xs text-muted-foreground">{shortHex(wallet.l1Address, 6, 4)}</div>
            </div>
            <Badge variant={isCtrl ? "default" : "secondary"} className="shrink-0">
              {isCtrl ? "Controller" : "Dev"}
            </Badge>
          </div>
          <Button
            variant="outline"
            className="gap-2"
            onClick={() => {
              wallet.disconnect().catch((e) => console.error("disconnect failed", e));
              onOpenChange(false);
            }}
          >
            <LogOut className="size-4" /> Log out
          </Button>
        </DialogContent>
      </Dialog>
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Wallet className="size-5 text-primary" /> Connect a wallet
          </DialogTitle>
          <DialogDescription>
            Connect to play — choose how buy, roll, and bank are signed. Nothing is connected by default.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2.5">
          <WalletOption
            icon={<Coins className="size-5 text-muted-foreground" />}
            title="Dev account"
            subtitle={<>Prefunded local key · <span className="font-mono">{shortHex(BUYER_ADDRESS, 6, 4)}</span></>}
            onClick={() => wallet.useDevAccount().then(() => onOpenChange(false)).catch((e) => console.error(e))}
          />
          <WalletOption
            busy={wallet.connecting}
            disabled={!wallet.controllerAvailable}
            icon={<Wallet className="size-5 text-primary" />}
            title="Cartridge Controller"
            subtitle={
              wallet.controllerAvailable
                ? "Connect a passkey/social wallet"
                : "Unavailable — start the stack first (./up.sh)"
            }
            onClick={() => {
              if (wallet.controllerAvailable)
                wallet
                  .connectController()
                  .then(() => onOpenChange(false))
                  .catch((e) => console.error("controller connect failed", e));
            }}
          />
        </div>
        <p className="rounded-md bg-muted/60 p-2.5 text-xs text-muted-foreground [&_b]:text-foreground">
          Controller is optional and needs the stack started with <Code>CONTROLLER=1 ./up.sh</Code> (plus a Controller
          login). The dev account works offline. See the README.
        </p>
      </DialogContent>
    </Dialog>
  );
}

function WalletOption(props: {
  active?: boolean;
  busy?: boolean;
  disabled?: boolean;
  icon: React.ReactNode;
  title: string;
  subtitle: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={props.onClick}
      disabled={props.disabled}
      className={cn(
        "flex w-full items-center gap-3 rounded-lg border p-3 text-left transition-colors",
        props.disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer hover:bg-muted/50",
        props.active && "border-primary/50 bg-primary/5",
      )}
    >
      <span className="shrink-0">{props.icon}</span>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">{props.title}</div>
        <div className="truncate text-xs text-muted-foreground">{props.subtitle}</div>
      </div>
      {props.busy ? (
        <Loader2 className="size-4 shrink-0 animate-spin text-primary" />
      ) : props.active ? (
        <Check className="size-4 shrink-0 text-primary" />
      ) : null}
    </button>
  );
}

function CoinMessagesDialog({
  open,
  onOpenChange,
  purchases,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  purchases: PurchaseRecord[];
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Coins className="size-5 text-primary" /> L1 → L2 messages
            <Badge variant="secondary" className="text-[10px]">{purchases.length}</Badge>
          </DialogTitle>
          <DialogDescription>
            Each <b>Insert coin</b> calls <Code>buy_game</Code> on the L1 store, which messages the appchain (relayed into{" "}
            <Code>mint_game</Code>) to mint one credit on L2.
          </DialogDescription>
        </DialogHeader>
        {purchases.length === 0 ? (
          <p className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
            No credits bought yet — hit <b>Insert coin</b> to send your first L1 → L2 message.
          </p>
        ) : (
          <div className="flex max-h-[60vh] flex-col gap-2 overflow-y-auto py-px pr-1">
            {[...purchases].reverse().map((p) => (
              <div key={p.seq} className="flex items-center gap-3 rounded-lg border bg-card px-3 py-2">
                <span className="grid size-7 shrink-0 place-items-center rounded-md bg-primary/10 text-xs font-bold tabular-nums text-primary">
                  {p.seq}
                </span>
                <div className="flex flex-1 flex-wrap items-center gap-1.5">
                  <TxLink
                    label="L1 send"
                    tone="l1"
                    hash={p.l1TxHash}
                    href={explorerTxUrl(SETTLEMENT_EXPLORER, p.l1TxHash)}
                  />
                  {p.mintTxHash ? (
                    <TxLink
                      label="L2 mint"
                      tone="l2"
                      hash={p.mintTxHash}
                      href={explorerTxUrl(APPCHAIN_EXPLORER, p.mintTxHash)}
                    />
                  ) : (
                    <span className="inline-flex items-center gap-1 rounded-full border border-amber-500/30 bg-amber-500/10 px-2 py-0.5 text-xs text-amber-600">
                      <Loader2 className="size-3 animate-spin" /> relaying…
                    </span>
                  )}
                </div>
                <span className={cn("shrink-0 text-xs", p.mintTxHash ? "text-green-600" : "text-amber-600")}>
                  {p.mintTxHash ? "Minted" : "Pending"}
                </span>
              </div>
            ))}
          </div>
        )}
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
          The buyer calls <Code>buy_game</Code> on the <b>store</b> contract (L1), which runs the store’s rules.
        </>,
        <>
          The store then calls <Code>send_message_to_appchain</Code> on the <b>piltover core</b>, emitting{" "}
          <Code>MessageSent</Code>.
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
      services={["katana — embedded settler", "settlement katana (L1)"]}
      steps={[
        <>
          The <b>settler</b> proves the appchain block and submits <Code>update_state</Code> to the <b>piltover core</b>,
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

function IntroDialog({
  open,
  onSkip,
  onStartTutorial,
}: {
  open: boolean;
  onSkip: () => void;
  onStartTutorial: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onSkip()}>
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
            settlement layer, “L1”) — the permanent record, secured by the <b>settler</b>.
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
                <b className="text-green-600">Bank</b> — settle a score to L1 (L2 → L1, via the settler) to lock it into the Vault
                for good.
              </p>
            </div>
          </div>
          <p className="text-muted-foreground">
            New to appchains? The tutorial walks through each action and shows what happens behind the stage — the chains,
            contracts, and messages that make it work.
          </p>
          <div className="flex items-start gap-2.5 rounded-lg border bg-muted/50 p-3 text-xs text-muted-foreground">
            <ShieldCheck className="mt-0.5 size-4 shrink-0 text-green-600" />
            <p>
              The appchain settles to L1 with <b>TEE proving</b>. This demo runs it in <b>mock mode</b> for local dev
              (<Code>--tee mock</Code> / <Code>--mock-prove</Code>) — it exercises the real messaging &amp; settlement
              flow, not actual proof or enclave attestation.
            </p>
          </div>
        </div>
        <div className="flex flex-col gap-2 sm:flex-row-reverse">
          <Button className="w-full cursor-pointer sm:flex-1" onClick={onStartTutorial}>
            <Compass className="size-4" /> Start tutorial
          </Button>
          <Button variant="outline" className="w-full cursor-pointer sm:flex-1" onClick={onSkip}>
            I already know
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

// --- Tutorial: a spotlight tour over the real UI ---
//
// Each step highlights a live element (by its data-tour attribute) and explains
// what that operation does in appchain terms — not how to play the game.

type TourState = { online: boolean; available: number; playsLen: number; bankedLen: number };

type TourStep = {
  sel: string; // data-tour value of the element to spotlight
  tag: string;
  tone: "primary" | "green" | "muted";
  title: string;
  body: React.ReactNode;
  place?: "auto" | "left"; // popover placement relative to the target (default auto)
  // Interactive step: the user performs the real action on the highlighted
  // element; the tour drops its click-blocker and auto-advances once `done`
  // flips true (a new credit minted, a new roll landed, …).
  action?: { cta: React.ReactNode; done: (now: TourState, base: TourState) => boolean };
};

const TOUR_STEPS: TourStep[] = [
  {
    sel: "arcade",
    tag: "Appchain · L2",
    tone: "primary",
    title: "The Arcade is your appchain",
    body: (
      <>
        Everything you do here executes on a <b>Katana appchain</b> — a dedicated L2 with its own blocks. State lives in a
        Dojo <b>world</b> (typed models), and a <b>Torii</b> indexer mirrors it so this UI can read it. Because it's your
        own chain, playing is instant and free.
      </>
    ),
  },
  {
    sel: "coin",
    tag: "L1 → L2",
    tone: "primary",
    title: "Insert coin — a message from L1",
    action: {
      cta: (
        <>
          Click <b>Insert coin</b> — then watch the credit mint on L2.
        </>
      ),
      done: (now, base) => now.available > base.available,
    },
    body: (
      <TourFlow
        steps={[
          <>
            Calls <Code>buy_game</Code> on the <b>store</b> contract (settlement / L1), which runs the store’s rules then{" "}
            <Code>send_message_to_appchain</Code> on the <b>piltover core</b>, emitting <Code>MessageSent</Code>.
          </>,
          <>
            The appchain (<Code>--messaging.enabled</Code>) relays it as an <b>L1-handler</b> transaction.
          </>,
          <>
            <Code>mint_game</Code> runs on the appchain world and adds a credit. Instant — relayed by Katana, no prover
            needed.
          </>,
        ]}
      />
    ),
  },
  {
    sel: "roll",
    tag: "On L2",
    tone: "primary",
    title: "Roll — play on the appchain",
    action: {
      cta: (
        <>
          Click <b>Roll</b> to play on the appchain.
        </>
      ),
      done: (now, base) => now.playsLen > base.playsLen,
    },
    body: (
      <>
        <Code>play_game()</Code> spends a credit and rolls the score <i>on chain</i>. It writes the <Code>Stats</Code>{" "}
        model, emits <Code>GamePlayed</Code>, and fires <Code>send_message_to_l1</Code> with your score — an outbound
        message addressed to L1.
      </>
    ),
  },
  {
    sel: "rolls",
    tag: "L2 → L1 · pending",
    tone: "muted",
    title: "An unbanked message, waiting to settle",
    body: (
      <>
        Each roll's score is now an L2 → L1 message that exists <b>on the appchain but not yet on L1</b>. L1 can't consume
        it until the block it landed in is <i>settled</i> — which is the prover's job, next.
      </>
    ),
  },
  {
    sel: "settler",
    tag: "Settlement",
    tone: "muted",
    title: "The settler proves & settles the block",
    body: (
      <>
        The <b>settler</b> proves each appchain block and submits <Code>update_state</Code> to the piltover core contract
        (deployed on the L1), which <b>registers</b> the block's outbound message hashes. This gauge shows the settled
        block height vs the appchain tip. Only after settlement can L1 consume your score.
        <span className="mt-2 flex items-start gap-1.5 text-left text-xs text-muted-foreground/80">
          <ShieldCheck className="mt-0.5 size-3.5 shrink-0 text-green-600" />
          <span>
            The proving mode here is <b>TEE</b>, run in <b>mock</b> mode for this demo — a real TEE prover plugs in here
            unchanged.
          </span>
        </span>
      </>
    ),
  },
  {
    sel: "bank",
    tag: "L2 → L1 · consume",
    tone: "green",
    title: "Bank the roll to L1",
    action: {
      cta: (
        <>
          Once it's settled, click <b>Bank</b> to send the roll to L1.
        </>
      ),
      done: (now, base) => now.bankedLen > base.bankedLen,
    },
    body: (
      <>
        <b>Bank</b> calls <Code>claim_score</Code> on the settlement world, which calls{" "}
        <Code>consume_message_from_appchain</Code> on the piltover core — this succeeds only because the settler settled the
        message. The button stays disabled until then. On success the score is stored on L1 and <Code>ScoreClaimed</Code>{" "}
        is emitted.
      </>
    ),
  },
  {
    sel: "vault",
    tag: "On L1 · permanent",
    tone: "green",
    place: "left",
    title: "The Vault is the L1 record",
    body: (
      <>
        Your banked run now lives on L1 — stored in the settlement world's <Code>Leaderboard</Code> model and shown here.
        It's the permanent, settled record of what happened on the appchain. That's the full L1 → L2 → settle → L1 round
        trip.
      </>
    ),
  },
  {
    sel: "hood",
    tag: "Anytime",
    tone: "primary",
    title: "Peek under the hood",
    body: (
      <>
        Open this anytime to see the live plumbing: both Dojo <b>worlds</b>, the <b>piltover core</b>, and each chain's{" "}
        <b>Torii</b> indexer this UI reads from. That's the whole stack — enjoy!
      </>
    ),
  },
];

function TourFlow({ steps }: { steps: React.ReactNode[] }) {
  return (
    <ol className="space-y-1.5">
      {steps.map((s, i) => (
        <li key={i} className="flex gap-2">
          <span className="mt-0.5 grid size-4 shrink-0 place-items-center rounded-full bg-primary/15 text-[9px] font-bold text-primary">
            {i + 1}
          </span>
          <span className="flex-1 leading-snug">{s}</span>
        </li>
      ))}
    </ol>
  );
}

type Rect = { top: number; left: number; width: number; height: number };

function Tour({ step, onStep, state }: { step: number; onStep: (s: number) => void; state: TourState }) {
  const open = step >= 0 && step < TOUR_STEPS.length;
  const current = open ? TOUR_STEPS[step] : null;
  const interactive = !!current?.action && state.online;
  const [rect, setRect] = useState<Rect | null>(null);
  const [popH, setPopH] = useState(0);
  const popRef = useRef<HTMLDivElement>(null);
  const baseRef = useRef<TourState>(state); // game-state snapshot when the step began

  // Snapshot the baseline when the step changes, so an interactive step can tell
  // when the user's real action has landed (a new credit, a new roll, …).
  useEffect(() => {
    baseRef.current = state;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  // Auto-advance an interactive step once its action completes on chain.
  useEffect(() => {
    if (!current?.action || !state.online) return;
    if (current.action.done(state, baseRef.current)) onStep(step + 1);
  }, [state, current, step, onStep]);

  useEffect(() => {
    if (!current) return;
    const sel = current.sel;
    const measure = () => {
      const el = document.querySelector(`[data-tour="${sel}"]`);
      const r = el ? (el.getBoundingClientRect() as DOMRect) : null;
      // Only update when the rect actually moved, so the 250ms poll doesn't
      // re-render (and re-animate) the popover every tick.
      setRect((prev) => {
        if (!r) return prev === null ? prev : null;
        if (prev && Math.abs(prev.top - r.top) < 0.5 && Math.abs(prev.left - r.left) < 0.5 &&
          Math.abs(prev.width - r.width) < 0.5 && Math.abs(prev.height - r.height) < 0.5) return prev;
        return { top: r.top, left: r.left, width: r.width, height: r.height };
      });
    };
    document.querySelector(`[data-tour="${sel}"]`)?.scrollIntoView({ block: "nearest" });
    measure();
    const id = setInterval(measure, 250);
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      clearInterval(id);
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [current]);

  // Measure the popover so we can keep it fully on-screen for tall targets.
  useEffect(() => {
    if (popRef.current) setPopH(popRef.current.offsetHeight);
  }, [step, rect]);

  if (!open || !current) return null;

  const last = step === TOUR_STEPS.length - 1;
  const pad = 10;
  const toneText =
    current.tone === "green" ? "text-green-600" : current.tone === "muted" ? "text-muted-foreground" : "text-primary";
  const toneBg =
    current.tone === "green" ? "bg-green-600/10" : current.tone === "muted" ? "bg-muted" : "bg-primary/10";

  // Position the popover relative to the highlight: to the left if requested,
  // else below/above the target, clamped into the viewport. Centered if gone.
  const W = 340;
  let popStyle: React.CSSProperties;
  let highlight: React.ReactNode = null;
  if (rect) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    if (current.place === "left") {
      // Sit to the left of the target, vertically centered (clamped on-screen).
      const left = Math.max(12, rect.left - pad - 12 - W);
      const top = Math.max(12, Math.min(rect.top + rect.height / 2 - popH / 2, vh - 12 - popH));
      popStyle = { top, left, width: W };
    } else {
      const left = Math.max(12, Math.min(rect.left + rect.width / 2 - W / 2, vw - 12 - W));
      const below = rect.top + rect.height / 2 < vh / 2;
      // Prefer below/above the target, then clamp into the viewport so a tall
      // target (e.g. a full-height card) can't push the popover off-screen.
      const wanted = below ? rect.top + rect.height + pad + 12 : rect.top - pad - 12 - popH;
      const top = Math.max(12, Math.min(wanted, vh - 12 - popH));
      popStyle = { top, left, width: W };
    }
    highlight = (
      <div
        className={cn(
          "pointer-events-none fixed z-[61] rounded-xl ring-2 ring-primary ring-offset-2 ring-offset-background transition-all duration-200",
          interactive && "animate-pulse",
        )}
        style={{
          top: rect.top - pad,
          left: rect.left - pad,
          width: rect.width + pad * 2,
          height: rect.height + pad * 2,
          boxShadow: "0 0 0 9999px rgba(0,0,0,0.55)",
        }}
      />
    );
  } else {
    popStyle = { top: "50%", left: "50%", transform: "translate(-50%, -50%)", width: W };
  }

  return (
    // The container is click-through; only the blocker (non-interactive) and the
    // popover re-enable pointer events. This lets an interactive step's real
    // highlighted button receive the user's click.
    <div className="pointer-events-none fixed inset-0 z-[60]">
      {/* Dim + (for non-interactive steps) swallow page clicks. On interactive
          steps we drop the blocker so the user can click the real highlighted
          element; the box-shadow highlight still dims everything else. */}
      {(!interactive || !rect) && <div className={cn("pointer-events-auto absolute inset-0", !rect && "bg-black/55")} />}
      {highlight}
      <motion.div
        ref={popRef}
        key={step}
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.18 }}
        style={popStyle}
        className="pointer-events-auto fixed z-[62] max-h-[80vh] overflow-y-auto rounded-xl bg-popover p-4 text-sm text-popover-foreground shadow-xl ring-1 ring-foreground/10"
      >
        <div className="mb-2 flex items-center justify-between gap-2">
          <span className={cn("rounded-full px-2 py-0.5 font-mono text-[10px] font-medium", toneBg, toneText)}>
            {current.tag}
          </span>
          <div className="flex items-center gap-2">
            <span className="text-[11px] text-muted-foreground">
              {step + 1} / {TOUR_STEPS.length}
            </span>
            <button
              type="button"
              onClick={() => onStep(-1)}
              aria-label="Close tutorial"
              className="cursor-pointer text-muted-foreground transition-colors hover:text-foreground"
            >
              <X className="size-4" />
            </button>
          </div>
        </div>
        <h3 className="mb-1.5 font-semibold">{current.title}</h3>
        <div className="leading-snug text-muted-foreground [&_b]:text-foreground">{current.body}</div>
        {interactive && (
          <div className="mt-3 flex items-center gap-2 rounded-md bg-primary/10 p-2 text-xs font-medium text-primary [&_b]:text-primary">
            <MousePointerClick className="size-4 shrink-0 animate-pulse" />
            <span>{current.action!.cta}</span>
          </div>
        )}
        <div className="mt-4 flex items-center justify-between">
          <div className="flex gap-1">
            {TOUR_STEPS.map((_, i) => (
              <span
                key={i}
                className={cn("size-1.5 rounded-full transition-colors", i === step ? "bg-primary" : "bg-muted-foreground/30")}
              />
            ))}
          </div>
          <div className="flex gap-2">
            {step > 0 && (
              <Button variant="ghost" size="sm" className="h-7 px-2 text-xs" onClick={() => onStep(step - 1)}>
                <ArrowLeft className="size-3.5" /> Back
              </Button>
            )}
            {/* Interactive steps have no "Next"/"Skip" — the user must perform
                the action, which auto-advances the tour. (The header ✕ still
                exits the whole tutorial.) */}
            {!interactive && (
              <Button size="sm" className="h-7 px-2.5 text-xs" onClick={() => onStep(last ? -1 : step + 1)}>
                {last ? "Done" : "Next"} {!last && <ArrowRight className="size-3.5" />}
              </Button>
            )}
          </div>
        </div>
      </motion.div>
    </div>
  );
}
