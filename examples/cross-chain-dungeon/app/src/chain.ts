// Frontend data layer for cross-chain-dungeon.
//
// Settlement = real Starknet Sepolia: piltover core, the GAME_TOKEN/TokenSale/
// Entry contracts, and the `score` Dojo world. The settlement account (operator
// by default, or a Cartridge Controller) signs buy / enter / bank.
// Appchain = local Katana rollup: the `game` dungeon world. The appchain dev
// account signs the per-action play txns (move/attack/loot/use/extract).
//
// Writes go through starknet.js; reads come from Torii SQL (model rows for live
// state, event-message tables for the action + bank feeds) and a few raw RPC
// reads (token balances, piltover settled height, appchain tip).

import { Account, type AccountInterface, BlockTag, CallData, RpcProvider, TransactionFinalityStatus, cairo, ec, hash } from "starknet";
import { ToriiClient } from "@dojoengine/torii-wasm";
import deployments from "./deployments.json";

// Settlement network: Sepolia by default, or mainnet — set via SETTLEMENT_NETWORK at
// deploy time and recorded in deployments.json. Everything below is network-agnostic.
export const SETTLEMENT_NETWORK = deployments.settlement.network; // "sepolia" | "mainnet"
export const SETTLEMENT_CHAIN_ID = deployments.settlement.chainId; // "SN_SEPOLIA" | "SN_MAIN"
export const SETTLEMENT_NAME = SETTLEMENT_NETWORK === "mainnet" ? "Starknet Mainnet" : "Starknet Sepolia";

export const SETTLEMENT_RPC = deployments.settlement.rpcUrl;
export const APPCHAIN_RPC = deployments.appchain.rpcUrl;
export const SETTLEMENT_EXPLORER = deployments.settlement.explorer;
export const APPCHAIN_EXPLORER = deployments.appchain.explorer;

export const TORII_BANK = deployments.settlement.torii; // settlement: bank world
export const TORII_GAME = deployments.appchain.torii; // appchain: game world

export const PILTOVER = deployments.settlement.piltover;
export const USDC = deployments.settlement.usdc;
export const GAME_TOKEN = deployments.settlement.gameToken; // entry credit (L1)
export const GOLD_TOKEN = deployments.settlement.goldToken; // winnings, minted on bank (L1)
export const TOKEN_SALE = deployments.settlement.tokenSale;
export const ENTRY = deployments.settlement.entry;
export const BANK_SYSTEM = deployments.settlement.bankSystem; // L1 bank (consumes withdrawal)
export const GAME_SYSTEM = deployments.appchain.gameSystem;
export const BANK_WORLD = deployments.settlement.bankWorld; // Sepolia bank world
export const GAME_WORLD = deployments.appchain.gameWorld; // appchain game world

// Decimals: GAME + GOLD are standard OZ ERC20s (18); USDC on Sepolia is 6.
export const GAME_DECIMALS = 18;
export const GOLD_DECIMALS = 18;
export const USDC_DECIMALS = 6;

const num = (v: string | number): number => (typeof v === "number" ? v : Number(BigInt(v)));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const settlementProvider = new RpcProvider({ nodeUrl: SETTLEMENT_RPC });
const appchainProvider = new RpcProvider({ nodeUrl: APPCHAIN_RPC });

// starknet.js's waitForTransaction defaults to a 5s poll interval, so a tx that's
// actually confirmed in well under a second still blocks for one or more 5s polls.
// Poll fast on both chains: Sepolia confirms in ~1-2s (measured).
//
// The appchain mines on a 5s interval (--block-time), so ACCEPTED_ON_L2 (mined into a
// block) is up to 5s away — far too slow for click-to-click play. The appchain is a
// local, trusted rollup, so we resolve play actions on PRE_CONFIRMED instead: the tx
// has executed and its state writes are live in the pre-confirmed block immediately,
// well before the block is sealed. successStates also lists the accepted states so a
// tx that's already mined still resolves.
const APPCHAIN_TX_WAIT = {
  retryInterval: 200,
  successStates: [
    TransactionFinalityStatus.PRE_CONFIRMED,
    TransactionFinalityStatus.ACCEPTED_ON_L2,
    TransactionFinalityStatus.ACCEPTED_ON_L1,
  ],
};
const SETTLEMENT_TX_WAIT = { retryInterval: 1000 };

