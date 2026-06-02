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

import { Account, type AccountInterface, CallData, RpcProvider, cairo, hash } from "starknet";
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
// Poll fast on both chains. The appchain mines instantly; Sepolia confirms in ~1-2s
// (measured) — far quicker than the 5s default, which was the bulk of the perceived
// "entering…" / "banking…" latency. The interval is a poll cadence, not added work.
const APPCHAIN_TX_WAIT = { retryInterval: 200 };
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
export type Signer = Pick<AccountInterface, "execute" | "address">;

// Buy / enter / bank are all signed by the same settlement account, so serialize
// them through a promise-chain mutex to avoid racing the nonce.
let settlementQueue: Promise<unknown> = Promise.resolve();
function withSettlementLock<T>(fn: () => Promise<T>): Promise<T> {
  const run = settlementQueue.then(fn, fn);
  settlementQueue = run.then(() => {}, () => {});
  return run;
}

async function toriiSql<T = Record<string, string | number>>(base: string, sql: string): Promise<T[]> {
  const res = await fetch(`${base}/sql?query=${encodeURIComponent(sql)}`);
  if (!res.ok) throw new Error(`torii sql ${res.status}: ${await res.text()}`);
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
export async function subscribeToriiUpdates(onUpdate: () => void): Promise<() => void> {
  const subs: { cancel: () => void }[] = [];
  const clients: ToriiClient[] = [];

  const connect = async (toriiUrl: string, worldAddress: string) => {
    // NB: despite its `.d.ts`, the wasm `ToriiClient` constructor is async — it
    // returns a Promise that resolves to the connected client.
    const client = await (new ToriiClient({ toriiUrl, worldAddress }) as unknown as Promise<ToriiClient>);
    clients.push(client);
    subs.push(await client.onEntityUpdated(null, null, () => onUpdate()));
    subs.push(await client.onEventMessageUpdated(null, null, () => onUpdate()));
  };

  await Promise.all([connect(TORII_GAME, GAME_WORLD), connect(TORII_BANK, BANK_WORLD)]);

  return () => {
    for (const s of subs) {
      try {
        s.cancel();
      } catch {
        // already gone — fine
      }
    }
    for (const c of clients) {
      try {
        c.free();
      } catch {
        // already freed — fine
      }
    }
  };
}

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

// --- live run state (game world `RunState`, keyed by the L1 player) ---

export type RunState = {
  alive: boolean;
  depth: number;
  hp: number;
  maxHp: number;
  gold: number;
  roomKind: number;
  enemyHp: number;
  potions: number;
};

export async function readRun(player: string): Promise<RunState | null> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT alive, depth, hp, max_hp, gold, room_kind, enemy_hp, potions FROM "game-RunState" WHERE player = "${player}"`,
  );
  const r = rows[0];
  if (!r || !num(r.alive)) return null;
  return {
    alive: !!num(r.alive),
    depth: num(r.depth),
    hp: num(r.hp),
    maxHp: num(r.max_hp),
    gold: num(r.gold),
    roomKind: num(r.room_kind),
    enemyHp: num(r.enemy_hp),
    potions: num(r.potions),
  };
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

// --- action feed (game world `ActionTaken`) — the roguelike message log ---

export type ActionRow = {
  actionNo: number;
  runNo: number;
  player: string; // the run's player (will become a Controller username once integrated)
  kind: string;
  outcome: string;
  depth: number;
  hp: number;
  gold: number;
  block: number;
  txHash: string;
};

const feltToStr = (v: string | number): string => {
  // ActionTaken kind/outcome are short-string felts; decode to ASCII.
  try {
    let n = BigInt(v);
    let s = "";
    while (n > 0n) {
      s = String.fromCharCode(Number(n & 0xffn)) + s;
      n >>= 8n;
    }
    return s || String(v);
  } catch {
    return String(v);
  }
};

export async function getActionFeed(limit = 40): Promise<ActionRow[]> {
  // `run_no` is an authoritative field on the ActionTaken event (set from the
  // run's RunState), so each action carries its run id directly.
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT action_no, run_no, player, kind, outcome, depth, hp, gold, internal_event_id FROM "game-ActionTaken" ORDER BY action_no DESC LIMIT ${limit}`,
  );
  return rows.map((r) => {
    const { block, txHash } = parseEventId(String(r.internal_event_id));
    return {
      actionNo: num(r.action_no),
      runNo: num(r.run_no),
      player: String(r.player),
      kind: feltToStr(r.kind),
      outcome: feltToStr(r.outcome),
      depth: num(r.depth),
      hp: num(r.hp),
      gold: num(r.gold),
      block,
      txHash,
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
    `SELECT gold FROM "game-Vault" WHERE player = "${player}"`,
  );
  return num(rows[0]?.gold ?? 0);
}

// --- withdrawal / bank reconciliation (drives the "Bank" step) ---

export type WithdrawalRow = { withdrawNo: number; amount: number; block: number };

/** This player's withdrawals (vault → L1), oldest-first. Each needs a `bank` on
 *  Sepolia once saya has settled the block it landed in. */
export async function getWithdrawals(player: string): Promise<WithdrawalRow[]> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_GAME,
    `SELECT withdraw_no, amount, internal_event_id FROM "game-Withdrawal" WHERE player = "${player}" ORDER BY withdraw_no`,
  );
  return rows.map((r) => ({
    withdrawNo: num(r.withdraw_no),
    amount: num(r.amount),
    block: parseEventId(String(r.internal_event_id)).block,
  }));
}

