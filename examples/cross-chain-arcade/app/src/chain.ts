// Frontend data layer for the cross-chain arcade.
//
// Reads are plain starknet.js `callContract` (no Torii / no Dojo):
//   - each machine's `coins` / `last_player` from the appchain RPC
//   - the arcade's `total_coins` (messages sent) from the settlement RPC
// "Pending" relays = messages sent on L1 minus coins landed on L2. Writes go
// through the settlement dev account calling `arcade.play_all`.

import { Account, CallData, RpcProvider, shortString } from "starknet";
import deployments from "./deployments.json";

export const SETTLEMENT_EXPLORER = deployments.settlement.explorer;
export const APPCHAIN_EXPLORER = deployments.appchain.explorer;
export const ARCADE = deployments.settlement.arcade as string;
export const PLAYER = deployments.appchain.account.address;

const settlement = new RpcProvider({ nodeUrl: deployments.settlement.rpcUrl });
const appchain = new RpcProvider({ nodeUrl: deployments.appchain.rpcUrl });

const settlementAccount = new Account({
  provider: settlement,
  address: deployments.settlement.account.address,
  signer: deployments.settlement.account.privateKey,
});

export type MachineState = {
  name: string;
  address: string;
  coins: number;
  lastPlayer: string;
};

export type ArcadeState = {
  machines: MachineState[];
  sent: number; // messages dispatched from L1 (arcade.total_coins)
  landed: number; // coins received across all machines on L2
  pending: number; // in-flight relays
};

const num = (v: string) => Number(BigInt(v));

async function view(
  p: RpcProvider,
  contractAddress: string,
  entrypoint: string,
  calldata: (string | number)[] = [],
): Promise<string[]> {
  return p.callContract({ contractAddress, entrypoint, calldata: CallData.compile(calldata) });
}

export async function fetchState(): Promise<ArcadeState> {
  const defs = deployments.appchain.machines ?? [];
  const machines = await Promise.all(
    defs.map(async (m): Promise<MachineState> => {
      const [[coins], [lastPlayer]] = await Promise.all([
        view(appchain, m.address, "coins"),
        view(appchain, m.address, "last_player"),
      ]);
      return {
        name: shortString.decodeShortString(BigInt(m.name).toString()),
        address: m.address,
        coins: num(coins),
        lastPlayer,
      };
    }),
  );

  let sent = 0;
  try {
    const [total] = await view(settlement, ARCADE, "total_coins");
    sent = num(total);
  } catch {
    // arcade not deployed yet
  }

  const landed = machines.reduce((acc, m) => acc + m.coins, 0);
  return { machines, sent, landed, pending: Math.max(0, sent - landed) };
}

/** Dispatch one coin to every machine (a single L1 tx, fanning out). */
export async function playAll(): Promise<string> {
  const { transaction_hash } = await settlementAccount.execute({
    contractAddress: ARCADE,
    entrypoint: "play_all",
    calldata: CallData.compile([PLAYER]),
  });
  return transaction_hash;
}