// Default signers. The operator is a real funded Sepolia account (from
// deployments.json); the wallet layer can swap a Controller in for L1 ops. The
// appchain account is the rollup dev account and always signs the play actions.
export const operatorAccount = new Account({
  provider: settlementProvider,
  address: deployments.settlement.account.address,
  signer: deployments.settlement.account.privateKey,
  cairoVersion: "1",
});
export const appchainAccount = new Account({
  provider: appchainProvider,
  address: deployments.appchain.account.address,
  signer: deployments.appchain.account.privateKey,
  cairoVersion: "1",
});

/** Anything that can submit a tx (starknet.js Account or a Controller account). */
// Minimal signer: anything that can submit a tx — a starknet.js Account (dev keys),
// or a Controller account wrapped to target a specific chain (see wallet.tsx).
export type Signer = Pick<AccountInterface, "execute">;

// Buy / enter / bank are all signed by the same settlement account, so serialize
// them through a promise-chain mutex to avoid racing the nonce.
let settlementQueue: Promise<unknown> = Promise.resolve();
function withSettlementLock<T>(fn: () => Promise<T>): Promise<T> {
  const run = settlementQueue.then(fn, fn);
  settlementQueue = run.then(() => {}, () => {});
  return run;
}

/** Canonicalize an address for a Torii `WHERE` clause. Torii stores addresses lowercase
 *  and 64-hex zero-padded, and compares them as exact strings — so a mixed-case address
 *  (e.g. from deployments.json) or one whose wallet form drops a leading zero byte
 *  (any address < 2^248, like a Controller at 0x00cce9…) silently matches NOTHING.
 *  Normalize via BigInt so any input format maps to Torii's canonical form. */
function toriiAddr(a: string): string {
  return "0x" + BigInt(a).toString(16).padStart(64, "0");
}

async function toriiSql<T = Record<string, string | number>>(base: string, sql: string): Promise<T[]> {
  const res = await fetch(`${base}/sql?query=${encodeURIComponent(sql)}`);
  if (!res.ok) {
    const body = await res.text();
    // Torii creates an event table lazily — only on the first event of that type. Until
    // then a query against it 400s with "no such table", which for this poll-and-derive
    // UI just means "nothing emitted yet". Treat that as an empty result so a freshly
    // deployed world (no banks / withdrawals / actions) doesn't spam errors.
    if (res.status === 400 && /no such table/i.test(body)) return [] as T[];
    throw new Error(`torii sql ${res.status}: ${body}`);
  }
  return (await res.json()) as T[];
}

/**
 * Subscribe to live updates from both Torii worlds (game on the appchain, bank
 * on Sepolia) and call `onUpdate` whenever a model is set or an event is emitted.
 * This replaces fixed-interval Torii polling: the UI refetches only when there's
 * new data. We pass `null` clauses (every entity/event) and ignore the payload —
 * the caller re-reads via the typed SQL helpers, keeping the cross-table logic in
 * one place. Returns an async cleanup that cancels the subscriptions. Throws if a
 * client can't connect (e.g. the stack is down or worlds aren't deployed yet) —
 * the caller keeps a slow poll as a safety net (and for the RPC-only reads).
 */
