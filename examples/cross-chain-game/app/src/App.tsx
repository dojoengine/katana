import { Fragment, useEffect, useState } from "react";
import {
	ArrowLeft,
	ArrowRight,
	Check,
	ChevronRight,
	Dices,
	ExternalLink,
	Info,
	Loader2,
	ShoppingCart,
	Trophy,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
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
	GAME,
	BUYER_ADDRESS,
	PLAYER_ADDRESS,
} from "./chain.ts";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export default function App() {
	const [online, setOnline] = useState(false);
	const [game, setGame] = useState<GameState | null>(null);
	const [score, setScore] = useState<ScoreState | null>(null);
	const [settled, setSettled] = useState(-1);
	const [tip, setTip] = useState(0);

	// Feeds are event-sourced — rebuilt from chain every poll, so they survive a
	// page refresh instead of living only in volatile React state.
	const [purchases, setPurchases] = useState<PurchaseRecord[]>([]);
	const [plays, setPlays] = useState<PlayRecord[]>([]);

	const [buying, setBuying] = useState(false);
	const [rolling, setRolling] = useState(false);
	const [introOpen, setIntroOpen] = useState(true); // intro shown on load
	const [settling, setSettling] = useState<Set<number>>(new Set()); // play seqs being settled to L1

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

	// Explicit settle: consume the score's message on L1 via claim_score. Retries
	// in case saya is still settling the block the score was emitted in.
	async function onSettle(seq: number, sc: number) {
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

	async function onRoll() {
		if (rolling) return;
		setRolling(true);
		try {
			await playGame(); // rolls + publishes the message; reconciler handles the L1 claim
		} catch (e) {
			console.error("roll failed", e);
		} finally {
			setRolling(false);
		}
	}

	const available = game?.available ?? 0;
	const lastPlay = plays.length ? plays[plays.length - 1] : undefined;

	return (
		<TooltipProvider>
			<IntroDialog open={introOpen} onOpenChange={setIntroOpen} />
			<div className="min-h-screen bg-background bg-[radial-gradient(1100px_560px_at_85%_-12%,oklch(0.72_0.13_285/0.16),transparent_58%)] text-foreground">
				<div className="mx-auto max-w-5xl px-5 py-8 pb-16">
					<header className="mb-6 flex flex-wrap items-start justify-between gap-4">
						<div className="flex items-center gap-4">
							<div className="grid size-12 shrink-0 place-items-center rounded-xl bg-primary/15 text-primary">
								<Dices className="size-6" />
							</div>
							<div>
								<h1 className="text-2xl font-semibold tracking-tight">
									Cross-Chain Dice
								</h1>
								<p className="mt-1 max-w-2xl text-sm text-muted-foreground">
									Buy games on L1, play them on the appchain,
									and your score is published back to L1 —
									settlement by{" "}
									<b className="text-foreground">saya</b>.
								</p>
							</div>
						</div>
						<div className="flex flex-col items-end gap-2">
							<div className="flex items-center gap-2">
								<Button
									variant="ghost"
									size="sm"
									className="h-7 gap-1 px-2 text-xs"
									onClick={() => setIntroOpen(true)}
								>
									<Info className="size-3.5" /> About
								</Button>
								<Badge
									variant="outline"
									className="gap-1.5 py-1"
								>
									<span
										className={cn(
											"size-2 rounded-full",
											online
												? "bg-green-500"
												: "bg-amber-500",
										)}
									/>
									{online ? "Connected" : "Connecting…"}
								</Badge>
							</div>
							{online && (
								<Tooltip>
									<TooltipTrigger className="flex cursor-help items-center gap-1.5 text-xs text-muted-foreground decoration-dotted underline-offset-2 hover:underline">
										{settled >= tip ? (
											<Check className="size-3.5 text-green-600" />
										) : (
											<Loader2 className="size-3.5 animate-spin text-primary" />
										)}
										saya: settled{" "}
										<span className="font-mono text-foreground">
											{Math.max(settled, 0)}
										</span>{" "}
										/ tip{" "}
										<span className="font-mono text-foreground">
											{tip}
										</span>
									</TooltipTrigger>
									<TooltipContent className="max-w-xs">
										<div className="space-y-1.5 text-left leading-snug">
											<p>
												<b>settled</b> — the latest
												appchain block <b>saya</b> has
												proved and settled onto the L1
												piltover core.
											</p>
											<p>
												<b>tip</b> — the appchain's
												current block height.
											</p>
											<p>
												A rolled score (L2→L1) only
												becomes publishable on L1 once
												its block is settled, so{" "}
												<b>settled = tip</b> means saya
												is fully caught up.
											</p>
										</div>
									</TooltipContent>
								</Tooltip>
							)}
						</div>
					</header>

					<section className="mb-6 grid items-stretch gap-3 md:grid-cols-[1fr_auto_1fr]">
						<ChainCard
							tag="Settlement · “L1”"
							rpc={SETTLEMENT_RPC}
							explorer={SETTLEMENT_EXPLORER}
							contracts={[
								{ label: "piltover core", addr: PILTOVER },
								{
									label: "score_registry",
									addr: SCORE_REGISTRY,
								},
							]}
						/>
						<div className="flex flex-col items-center justify-center gap-2 px-2 text-center text-muted-foreground">
							<span className="font-mono text-[11px] text-primary">
								buy → mint
							</span>
							<ArrowRight className="size-5 text-primary" />
							<ArrowLeft className="size-5 text-green-600" />
							<span className="font-mono text-[11px] text-green-600">
								score + saya
							</span>
						</div>
						<ChainCard
							tag="Appchain · “L2”"
							rpc={APPCHAIN_RPC}
							explorer={APPCHAIN_EXPLORER}
							contracts={[{ label: "game", addr: GAME }]}
						/>
					</section>

					{/* Phase 1 — Buy */}
					<PhaseCard
						n={1}
						icon={<ShoppingCart className="size-4" />}
						title="Buy games"
						subtitle="L1 → L2 message"
						info={<BuyInfo />}
					>
						<div className="flex flex-wrap items-center gap-3">
							<Button
								onClick={onBuy}
								disabled={buying || !online}
							>
								{buying ? (
									<Loader2 className="size-4 animate-spin" />
								) : (
									<ShoppingCart className="size-4" />
								)}
								{buying ? "Submitting on L1…" : "Buy a game"}
							</Button>
							<span className="text-xs text-muted-foreground">
								buyer{" "}
								<a
									href={explorerAddrUrl(
										SETTLEMENT_EXPLORER,
										BUYER_ADDRESS,
									)}
									target="_blank"
									rel="noreferrer"
									className="font-mono text-primary hover:underline"
								>
									{shortHex(BUYER_ADDRESS)}
								</a>{" "}
								· buy as many as you like
							</span>
						</div>
						{purchases.length > 0 && (
							<div className="mt-4">
								<div className="mb-2 text-xs font-medium text-muted-foreground">
									Purchased games (
									{game ? game.totalMinted : purchases.length}
									)
								</div>
								<div className="flex max-h-[6.5rem] flex-col gap-2 overflow-y-auto py-px pr-1">
									<AnimatePresence initial={false}>
										{[...purchases].reverse().map((p) => (
											<motion.div
												key={p.seq}
												layout
												initial={{
													opacity: 0,
													y: -10,
													scale: 0.98,
												}}
												animate={{
													opacity: 1,
													y: 0,
													scale: 1,
												}}
												exit={{ opacity: 0 }}
												transition={{
													duration: 0.25,
													ease: "easeOut",
												}}
												className="shrink-0"
											>
												<PurchaseRow purchase={p} />
											</motion.div>
										))}
									</AnimatePresence>
								</div>
							</div>
						)}
					</PhaseCard>

					{/* Phase 2 — Play */}
					<PhaseCard
						n={2}
						icon={<Dices className="size-4" />}
						title="Play a game"
						subtitle="on the appchain"
						info={<PlayInfo />}
					>
						<div className="flex flex-col items-center gap-4 py-2 sm:flex-row sm:justify-between">
							<div className="text-center sm:text-left">
								<div className="text-5xl font-bold tabular-nums text-primary">
									{available}
								</div>
								<div className="text-sm text-muted-foreground">
									games available to play
								</div>
							</div>
							<div className="flex flex-col items-center gap-2">
								{lastPlay && (
									<div className="text-center text-sm">
										last roll{" "}
										<b className="text-2xl text-foreground">
											{lastPlay.score}
										</b>
									</div>
								)}
								<Button
									size="lg"
									className="h-14 px-10 text-lg"
									onClick={onRoll}
									disabled={
										rolling || available < 1 || !online
									}
								>
									{rolling ? (
										<Loader2 className="size-5 animate-spin" />
									) : (
										<Dices className="size-5" />
									)}
									{rolling
										? "Playing…"
										: available < 1
											? "No games — buy one above"
											: "🎲 Roll"}
								</Button>
								<span className="text-xs text-muted-foreground">
									played {game ? game.totalPlayed : 0} ·
									player{" "}
									<a
										href={explorerAddrUrl(
											APPCHAIN_EXPLORER,
											PLAYER_ADDRESS,
										)}
										target="_blank"
										rel="noreferrer"
										className="font-mono text-primary hover:underline"
									>
										{shortHex(PLAYER_ADDRESS)}
									</a>
								</span>
							</div>
						</div>
					</PhaseCard>

					{/* Phase 3 — Published to L1 */}
					<PhaseCard
						n={3}
						icon={<Trophy className="size-4" />}
						title="End a game"
						subtitle="L2 → L1 · you settle each score"
						info={<PublishInfo />}
					>
						<div className="mb-4 grid grid-cols-2 gap-3">
							<Stat
								label="Last score on L1"
								value={score ? score.lastPublished : "…"}
								highlight
							/>
							<Stat
								label="Total published"
								value={score ? score.totalPublished : "…"}
							/>
						</div>
						{plays.length === 0 ? (
							<p className="text-sm text-muted-foreground">
								Play a game, then settle its score onto the
								settlement chain here.
							</p>
						) : (
							<div className="flex max-h-[7.25rem] flex-col gap-2 overflow-y-auto py-px pr-1">
								<AnimatePresence initial={false}>
									{[...plays].reverse().map((p) => (
										<motion.div
											key={p.seq}
											layout
											initial={{
												opacity: 0,
												y: -10,
												scale: 0.98,
											}}
											animate={{
												opacity: 1,
												y: 0,
												scale: 1,
											}}
											exit={{ opacity: 0 }}
											transition={{
												duration: 0.25,
												ease: "easeOut",
											}}
											className="shrink-0"
										>
											<PlayRow
												play={p}
												settled={settled}
												settling={settling.has(p.seq)}
												onSettle={() =>
													onSettle(p.seq, p.score)
												}
											/>
										</motion.div>
									))}
								</AnimatePresence>
							</div>
						)}
					</PhaseCard>
				</div>
			</div>
		</TooltipProvider>
	);
}

function IntroDialog({
	open,
	onOpenChange,
}: {
	open: boolean;
	onOpenChange: (o: boolean) => void;
}) {
	const C = ({ children }: { children: React.ReactNode }) => (
		<code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
			{children}
		</code>
	);
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Dices className="size-5 text-primary" /> Cross-Chain
						Dice
					</DialogTitle>
					<DialogDescription>
						A hands-on demo of building an app as a Starknet
						appchain — and wiring up cross-chain messaging.
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 text-sm">
					<p>
						It runs two Katana nodes — a <b>settlement layer</b>{" "}
						(“L1”) and a rollup <b>appchain</b> (“L2”) that settles
						to it via <b>saya</b>. The dice game is just a vehicle
						to show how the two chains coordinate through{" "}
						<b>messages</b>.
					</p>
					<div className="space-y-2.5">
						<div className="flex items-start gap-2.5">
							<ArrowRight className="mt-0.5 size-4 shrink-0 text-primary" />
							<p>
								<b className="text-primary">
									L1 → L2 — buy &amp; mint.
								</b>{" "}
								Buying a game calls{" "}
								<C>send_message_to_appchain</C>; Katana relays
								it into the appchain’s <C>mint_game</C> handler.
							</p>
						</div>
						<div className="flex items-start gap-2.5">
							<ArrowLeft className="mt-0.5 size-4 shrink-0 text-green-600" />
							<p>
								<b className="text-green-600">
									L2 → L1 — play, then settle.
								</b>{" "}
								Playing rolls a score and emits{" "}
								<C>send_message_to_l1</C>; saya settles the
								block, then <i>you</i> settle it to L1 —{" "}
								<C>score_registry</C> consumes it via{" "}
								<C>consume_message_from_appchain</C>.
							</p>
						</div>
					</div>
					<p className="text-muted-foreground">
						Use it to understand the messaging system end to end —
						and as a reference for designing your own message-driven
						appchain app or game. Click any message card to trace
						its full lifecycle and the transactions on each chain.
					</p>
				</div>
				<Button
					className="w-full cursor-pointer"
					onClick={() => onOpenChange(false)}
				>
					Got it — let’s play
				</Button>
			</DialogContent>
		</Dialog>
	);
}

