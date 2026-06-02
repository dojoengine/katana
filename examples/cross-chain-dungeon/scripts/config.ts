// Environment + economy config for the cross-chain-dungeon deploy.
//
// Settlement is REAL Starknet Sepolia, so the operator/saya accounts, the Sepolia
// RPC, and the external USDC address all come from the environment (see .env.example).
// Bun loads `.env` automatically; `up.sh` also exports these before invoking.

function req(name: string): string {
  const v = process.env[name];
  if (!v) throw new Error(`missing required env var ${name} (see .env.example)`);
  return v;
}

export const config = {
  // Settlement RPC: SETTLEMENT_RPC_URL preferred, SEPOLIA_RPC_URL kept for back-compat.
  settlementRpcUrl: process.env.SETTLEMENT_RPC_URL ?? req("SEPOLIA_RPC_URL"),
  operator: { address: req("OPERATOR_ADDRESS"), privateKey: req("OPERATOR_PRIVATE_KEY") },
  saya: { address: req("SAYA_ADDRESS"), privateKey: req("SAYA_PRIVATE_KEY") },
  usdc: req("USDC_ADDRESS"),
  // Economy (base units, as bigint).
  rate: BigInt(process.env.GAME_RATE ?? "100000000000000"),
  entryFee: BigInt(process.env.ENTRY_FEE ?? "50000000000000000000"),
  // GOLD minted per unit of dungeon gold banked (GOLD is 18-decimal): 1 gold = 1 GOLD.
  rewardPerGold: BigInt(process.env.REWARD_PER_GOLD ?? "1000000000000000000"),
};

// Local appchain endpoints (distinct port band from cross-chain-game — see PLAN.md).
export const APPCHAIN_RPC = "http://localhost:5070";
export const SEPOLIA_EXPLORER = "https://sepolia.voyager.online";
export const APPCHAIN_EXPLORER = "http://localhost:5070/explorer";
export const TORII_SCORE = "http://localhost:8091";
export const TORII_GAME = "http://localhost:8092";