async function subscribeWorld(toriiUrl: string, worldAddress: string, onUpdate: () => void): Promise<() => void> {
  // NB: despite its `.d.ts`, the wasm `ToriiClient` constructor is async — it
  // returns a Promise that resolves to the connected client. Each client is a full
  // torii-wasm instance, so we connect only the worlds the current view needs.
  const client = await (new ToriiClient({ toriiUrl, worldAddress }) as unknown as Promise<ToriiClient>);
  const subs = [
    await client.onEntityUpdated(null, null, () => onUpdate()),
    await client.onEventMessageUpdated(null, null, () => onUpdate()),
  ];
  return () => {
    for (const s of subs) {
      try {
        s.cancel();
      } catch {
        // already gone — fine
      }
    }
    try {
      client.free();
    } catch {
      // already freed — fine
    }
  };
}

/** Game world (appchain): runs, leaderboard, action feed — shown even when nothing is
 *  connected, so this is always subscribed. */
export const subscribeGameTorii = (onUpdate: () => void) => subscribeWorld(TORII_GAME, GAME_WORLD, onUpdate);

/** Bank world (Sepolia): withdrawals/banks — per-player, so only subscribed once a
 *  wallet is connected. Keeps the idle starting page to a single torii-wasm client. */
export const subscribeBankTorii = (onUpdate: () => void) => subscribeWorld(TORII_BANK, BANK_WORLD, onUpdate);

function parseEventId(internalEventId: string): { block: number; txHash: string } {
  const [blockHex, txHash] = internalEventId.split(":");
  return { block: num(blockHex), txHash };
}

export function explorerTxUrl(base: string, txHash: string): string {
  return `${base}/tx/${txHash}`;
}
export function shortHex(value: string, lead = 6, tail = 4): string {
  if (!value) return "—";
  if (value.length <= lead + tail + 2) return value;
  return `${value.slice(0, lead)}…${value.slice(-tail)}`;
}
/** Address as `0x` + first 3 and last 3 hex digits, after trimming leading zeros. */
export function shortAddr(value: string): string {
  if (!value) return "—";
  const hex = value.replace(/^0x0*/i, "");
  if (hex === "") return "0x0";
  if (hex.length <= 6) return `0x${hex}`;
  return `0x${hex.slice(0, 3)}…${hex.slice(-3)}`;
}

// --- room kinds (mirror cairo/game) ---
export const ROOM = ["entrance", "monster", "treasure", "trap", "shrine", "empty"] as const;
export function roomLabel(kind: number): string {
  return (ROOM[kind] ?? "unknown").toUpperCase();
}

// --- live run state (game world `RunState`, keyed by run_no) ---
// Runs are keyed by a unique run_no, so a player can have many unfinished runs at
// once. `readRun` reads one by id; `listRuns` lists a player's still-alive runs.

export type RunState = {
  runNo: number;
  alive: boolean;
  depth: number;
  hp: number;
  maxHp: number;
  gold: number;
  roomKind: number;
  enemyHp: number;
  potions: number;
};

const RUN_COLS = "run_no, alive, depth, hp, max_hp, gold, room_kind, enemy_hp, potions";
const toRun = (r: Record<string, string | number>): RunState => ({
  runNo: num(r.run_no),
  alive: !!num(r.alive),
  depth: num(r.depth),
  hp: num(r.hp),
  maxHp: num(r.max_hp),
  gold: num(r.gold),
  roomKind: num(r.room_kind),
  enemyHp: num(r.enemy_hp),
  potions: num(r.potions),
});

// Torii stores a u64 `#[key]` as a 0x-prefixed, 16-hex-digit text string
// (e.g. run_no 1 → "0x0000000000000001"), so an integer `WHERE run_no = 1`
// never matches — the key must be compared as that exact hex string.
const runKey = (runNo: number) => `0x${runNo.toString(16).padStart(16, "0")}`;

/** A single run by id; null only if it doesn't exist. An ended run is still
 *  returned (with `alive: false`) so the caller can detect the transition and
 *  show the outcome screen — filtering it out here would hide death/extract. */