function PhaseCard(props: {
	n: number;
	icon: React.ReactNode;
	title: string;
	subtitle: string;
	info?: React.ReactNode;
	children: React.ReactNode;
}) {
	const [infoOpen, setInfoOpen] = useState(false);
	return (
		<Card className="relative mb-4 py-5">
			<CardContent className="px-5">
				<div className="mb-4 flex items-center gap-3">
					<span className="grid size-7 shrink-0 place-items-center rounded-full bg-primary/15 text-sm font-bold text-primary">
						{props.n}
					</span>
					<span className="text-primary">{props.icon}</span>
					<h2 className="text-base font-semibold">{props.title}</h2>
					<Badge variant="secondary" className="text-[10px]">
						{props.subtitle}
					</Badge>
					{props.info && (
						<>
							<button
								type="button"
								onClick={() => setInfoOpen(true)}
								aria-label={`How “${props.title}” works`}
								className="absolute top-4 right-4 inline-flex cursor-pointer text-muted-foreground transition-colors hover:text-foreground"
							>
								<Info className="size-4" />
							</button>
							<Dialog open={infoOpen} onOpenChange={setInfoOpen}>
								<DialogContent className="max-w-lg sm:max-w-lg">
									<DialogHeader>
										<DialogTitle className="flex items-center gap-2">
											<span className="text-primary">
												{props.icon}
											</span>{" "}
											{props.title}
										</DialogTitle>
										<DialogDescription>
											{props.subtitle}
										</DialogDescription>
									</DialogHeader>
									{props.info}
								</DialogContent>
							</Dialog>
						</>
					)}
				</div>
				{props.children}
			</CardContent>
		</Card>
	);
}

