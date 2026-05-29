// Thin wrapper around starknet.js for the demo. Reads addresses from the
// generated deployments.json (written by the deploy scripts via up.sh).

import { Account, RpcProvider, hash } from "starknet";
import deployments from "./deployments.json";

export const SETTLEMENT_RPC = deployments.settlement.rpcUrl;
export const APPCHAIN_RPC = deployments.appchain.rpcUrl;
export const MESSAGING_CONTRACT = deployments.settlement.messagingContract as string;
export const GAME_CONTRACT = deployments.appchain.gameContract as string;
export const BUYER_ADDRESS = deployments.account.address;

// Each Katana node serves its own block explorer at <rpc-url>/explorer when
// started with `--explorer`. Transactions deep-link at /explorer/tx/<hash>.
export const SETTLEMENT_EXPLORER = `${SETTLEMENT_RPC}/explorer`;
export const APPCHAIN_EXPLORER = `${APPCHAIN_RPC}/explorer`;

export function explorerTxUrl(explorerBase: string, txHash: string): string {
  return `${explorerBase}/tx/${txHash}`;
}

const MINT_GAME_SELECTOR = hash.getSelectorFromName("mint_game");
const GAME_MINTED_KEY = hash.getSelectorFromName("GameMinted");

// Read provider for the appchain ("L2").
const appchainProvider = new RpcProvider({ nodeUrl: APPCHAIN_RPC });

// Signing account on the settlement layer ("L1"). The dev key is a throwaway
// local key printed by `katana --dev`; safe to embed for a local demo only.
const settlementAccount = new Account({
  provider: new RpcProvider({ nodeUrl: SETTLEMENT_RPC }),
  address: deployments.account.address,
  signer: deployments.account.privateKey,
  cairoVersion: "1",
});

export type AppchainState = {
  totalMinted: number;
  mintedByYou: number;
  lastBuyer: string;
};

async function callFelt(contractAddress: string, entrypoint: string, calldata: string[] = []) {
  const res = await appchainProvider.callContract({ contractAddress, entrypoint, calldata });
  return BigInt(res[0]);
}

/** Read the live game state from the appchain. */
export async function readAppchainState(): Promise<AppchainState> {
  const [total, mine, last] = await Promise.all([
    callFelt(GAME_CONTRACT, "total_minted"),
    callFelt(GAME_CONTRACT, "minted_by", [BUYER_ADDRESS]),
    callFelt(GAME_CONTRACT, "last_buyer"),
  ]);
  return {
    totalMinted: Number(total),
    mintedByYou: Number(mine),
    lastBuyer: last === 0n ? "" : "0x" + last.toString(16),
  };
}

/**
 * Read the L2 transaction hashes of every `mint_game` execution so far, in order.
 * These are the L1-handler txs Katana submitted on the appchain when it relayed
 * each message — the "other half" of the cross-chain round trip.
 */
export async function getMintTxHashes(): Promise<string[]> {
  const hashes: string[] = [];
  let continuationToken: string | undefined;
  // A handful of chunks is plenty for a demo; cap to stay bounded.
  for (let page = 0; page < 10; page++) {
    const res = await appchainProvider.getEvents({
      address: GAME_CONTRACT,
      from_block: { block_number: 0 },
      to_block: "latest",
      keys: [[GAME_MINTED_KEY]],
      chunk_size: 100,
      continuation_token: continuationToken,
    });
    for (const ev of res.events) hashes.push(ev.transaction_hash);
    continuationToken = res.continuation_token;
    if (!continuationToken) break;
  }
  return hashes;
}

/**
 * Perform the settlement-layer ("L1") operation: send a message to the appchain
 * that will invoke `mint_game(from_address, game_id)`. Returns the L1 tx hash.
 */
export async function purchaseGame(gameId: number): Promise<string> {
  const { transaction_hash } = await settlementAccount.execute({
    contractAddress: MESSAGING_CONTRACT,
    entrypoint: "send_message_to_appchain",
    // send_message_to_appchain(to_address, selector, payload: Span<felt252>)
    // Span serializes as [len, ...elements].
    calldata: [GAME_CONTRACT, MINT_GAME_SELECTOR, "1", "0x" + gameId.toString(16)],
  });
  return transaction_hash;
}

export function shortHex(value: string, lead = 6, tail = 4): string {
  if (!value) return "—";
  if (value.length <= lead + tail + 2) return value;
  return `${value.slice(0, lead)}…${value.slice(-tail)}`;
}