export async function readRun(runNo: number): Promise<RunState | null> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT ${RUN_COLS} FROM "game-RunState" WHERE run_no = "${runKey(runNo)}"`,
  );
  const r = rows[0];
  return r ? toRun(r) : null;
}

/** A player's still-unfinished runs, newest first — the dungeon "lobby" list. */
export async function listRuns(player: string): Promise<RunState[]> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT ${RUN_COLS} FROM "game-RunState" WHERE player = "${toriiAddr(player)}" AND alive = 1 ORDER BY run_no DESC`,
  );
  return rows.map(toRun);
}

export type Stats = { totalRuns: number; activeRuns: number; totalActions: number; totalBanked: number };
export async function readStats(): Promise<Stats> {
  const rows = await toriiSql(TORII_GAME, 'SELECT total_runs, active_runs, total_actions, total_banked FROM "game-Stats" WHERE id = 0');
  const r = rows[0];
  if (!r) return { totalRuns: 0, activeRuns: 0, totalActions: 0, totalBanked: 0 };
  return {
    totalRuns: num(r.total_runs),
    activeRuns: num(r.active_runs),
    totalActions: num(r.total_actions),
    totalBanked: num(r.total_banked),
  };
}

// --- run-outcome feed (game world `RunEnded`) — the global message log ---
// One entry per run ending (death or extract) across ALL players, newest first.
// `end_no` is a global monotonic sequence, so it doubles as the feed ordering + id.

export type OutcomeRow = {
  endNo: number;
  runNo: number;
  player: string; // the run's player (will become a Controller username once integrated)
  died: boolean;
  loot: number; // gold forfeited (death) or banked to the vault (extract)
  depth: number;
  block: number;
  txHash: string;
  ts: number; // when the run finished (ms epoch, from the run-ending block's timestamp)
};

export async function getOutcomeFeed(limit = 40): Promise<OutcomeRow[]> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT end_no, run_no, player, score, loot, died, internal_event_id, internal_executed_at FROM "game-RunEnded" ORDER BY end_no DESC LIMIT ${limit}`,
  );
  return rows.map((r) => {
    const { block, txHash } = parseEventId(String(r.internal_event_id));
    const score = num(r.score);
    const loot = num(r.loot);
    const died = !!num(r.died);
    // score = DEPTH_WEIGHT*depth + (gold on extract, 0 on death) — so derive depth.
    const depth = Math.round((died ? score : score - loot) / DEPTH_WEIGHT);
    return {
      endNo: num(r.end_no),
      runNo: num(r.run_no),
      player: String(r.player),
      died,
      loot,
      depth,
      block,
      txHash,
      ts: Date.parse(String(r.internal_executed_at)),
    };
  });
}

// --- leaderboard (game world, on the appchain) ---

// The leaderboard lives entirely on L2: one row per player (`game-Leaderboard`),
// ranked by their best run score (DEPTH_WEIGHT * depth + gold). Banking happens on
// L1 and doesn't touch this board.
export type LeaderRow = { player: string; bestScore: number; runs: number; totalGold: number };
export async function readLeaderboard(limit = 10): Promise<LeaderRow[]> {
  const rows = await toriiSql<Record<string, string>>(
    TORII_GAME,
    `SELECT player, best_score, runs, total_gold FROM "game-Leaderboard" ORDER BY best_score DESC LIMIT ${limit}`,
  );
  return rows.map((r) => ({
    player: r.player,
    bestScore: num(r.best_score),
    runs: num(r.runs),
    totalGold: num(r.total_gold),
  }));
}

// --- vault (accumulated GOLD on L2, awaiting bank) ---

/** The player's unbanked GOLD accumulated on the appchain (0 if none). */
export async function readVault(player: string): Promise<number> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT gold FROM "game-Vault" WHERE player = "${toriiAddr(player)}"`,
  );
  return num(rows[0]?.gold ?? 0);
}

// --- withdrawal / bank reconciliation (drives the "Bank" step) ---

export type WithdrawalRow = { withdrawNo: number; amount: number; block: number; txHash: string };

/** This player's withdrawals (vault → L1), oldest-first. Each needs a `bank` on
 *  Sepolia once saya has settled the block it landed in. */