function Code({ children }: { children: React.ReactNode }) {
	return (
		<code className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
			{children}
		</code>
	);
}

/** Reusable "how this step works" body: services involved + numbered on-chain steps. */
function StepInfo(props: {
	services: string[];
	steps: React.ReactNode[];
	note: React.ReactNode;
}) {
	return (
		<div className="space-y-4 text-sm">
			<div>
				<div className="mb-1.5 text-xs font-medium tracking-wide text-muted-foreground uppercase">
					Services involved
				</div>
				<div className="flex flex-wrap gap-1.5">
					{props.services.map((s) => (
						<Badge
							key={s}
							variant="secondary"
							className="font-mono text-[11px]"
						>
							{s}
						</Badge>
					))}
				</div>
			</div>
			<div>
				<div className="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
					On-chain flow
				</div>
				<ol className="space-y-2.5">
					{props.steps.map((s, i) => (
						<li key={i} className="flex gap-2.5">
							<span className="grid size-5 shrink-0 place-items-center rounded-full bg-primary/15 text-[10px] font-bold text-primary">
								{i + 1}
							</span>
							<div className="flex-1 leading-snug">{s}</div>
						</li>
					))}
				</ol>
			</div>
			<p className="rounded-md bg-muted/60 p-2.5 text-xs text-muted-foreground">
				{props.note}
			</p>
		</div>
	);
}