/** How many of this player's withdrawals have already been banked on Sepolia. */
export async function getBankCount(player: string): Promise<number> {
  const rows = await toriiSql<Record<string, string | number>>(
    TORII_BANK,
    `SELECT COUNT(*) AS c FROM "bank-Banked" WHERE player = "${player}"`,
  );
  return num(rows[0]?.c ?? 0);
}

// Mirror of the appchain contract's DEPTH_WEIGHT — on death, RunEnded.score is
// exactly DEPTH_WEIGHT * depth (no gold), so depth = score / DEPTH_WEIGHT.
export const DEPTH_WEIGHT = 80;

export type RunEndRow = { endNo: number; depth: number; loot: number; died: boolean; hp: number; maxHp: number };

/** The player's most recent run ending (death or extract), for showing the
 *  outcome once the run clears. On extract `loot` is the gold banked to the vault;
 *  on death it's the forfeited gold. score = DEPTH_WEIGHT*depth + (gold on extract,
 *  0 on death), so derive depth accordingly. The RunState row keeps the run's final
 *  hp/max_hp after it ends (0 on death), so read those for the outcome screen. */
export async function getLastRunEnded(player: string): Promise<RunEndRow | null> {
  const [ended, state] = await Promise.all([
    toriiSql<Record<string, string | number>>(
      TORII_GAME,
      `SELECT end_no, score, loot, died FROM "game-RunEnded" WHERE player = "${player}" ORDER BY end_no DESC LIMIT 1`,
    ),
    toriiSql<Record<string, string | number>>(
      TORII_GAME,
      `SELECT hp, max_hp FROM "game-RunState" WHERE player = "${player}"`,
    ),
  ]);
  const r = ended[0];
  if (!r) return null;
  const score = num(r.score);
  const loot = num(r.loot);
  const died = !!num(r.died);
  const depth = Math.round((died ? score : score - loot) / DEPTH_WEIGHT);
  const s = state[0];
  return { endNo: num(r.end_no), depth, loot, died, hp: num(s?.hp ?? 0), maxHp: num(s?.max_hp ?? 0) };
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

// --- writes: settlement (Sepolia) ---

const MINT_RUN_SELECTOR = hash.getSelectorFromName("mint_run");
const MESSAGE_SENT_KEY = hash.getSelectorFromName("MessageSent");
void MINT_RUN_SELECTOR; // reserved for pending-entry tracking

/** Dev faucet: mint GAME directly to the signer (no USDC). */
export async function devMint(account: Signer, amount: bigint): Promise<string> {
  return withSettlementLock(async () => {
    const { transaction_hash } = await account.execute({
      contractAddress: GAME_TOKEN,
      entrypoint: "dev_mint",
      calldata: CallData.compile([cairo.uint256(amount)]),
    });
    await settlementProvider.waitForTransaction(transaction_hash, SETTLEMENT_TX_WAIT);
    return transaction_hash;
  });
}

/** Buy GAME with USDC: approve USDC to the sale, then buy (one multicall). */
export async function buyGame(account: Signer, usdcAmount: bigint): Promise<string> {
  return withSettlementLock(async () => {
    const { transaction_hash } = await account.execute([
      { contractAddress: USDC, entrypoint: "approve", calldata: CallData.compile([TOKEN_SALE, cairo.uint256(usdcAmount)]) },
      { contractAddress: TOKEN_SALE, entrypoint: "buy", calldata: CallData.compile([cairo.uint256(usdcAmount)]) },
    ]);
    await settlementProvider.waitForTransaction(transaction_hash, SETTLEMENT_TX_WAIT);
    return transaction_hash;
  });
}

/** Enter the dungeon: approve the GAME entry fee to Entry, then enter (multicall).
 *  Sends the L1→L2 message that starts the run for `account.address` on L2. */
export async function enterDungeon(account: Signer): Promise<string> {
  const fee = await entryFee();
  return withSettlementLock(async () => {
    const { transaction_hash } = await account.execute([
      { contractAddress: GAME_TOKEN, entrypoint: "approve", calldata: CallData.compile([ENTRY, cairo.uint256(fee)]) },
      { contractAddress: ENTRY, entrypoint: "enter", calldata: [] },
    ]);
    await settlementProvider.waitForTransaction(transaction_hash, SETTLEMENT_TX_WAIT);
    return transaction_hash;
  });
}

/** Bank a settled withdrawal on Sepolia: consume the L2→L1 message, mint GOLD. */
export async function bankRun(account: Signer, player: string, amount: number, withdrawNo: number): Promise<string> {
  return withSettlementLock(async () => {
    const { transaction_hash } = await account.execute({
      contractAddress: BANK_SYSTEM,
      entrypoint: "bank",
      // bank(from_address = game system, player, amount, withdraw_no)
      calldata: [GAME_SYSTEM, player, "0x" + amount.toString(16), "0x" + withdrawNo.toString(16)],
    });
    await settlementProvider.waitForTransaction(transaction_hash, SETTLEMENT_TX_WAIT);
    return transaction_hash;
  });
}

// --- writes: appchain play actions (signed by the dev account) ---

async function appchainAction(entrypoint: string, player: string): Promise<string> {
  const { transaction_hash } = await appchainAccount.execute({
    contractAddress: GAME_SYSTEM,
    entrypoint,
    calldata: [player],
  });
  await appchainProvider.waitForTransaction(transaction_hash, APPCHAIN_TX_WAIT);
  // Give Torii a beat to index the resulting model/event write.
  await sleep(150);
  return transaction_hash;
}

export const moveRoom = (player: string) => appchainAction("move_room", player);
export const attack = (player: string) => appchainAction("attack", player);
export const loot = (player: string) => appchainAction("loot", player);
export const useItem = (player: string) => appchainAction("use_item", player);

/** Extract: ends the run alive and banks its gold into the player's L2 vault. */
export const extract = (player: string) => appchainAction("extract", player);

/** Withdraw: send the whole vault to L1 as one message (the first half of banking).
 *  Signed by the appchain account; the second half is `bankRun` on Sepolia. */
export const withdraw = (player: string) => appchainAction("withdraw", player);

export { MESSAGE_SENT_KEY };