export async function getWithdrawals(player: string): Promise<WithdrawalRow[]> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT withdraw_no, amount, internal_event_id FROM "game-Withdrawal" WHERE player = "${toriiAddr(player)}" ORDER BY withdraw_no`,
  );
  return rows.map((r) => {
    const { block, txHash } = parseEventId(String(r.internal_event_id));
    return { withdrawNo: num(r.withdraw_no), amount: num(r.amount), block, txHash };
  });
}

/** The L2→L1 message hash piltover registers for a withdrawal — the same value its
 *  `consume_message_from_appchain` reconstructs (and `bank` consumes). Mirrors the
 *  cairo `compute_message_hash_appc_to_sn`: poseidon over
 *  [from_address, to_address, payload_len, ...payload], where the message is sent
 *  from the appchain game system to the settlement bank system with payload
 *  [player, amount, withdraw_no]. */
export function withdrawalMessageHash(player: string, amount: number, withdrawNo: number): string {
  const data = [GAME_SYSTEM, BANK_SYSTEM, "0x3", player, "0x" + amount.toString(16), "0x" + withdrawNo.toString(16)];
  return "0x" + ec.starkCurve.poseidonHashMany(data.map((v) => BigInt(v))).toString(16);
}

/** How many of this player's withdrawals have already been banked on Sepolia. */
export async function getBankCount(player: string): Promise<number> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_BANK,
    `SELECT COUNT(*) AS c FROM "bank-Banked" WHERE player = "${toriiAddr(player)}"`,
  );
  return num(rows[0]?.c ?? 0);
}

// Mirror of the appchain contract's DEPTH_WEIGHT — on death, RunEnded.score is
// exactly DEPTH_WEIGHT * depth (no gold), so depth = score / DEPTH_WEIGHT.
export const DEPTH_WEIGHT = 80;

export type RunEndRow = { endNo: number; runNo: number; depth: number; loot: number; died: boolean; hp: number; maxHp: number };

/** The player's most recent run ending (death or extract), for the outcome screen.
 *  On extract `loot` is the gold banked to the vault; on death it's the forfeited
 *  gold. score = DEPTH_WEIGHT*depth + (gold on extract, 0 on death), so derive depth
 *  accordingly. The RunState row (keyed by run_no) keeps the run's final hp/max_hp. */
export async function getLastRunEnded(player: string): Promise<RunEndRow | null> {
  const ended = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT end_no, run_no, score, loot, died FROM "game-RunEnded" WHERE player = "${toriiAddr(player)}" ORDER BY end_no DESC LIMIT 1`,
  );
  const r = ended[0];
  if (!r) return null;
  const runNo = num(r.run_no);
  const score = num(r.score);
  const loot = num(r.loot);
  const died = !!num(r.died);
  const depth = Math.round((died ? score : score - loot) / DEPTH_WEIGHT);
  const state = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT hp, max_hp FROM "game-RunState" WHERE run_no = "${runKey(runNo)}"`,
  );
  const s = state[0];
  return { endNo: num(r.end_no), runNo, depth, loot, died, hp: num(s?.hp ?? 0), maxHp: num(s?.max_hp ?? 0) };
}

// --- raw RPC reads ---

function u256FromParts(parts: string[]): bigint {
  const low = BigInt(parts[0] ?? 0);
  const high = BigInt(parts[1] ?? 0);
  return low + (high << 128n);
}

async function erc20Balance(token: string, owner: string): Promise<bigint> {
  if (BigInt(token) === 0n) return 0n;
  const res = await settlementProvider.callContract({ contractAddress: token, entrypoint: "balanceOf", calldata: [owner] });
  return u256FromParts(res as string[]);
}
export const gameBalance = (owner: string) => erc20Balance(GAME_TOKEN, owner);
export const goldBalance = (owner: string) => erc20Balance(GOLD_TOKEN, owner);
export const usdcBalance = (owner: string) => erc20Balance(USDC, owner);

export async function entryFee(): Promise<bigint> {
  if (BigInt(ENTRY) === 0n) return 0n;
  const res = await settlementProvider.callContract({ contractAddress: ENTRY, entrypoint: "entry_fee", calldata: [] });
  return u256FromParts(res as string[]);
}

export async function appchainBlock(): Promise<number> {
  return appchainProvider.getBlockNumber();
}

/** Block height settled onto the piltover core by saya (get_state()[1]). */
export async function settledBlock(): Promise<number> {
  const res = await settlementProvider.callContract({ contractAddress: PILTOVER, entrypoint: "get_state", calldata: [] });
  const bn = BigInt(res[1]);
  return bn > 0xffffffffffffffffn ? -1 : Number(bn);
}

/** Format a base-unit token amount to a human string. */
export function fmtToken(raw: bigint, decimals: number, frac = 2): string {
  const base = 10n ** BigInt(decimals);
  const whole = raw / base;
  const rem = raw % base;
  if (frac === 0) return whole.toString();
  const fracStr = (rem * 10n ** BigInt(frac) / base).toString().padStart(frac, "0");
  return `${whole}.${fracStr}`;
}

// --- transaction log (every signed write, L1 settlement + L2 appchain) ---
// A single in-memory feed so the UI can show all transactions the client submits,
// regardless of chain. Reads/`callContract` are not txs and are not logged.

export type TxChain = "L1" | "L2";
export type TxStatus = "pending" | "ok" | "err";
export type TxEntry = {
  id: number;
  chain: TxChain; // L1 = settlement (Sepolia), L2 = appchain
  label: string; // entrypoint-ish name, e.g. "enter", "move_room"
  hash: string; // "" until the tx is submitted
  status: TxStatus;
  ts: number; // ms epoch (submission time)
  explorer: string; // explorer base for this chain (for the hash link)
  error?: string;
};

let txSeq = 0;
const txEntries: TxEntry[] = [];
const txSubs = new Set<(log: TxEntry[]) => void>();
const emitTx = () => {
  const snap = txEntries.slice();
  txSubs.forEach((f) => f(snap));
};

/** Subscribe to the running transaction log; replays the current list immediately. */
export function subscribeTxLog(fn: (log: TxEntry[]) => void): () => void {
  txSubs.add(fn);
  fn(txEntries.slice());
  return () => {
    txSubs.delete(fn);
  };
}
export const clearTxLog = () => {
  txEntries.length = 0;
  emitTx();
};

/** Wrap a write so its lifecycle lands in the tx log: pending (with hash once
 *  submitted) → ok on confirmation, or err if submit/confirm throws. */
async function loggedTx(
  chainTag: TxChain,
  label: string,
  submit: () => Promise<string>,
  confirm: (hash: string) => Promise<unknown>,
): Promise<string> {
  const entry: TxEntry = {
    id: ++txSeq,
    chain: chainTag,
    label,
    hash: "",
    status: "pending",
    ts: Date.now(),
    explorer: chainTag === "L1" ? SETTLEMENT_EXPLORER : APPCHAIN_EXPLORER,
  };
  txEntries.push(entry);
  emitTx();
  try {
    entry.hash = await submit();
    emitTx(); // hash known, still pending confirmation
    await confirm(entry.hash);
    entry.status = "ok";
    emitTx();
    return entry.hash;
  } catch (e) {
    entry.status = "err";
    entry.error = String((e as Error)?.message ?? e);
    emitTx();
    throw e;
  }
}

// --- writes: settlement (Sepolia) ---

const MINT_RUN_SELECTOR = hash.getSelectorFromName("mint_run");
const MESSAGE_SENT_KEY = hash.getSelectorFromName("MessageSent");
void MINT_RUN_SELECTOR; // reserved for pending-entry tracking

/** A settlement (L1) write: serialized behind the settlement lock and logged. */
const settlementTx = (label: string, submit: () => Promise<string>) =>
  withSettlementLock(() =>
    loggedTx("L1", label, submit, (h) => settlementProvider.waitForTransaction(h, SETTLEMENT_TX_WAIT)),
  );

/** Dev faucet: mint GAME directly to the signer (no USDC). */
export async function devMint(account: Signer, amount: bigint): Promise<string> {
  return settlementTx("dev_mint", () =>
    account
      .execute({
        contractAddress: GAME_TOKEN,
        entrypoint: "dev_mint",
        calldata: CallData.compile([cairo.uint256(amount)]),
      })
      .then((r) => r.transaction_hash),
  );
}

/** Buy GAME with USDC: approve USDC to the sale, then buy (one multicall). */
export async function buyGame(account: Signer, usdcAmount: bigint): Promise<string> {
  return settlementTx("buy", () =>
    account
      .execute([
        { contractAddress: USDC, entrypoint: "approve", calldata: CallData.compile([TOKEN_SALE, cairo.uint256(usdcAmount)]) },
        { contractAddress: TOKEN_SALE, entrypoint: "buy", calldata: CallData.compile([cairo.uint256(usdcAmount)]) },
      ])
      .then((r) => r.transaction_hash),
  );
}

/** Enter the dungeon: approve the GAME entry fee to Entry, then enter (multicall).
 *  Sends the L1→L2 message that starts the run for `account.address` on L2. */
export async function enterDungeon(account: Signer): Promise<string> {
  const fee = await entryFee();
  // Free entry: dev-mint exactly the entry fee and spend it in the same multicall, so
  // the player never has to fund. The run is still started from L1 — `enter` is the L1
  // call that sends the L1→L2 message — it just self-funds via the GAME faucet.
  const calls =
    fee > 0n
      ? [
          { contractAddress: GAME_TOKEN, entrypoint: "dev_mint", calldata: CallData.compile([cairo.uint256(fee)]) },
          { contractAddress: GAME_TOKEN, entrypoint: "approve", calldata: CallData.compile([ENTRY, cairo.uint256(fee)]) },
          { contractAddress: ENTRY, entrypoint: "enter", calldata: [] },
        ]
      : [{ contractAddress: ENTRY, entrypoint: "enter", calldata: [] }];
  return settlementTx("enter", () => account.execute(calls).then((r) => r.transaction_hash));
}

/** Bank settled withdrawals on Sepolia. Fast path: one multicall that consumes each
 *  L2→L1 message and mints its GOLD. Callers pass only settled rows (an unsettled
 *  `consume_message_from_appchain` reverts).
 *
 *  Resilience: a single bad row reverts the WHOLE multicall. That can happen when a row
 *  is already consumed but still listed as unclaimed — e.g. the settlement Torii had an
 *  indexing gap so `getBankCount` undercounts (it reads Torii, not the chain). So if the
 *  multicall fails, fall back to banking each row on its own and skip the ones that won't
 *  consume, so the genuinely-claimable withdrawals still go through. Returns the last
 *  successful tx hash; rethrows the original error only if nothing could be banked. */
export async function bankMany(account: Signer, player: string, rows: WithdrawalRow[]): Promise<string> {
  if (!rows.length) throw new Error("no settled withdrawals to bank");
  const mkCall = (w: WithdrawalRow) => ({
    contractAddress: BANK_SYSTEM,
    entrypoint: "bank",
    // bank(from_address = game system, player, amount, withdraw_no)
    calldata: [GAME_SYSTEM, player, "0x" + w.amount.toString(16), "0x" + w.withdrawNo.toString(16)],
  });
  try {
    return await settlementTx("bank", () => account.execute(rows.map(mkCall)).then((r) => r.transaction_hash));
  } catch (multicallErr) {
    if (rows.length === 1) throw multicallErr; // single row — nothing to isolate
    let last = "";
    for (const w of rows) {
      try {
        last = await settlementTx("bank", () => account.execute([mkCall(w)]).then((r) => r.transaction_hash));
      } catch {
        // Skip a row that won't consume (already banked / not yet registered) and keep going.
      }
    }
    if (!last) throw multicallErr; // nothing banked — surface the original multicall failure
    return last;
  }
}

// --- writes: appchain play actions (dev account by default, or a Controller) ---

// Every play action is signed by the one appchain dev account, so serialize them
// through a promise-chain mutex (same idiom as withSettlementLock). Without this,
// two actions fired in quick succession both read the same pending nonce and the
// second is rejected as an invalid nonce. Each action holds the lock until its tx
// is pre-confirmed, so the next one reads the bumped nonce.
let appchainQueue: Promise<unknown> = Promise.resolve();
function withAppchainLock<T>(fn: () => Promise<T>): Promise<T> {
  const run = appchainQueue.then(fn, fn);
  appchainQueue = run.then(() => {}, () => {});
  return run;
}

/** Run a one-felt appchain entrypoint and wait for it (+ a beat for Torii).
 *  With no `account`, the local dev key signs (the fast path below). When a
 *  Controller is connected, its appchain signer is passed in instead. */
async function appchainCall(entrypoint: string, arg: string, account?: Signer): Promise<string> {
  return withAppchainLock(async () => {
    const transaction_hash = await loggedTx(
      "L2",
      entrypoint,
      async () => {
        const call = { contractAddress: GAME_SYSTEM, entrypoint, calldata: [arg] };
        // A Controller signs via the keychain, which owns the nonce + fee — just execute.
        // The dev account (passed explicitly in operator mode, or defaulted) takes the
        // pre-confirmed fast path below instead.
        if (account && account !== appchainAccount) {
          const r = await account.execute(call);
          return r.transaction_hash;
        }
        // Dev-key fast path. Interval mining (--block-time) means `latest` (the last mined
        // block) lags the pre-confirmed block, and starknet.js reads BOTH the nonce and the
        // fee estimate against `latest` by default. Two consequences, both fixed by pinning
        // to pre_confirmed: (1) the latest nonce ignores a tx still pending in this block
        // window, so consecutive actions reuse a stale nonce and are rejected; (2) the
        // estimate simulates against stale state, so an action that only just became valid
        // (e.g. loot right after moving into a treasure room) reverts even though it would
        // execute fine. The lock keeps fetch+estimate+submit atomic.
        const nonce = await appchainProvider.getNonceForAddress(appchainAccount.address, BlockTag.PRE_CONFIRMED);
        const { resourceBounds } = await appchainAccount.estimateInvokeFee(call, {
          blockIdentifier: BlockTag.PRE_CONFIRMED,
          nonce,
        });
        const r = await appchainAccount.execute(call, { nonce, resourceBounds });
        return r.transaction_hash;
      },
      (h) => appchainProvider.waitForTransaction(h, APPCHAIN_TX_WAIT),
    );
    await sleep(150); // give Torii a beat to index the resulting model/event write
    return transaction_hash;
  });
}

// Play actions target a specific run by its run_no. `account` is the Controller's
// appchain signer when connected, else undefined (the dev key signs).
const action = (entrypoint: string, runNo: number, account?: Signer) =>
  appchainCall(entrypoint, "0x" + runNo.toString(16), account);
export const moveRoom = (runNo: number, account?: Signer) => action("move_room", runNo, account);
export const attack = (runNo: number, account?: Signer) => action("attack", runNo, account);
export const loot = (runNo: number, account?: Signer) => action("loot", runNo, account);
export const useItem = (runNo: number, account?: Signer) => action("use_item", runNo, account);

/** Extract run `runNo`: ends it alive and banks its gold into the player's L2 vault. */
export const extract = (runNo: number, account?: Signer) => action("extract", runNo, account);

/** Withdraw: send the whole vault to L1 as one message (the first half of banking).
 *  Keyed by player (the vault spans all runs); the second half is `bankMany` on L1. */
export const withdraw = (player: string, account?: Signer) => appchainCall("withdraw", player, account);

export { MESSAGE_SENT_KEY };