function BuyInfo() {
	return (
		<StepInfo
			services={[
				"settlement katana (L1)",
				"appchain katana — messaging service",
			]}
			steps={[
				<>
					The buyer account calls{" "}
					<Code>
						send_message_to_appchain(game, mint_game, [game_id])
					</Code>{" "}
					on the <b>piltover core</b> contract (L1), which emits a{" "}
					<Code>MessageSent</Code> event.
				</>,
				<>
					The appchain runs with <Code>--messaging.enabled</Code>; its
					messaging service polls the settlement chain, picks up{" "}
					<Code>MessageSent</Code>, and submits an <b>L1-handler</b>{" "}
					transaction to the appchain.
				</>,
				<>
					<Code>mint_game(from_address, game_id)</Code> runs on the{" "}
					<b>game</b> contract (L2), growing the playable pool and
					emitting <Code>GameMinted</Code>.
				</>,
			]}
			note={
				<>
					Evaluated as <b>minted</b> once the appchain executes the L1
					handler (the <Code>GameMinted</Code> event). No prover is
					involved — this direction is relayed by Katana directly.
				</>
			}
		/>
	);
}

function PlayInfo() {
	return (
		<StepInfo
			services={["appchain katana (L2)"]}
			steps={[
				<>
					The player account calls <Code>play_game()</Code> on the{" "}
					<b>game</b> contract (L2).
				</>,
				<>
					It asserts <Code>available &gt; 0</Code>, consumes one game,
					and rolls a score on chain —{" "}
					<Code>poseidon(block_timestamp, play_no) % 100 + 1</Code> —
					storing <Code>last_score</Code> and emitting{" "}
					<Code>GamePlayed</Code>.
				</>,
				<>
					In the <i>same</i> transaction it emits the score to L1 via{" "}
					<Code>
						send_message_to_l1(score_registry, [player, score])
					</Code>{" "}
					— picked up in step 3.
				</>,
			]}
			note={
				<>
					Evaluated entirely on the appchain: the game finishes the
					instant the tx is mined, and the score is the on-chain roll.
					The L1 side happens in the next phase.
				</>
			}
		/>
	);
}

function PublishInfo() {
	return (
		<StepInfo
			services={[
				"appchain katana (L2)",
				"saya-tee — prover / settler",
				"settlement katana (L1)",
			]}
			steps={[
				<>
					<Code>play_game</Code> recorded an L2→L1 message in the
					appchain block (via <Code>send_message_to_l1</Code>).
				</>,
				<>
					<b>saya-tee</b> polls the appchain, proves each block (mock
					proof here), and submits <Code>update_state</Code> to the{" "}
					<b>piltover core</b> (L1), which registers the message
					hashes via <Code>process_messages_to_starknet</Code>.
					Settlement advances block-by-block (the{" "}
					<i>saya: settled / tip</i> indicator).
				</>,
				<>
					Once saya has settled the block, you click{" "}
					<b>Settle to L1</b>, which calls{" "}
					<Code>claim_score(game, player, score)</Code> on{" "}
					<b>score_registry</b> (L1). It calls{" "}
					<Code>consume_message_from_appchain</Code> on the piltover
					core, stores the score, and emits <Code>ScoreClaimed</Code>.
				</>,
			]}
			note={
				<>
					Settling is an <b>explicit step</b> — playing only emits the
					message; you publish it to L1 yourself. The{" "}
					<b>Settle to L1</b> button unlocks once saya has settled the
					block the score was emitted in.
				</>
			}
		/>
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
					<Badge
						variant="secondary"
						className="text-[10px] tracking-wide uppercase"
					>
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
					<dt className="text-[11px] tracking-wide text-muted-foreground uppercase">
						RPC
					</dt>
					<dd className="font-mono text-[13px] break-all">
						{props.rpc}
					</dd>
					{props.contracts.map((c) => (
						<div key={c.label}>
							<dt className="mt-2 text-[11px] tracking-wide text-muted-foreground uppercase">
								{c.label}
							</dt>
							<dd className="font-mono text-[13px] break-all">
								{c.addr ? (
									<a
										href={explorerAddrUrl(
											props.explorer,
											c.addr,
										)}
										target="_blank"
										rel="noreferrer"
										className="inline-flex items-center gap-1 text-primary hover:underline"
									>
										{shortHex(c.addr, 10, 6)}{" "}
										<ExternalLink className="size-3" />
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

function Stat(props: {
	label: string;
	value: React.ReactNode;
	highlight?: boolean;
	tip?: string;
}) {
	return (
		<div
			className={cn(
				"rounded-lg border p-3 text-center",
				props.highlight && "border-green-600/40 bg-green-600/10",
			)}
		>
			<div
				className={cn(
					"text-2xl font-bold tabular-nums",
					props.highlight && "text-green-600",
				)}
			>
				{props.value}
			</div>
			<div className="mt-0.5 flex items-center justify-center gap-1 text-xs text-muted-foreground">
				{props.label}
				{props.tip && (
					<Tooltip>
						<TooltipTrigger
							aria-label={`What does “${props.label}” mean?`}
							className="inline-flex text-muted-foreground transition-colors hover:text-foreground"
						>
							<Info className="size-3.5" />
						</TooltipTrigger>
						<TooltipContent>{props.tip}</TooltipContent>
					</Tooltip>
				)}
			</div>
		</div>
	);
}

function TxLink(props: {
	label: string;
	href: string;
	hash: string;
	tone: "l1" | "l2";
}) {
	const tone =
		props.tone === "l1"
			? "border-primary/30 bg-primary/10 text-primary hover:bg-primary/20"
			: "border-green-600/30 bg-green-600/10 text-green-600 hover:bg-green-600/20";
	return (
		<a
			href={props.href}
			target="_blank"
			rel="noreferrer"
			title={props.hash}
			className={cn(
				"inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono text-xs transition-colors",
				tone,
			)}
		>
			{props.label} {shortHex(props.hash)}{" "}
			<ExternalLink className="size-3" />
		</a>
	);
}

function Step(props: {
	done: boolean;
	active: boolean;
	label: string;
	grow?: boolean;
}) {
	return (
		<div
			className={cn(
				"flex items-center gap-1.5 text-xs",
				props.grow === false ? "shrink-0" : "flex-1",
				props.done || props.active
					? "text-foreground"
					: "text-muted-foreground",
			)}
		>
			<span
				className={cn(
					"grid size-5 shrink-0 place-items-center rounded-full border text-[10px]",
					props.done && "border-green-600 bg-green-600 text-white",
					props.active && "border-primary text-primary",
				)}
			>
				{props.done ? (
					<Check className="size-3" />
				) : props.active ? (
					<Loader2 className="size-3 animate-spin" />
				) : null}
			</span>
			{props.label}
		</div>
	);
}

/** A long connector arrow that grows to fill the gap between two phases
 *  (line + arrowhead, tip to tip). `lit` colors it as a completed transition. */
function FlowArrow({ lit }: { lit?: boolean }) {
	return (
		<div
			className={cn(
				"flex flex-1 items-center",
				lit ? "text-green-600" : "text-muted-foreground/40",
			)}
			aria-hidden
		>
			<span className="h-0.5 flex-1 rounded-full bg-current" />
			<ChevronRight className="-ml-2 size-4 shrink-0" strokeWidth={3} />
		</div>
	);
}

type StepState = "done" | "active" | "pending";
type FlowStep = {
	label: string;
	description: string;
	state: StepState;
	tx?: { label: string; hash: string; href: string; tone: "l1" | "l2" };
};
type FlowSpec = {
	title: string;
	direction: string;
	status: string;
	done: boolean;
	steps: FlowStep[];
	action?: {
		label: string;
		onClick: () => void;
		disabled: boolean;
		busy: boolean;
	};
	// When true, the card shows a single status icon (spinner → check) instead of
	// the full stepper. The detailed flow still lives in the modal.
	simple?: boolean;
};

/** Render text with `backtick`-wrapped tokens as inline <code>. */
function withCode(text: string) {
	return text.split("`").map((part, i) =>
		i % 2 === 1 ? (
			<code
				key={i}
				className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]"
			>
				{part}
			</code>
		) : (
			<Fragment key={i}>{part}</Fragment>
		),
	);
}

/** A clickable message card: compact stepper on the card, full flow + tx hashes
 *  in a modal. Shared by both the purchase ("game") and the play ("score") feeds. */
function MessageCard({ spec }: { spec: FlowSpec }) {
	const [open, setOpen] = useState(false);
	return (
		<>
			<Card
				role="button"
				tabIndex={0}
				onClick={() => setOpen(true)}
				onKeyDown={(e) =>
					(e.key === "Enter" || e.key === " ") &&
					(e.preventDefault(), setOpen(true))
				}
				className={cn(
					"shrink-0 cursor-pointer gap-0 border-l-4 py-1.5 transition-colors hover:bg-muted/50",
					spec.simple ? "rounded-full" : "",
					spec.done ? "border-l-green-500" : "border-l-amber-500",
				)}
			>
				<CardContent className="flex items-center gap-3 px-3">
					<div
						className={cn(
							"flex shrink-0 items-center",
							spec.simple ? "pl-2" : "w-28",
						)}
					>
						<span className="text-xs font-semibold">
							{spec.title}
						</span>
					</div>
					<div
						className={cn(
							"flex flex-1 items-center gap-2",
							spec.simple && "justify-end",
						)}
					>
						{spec.simple ? (
							spec.done ? (
								<span className="grid size-4 place-items-center rounded-full bg-green-600 text-white">
									<Check className="size-2.5" />
								</span>
							) : (
								<Loader2 className="size-3.5 animate-spin text-primary" />
							)
						) : (
							spec.steps.map((s, i) => (
								<Fragment key={s.label}>
									{i > 0 && (
										<FlowArrow
											lit={
												spec.steps[i - 1].state ===
												"done"
											}
										/>
									)}
									<Step
										grow={false}
										label={s.label}
										done={s.state === "done"}
										active={s.state === "active"}
									/>
								</Fragment>
							))
						)}
					</div>
					{spec.action && (
						<Button
							size="sm"
							className="h-7 shrink-0 cursor-pointer gap-1 px-2.5 text-xs"
							disabled={spec.action.disabled}
							onClick={(e) => {
								e.stopPropagation();
								spec.action!.onClick();
							}}
						>
							{spec.action.busy ? (
								<Loader2 className="size-3.5 animate-spin" />
							) : (
								<ArrowLeft className="size-3.5" />
							)}
							{spec.action.label}
						</Button>
					)}
					{!spec.simple && (
						<ChevronRight className="size-4 shrink-0 text-muted-foreground" />
					)}
				</CardContent>
			</Card>

			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle className="flex items-center gap-2">
							{spec.title}
							<Badge variant="secondary" className="text-[10px]">
								{spec.direction}
							</Badge>
						</DialogTitle>
						<DialogDescription>
							Cross-chain message ·{" "}
							<span
								className={
									spec.done
										? "text-green-600"
										: "text-amber-600"
								}
							>
								{spec.status}
							</span>
						</DialogDescription>
					</DialogHeader>
					<ol className="flex flex-col">
						{spec.steps.map((s, i) => (
							<li key={s.label} className="flex gap-3">
								<div className="flex flex-col items-center">
									<span
										className={cn(
											"grid size-6 shrink-0 place-items-center rounded-full border text-[10px]",
											s.state === "done" &&
												"border-green-600 bg-green-600 text-white",
											s.state === "active" &&
												"border-primary text-primary",
											s.state === "pending" &&
												"border-border text-muted-foreground",
										)}
									>
										{s.state === "done" ? (
											<Check className="size-3.5" />
										) : s.state === "active" ? (
											<Loader2 className="size-3.5 animate-spin" />
										) : (
											i + 1
										)}
									</span>
									{i < spec.steps.length - 1 && (
										<span
											className={cn(
												"my-1 w-px flex-1",
												s.state === "done"
													? "bg-green-600"
													: "bg-border",
											)}
										/>
									)}
								</div>
								<div className="flex-1 pb-4">
									<div className="text-sm font-medium">
										{s.label}
									</div>
									<div className="mt-0.5 text-xs text-muted-foreground">
										{withCode(s.description)}
									</div>
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
		</>
	);
}

function PurchaseRow({ purchase }: { purchase: PurchaseRecord }) {
	const minted = !!purchase.mintTxHash;
	const spec: FlowSpec = {
		title: `Game #${purchase.seq}`,
		direction: "L1 → L2",
		status: minted ? "Minted" : "Relaying",
		done: minted,
		simple: true,
		steps: [
			{
				label: "L1 message sent",
				state: "done",
				description:
					"`send_message_to_appchain` was called on the piltover core, emitting a `MessageSent` event on the settlement layer.",
				tx: {
					label: "L1 tx",
					tone: "l1",
					hash: purchase.l1TxHash,
					href: explorerTxUrl(SETTLEMENT_EXPLORER, purchase.l1TxHash),
				},
			},
			{
				label: "Katana relaying",
				state: minted ? "done" : "active",
				description:
					"Katana's messaging service picks up the event and submits it to the appchain as an L1-handler transaction.",
			},
			{
				label: "Game minted",
				state: minted ? "done" : "pending",
				description:
					"`mint_game` runs on the appchain, adding the game to the playable pool.",
				tx: purchase.mintTxHash
					? {
							label: "L2 tx",
							tone: "l2",
							hash: purchase.mintTxHash,
							href: explorerTxUrl(
								APPCHAIN_EXPLORER,
								purchase.mintTxHash,
							),
						}
					: undefined,
			},
		],
	};
	return <MessageCard spec={spec} />;
}

function PlayRow({
	play,
	settled,
	settling,
	onSettle,
}: {
	play: PlayRecord;
	settled: number;
	settling: boolean;
	onSettle: () => void;
}) {
	const published = !!play.claimTxHash;
	const sayaSettled = settled >= play.block; // saya has settled the block the score was emitted in
	const spec: FlowSpec = {
		title: `🎲 Score ${play.score}`,
		direction: "L2 → L1",
		status: published
			? "Published"
			: settling
				? "Settling…"
				: sayaSettled
					? "Ready to settle"
					: "Awaiting saya",
		done: published,
		steps: [
			{
				label: "Rolled on L2",
				state: "done",
				description:
					"`play_game` rolled the score on the appchain and emitted it to L1 via `send_message_to_l1`.",
				tx: {
					label: "L2 tx",
					tone: "l2",
					hash: play.l2TxHash,
					href: explorerTxUrl(APPCHAIN_EXPLORER, play.l2TxHash),
				},
			},
			{
				label: "Settled by saya",
				state: published || sayaSettled ? "done" : "pending",
				description:
					"saya proves the appchain block and submits `update_state` to the piltover core, registering the message.",
			},
			{
				label: "Published to L1",
				state: published ? "done" : settling ? "active" : "pending",
				description:
					"You settle the game: `score_registry` consumes the message via `consume_message_from_appchain` and records the score.",
				tx: play.claimTxHash
					? {
							label: "L1 tx",
							tone: "l1",
							hash: play.claimTxHash,
							href: explorerTxUrl(
								SETTLEMENT_EXPLORER,
								play.claimTxHash,
							),
						}
					: undefined,
			},
		],
		action: published
			? undefined
			: {
					label: settling
						? "Settling…"
						: sayaSettled
							? "Settle to L1"
							: "Awaiting saya…",
					onClick: onSettle,
					disabled: settling || !sayaSettled,
					busy: settling,
				},
	};
	return <MessageCard spec={spec} />;
}
