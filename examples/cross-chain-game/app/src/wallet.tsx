// Wallet layer: lets the player pick how transactions are signed.
//
// Default is the hardcoded **dev accounts** (offline, one-click) — no connect
// needed. A runtime "Login" choice can switch to a **Cartridge Controller**: one
// identity that signs on BOTH chains (buy + bank on L1, roll on L2) at the same
// address. Controller is a hosted-keychain wallet, so it needs the stack started
// in Controller mode (`CONTROLLER=1 ./up.sh`) + a Controller login; see README.

import { createContext, useContext, useState, type PropsWithChildren } from "react";
import { type AccountInterface, constants, shortString } from "starknet";
import ControllerConnector from "@cartridge/connector/controller";
import { StarknetConfig, jsonRpcProvider, useAccount, useConnect, useDisconnect } from "@starknet-react/core";
import type { Chain } from "@starknet-react/chains";
import {
  SETTLEMENT_RPC,
  APPCHAIN_RPC,
  SCORE_REGISTRY,
  STORE,
  GAME,
  settlementAccount,
  appchainAccount,
  shortHex,
  type Signer,
} from "./chain.ts";

// chain ids (felt-hex). Settlement runs --chain-id SN_SEPOLIA; the appchain runs
// `katana init rollup --id GAMECHAIN`. switchStarknetChain takes these strings.
const SETTLEMENT_CHAIN_ID = constants.StarknetChainId.SN_SEPOLIA;
const APPCHAIN_CHAIN_ID = shortString.encodeShortString("GAMECHAIN");

const STRK = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";
const chain = (id: string, name: string, rpc: string): Chain => ({
  id: BigInt(id),
  network: name.toLowerCase().replace(/\s+/g, "-"),
  name,
  nativeCurrency: { name: "Stark", symbol: "STRK", decimals: 18, address: STRK },
  rpcUrls: { default: { http: [rpc] }, public: { http: [rpc] } },
  // starknet-react requires a paymaster provider per chain (its default reads
  // `paymasterRpcUrls.avnu.http`). We don't use a starknet-react paymaster — the
  // Controller handles its own — so just point it at the chain's own RPC.
  paymasterRpcUrls: { avnu: { http: [rpc] } },
});
const settlementChain = chain(SETTLEMENT_CHAIN_ID, "Katana Settlement", SETTLEMENT_RPC);
const appchainChain = chain(APPCHAIN_CHAIN_ID, "Katana Appchain", APPCHAIN_RPC);

const provider = jsonRpcProvider({
  rpc: (c: Chain) => ({ nodeUrl: c.id === appchainChain.id ? APPCHAIN_RPC : SETTLEMENT_RPC }),
});

// Session policies: scope the Controller session to the demo's entrypoints (per
// chain) so buy/roll/bank are gasless session calls (no per-tx popup).
const policies = {
  contracts: {
    [STORE]: { methods: [{ name: "Buy game", entrypoint: "buy_game" }] },
    [SCORE_REGISTRY]: { methods: [{ name: "Bank score", entrypoint: "claim_score" }] },
    [GAME]: { methods: [{ name: "Roll", entrypoint: "play_game" }] },
  },
};

// Created at module level (the connector warns against per-render instances).
//
// The connector probes each RPC **synchronously** at construction to resolve its
// chain id. If the settlement/appchain nodes are down, that throws — and because
// this runs at module load, an unguarded throw would blank the entire app (React
// never mounts). Build it defensively: on failure the app still renders with the
// dev account, and the Controller option is marked unavailable until the stack is
// up. See <ServicesOffline> in App.tsx for the user-facing banner.
function createControllerConnector(): ControllerConnector | null {
  try {
    return new ControllerConnector({
      chains: [{ rpcUrl: SETTLEMENT_RPC }, { rpcUrl: APPCHAIN_RPC }],
      defaultChainId: SETTLEMENT_CHAIN_ID,
      // Hosted keychain by default; override for a self-hosted keychain.
      url: import.meta.env.VITE_KEYCHAIN_URL || undefined,
      policies,
    });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("Cartridge Controller unavailable — is the local stack running?", err);
    return null;
  }
}
export const controllerConnector = createControllerConnector();

export type WalletMethod = "dev" | "controller";

type WalletCtx = {
  method: WalletMethod;
  /** L1 signer (buy + bank): dev account, or the Controller when connected. */
  l1Account: AccountInterface;
  /** L2 signer (roll): dev appchain account, or the Controller (on the appchain). */
  l2Account: Signer;
  l1Address: string;
  /** Address that `play_game` records as the player (and that `claim_score` must
   *  consume the L2→L1 message for): the dev appchain account, or the Controller
   *  (same address on both chains). */
  playerAddress: string;
  label: string;
  username?: string;
  connecting: boolean;
  /** False when the Controller connector couldn't be built (stack down). */
  controllerAvailable: boolean;
  connectController: () => Promise<void>;
  useDevAccount: () => Promise<void>;
};

const Ctx = createContext<WalletCtx | null>(null);

export function useWallet(): WalletCtx {
  const v = useContext(Ctx);
  if (!v) throw new Error("useWallet must be used within <WalletProvider>");
  return v;
}

function WalletInner({ children }: PropsWithChildren) {
  const { connectAsync } = useConnect();
  const { disconnectAsync } = useDisconnect();
  const { account: ctrlAccount, address: ctrlAddress } = useAccount();
  const [method, setMethod] = useState<WalletMethod>("dev");
  const [username, setUsername] = useState<string>();
  const [connecting, setConnecting] = useState(false);

  const connectController = async () => {
    if (!controllerConnector) throw new Error("Controller unavailable — start the stack first (./up.sh).");
    setConnecting(true);
    try {
      await connectAsync({ connector: controllerConnector });
      try {
        setUsername(await controllerConnector.username());
      } catch {
        setUsername(undefined);
      }
      setMethod("controller");
    } finally {
      setConnecting(false);
    }
  };

  const useDevAccount = async () => {
    setMethod("dev");
    setUsername(undefined);
    try {
      await disconnectAsync();
    } catch {
      // not connected — fine
    }
  };

  // Controller is only the active signer once a method=controller session exists;
  // otherwise everything defaults to the dev accounts.
  const usingController = method === "controller" && !!ctrlAccount;
  const l1Account = (usingController ? ctrlAccount : settlementAccount) as AccountInterface;
  const l1Address = usingController ? (ctrlAddress ?? "") : settlementAccount.address;
  // The roll's caller: the Controller (same address on both chains) when connected,
  // else the dev appchain account. claim_score must consume the score message for
  // this address, not the L1 signer (which differs in dev mode).
  const playerAddress = usingController ? (ctrlAddress ?? "") : appchainAccount.address;
  const label = usingController ? (username ?? shortHex(l1Address)) : "Dev account";

  // L2 signer. The Controller's default chain is the settlement layer (for
  // buy/bank), so for a roll we switch it to the appchain, execute, then switch
  // back — keeping the same Controller identity/address across both chains.
  //
  // We execute via the raw `controller.account` (a ControllerAccount), not the
  // `useAccount()` value: starknet-react's WalletAccount is bound to the
  // *settlement* RPC, so its client-side fee estimate hits :5050 and fails for
  // the appchain game contract. ControllerAccount.execute delegates straight to
  // the keychain, which runs against whatever chain we just switched it to.
  const l2Account: Signer = usingController
    ? {
        execute: async (calls) => {
          // usingController implies a live connection, so the connector exists.
          const ctrl = controllerConnector!.controller;
          await ctrl.switchStarknetChain(APPCHAIN_CHAIN_ID);
          try {
            return await (ctrl.account ?? ctrlAccount!).execute(calls);
          } finally {
            await ctrl.switchStarknetChain(SETTLEMENT_CHAIN_ID);
          }
        },
      }
    : appchainAccount;

  return (
    <Ctx.Provider
      value={{
        method,
        l1Account,
        l2Account,
        l1Address,
        playerAddress,
        label,
        username,
        connecting,
        controllerAvailable: !!controllerConnector,
        connectController,
        useDevAccount,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function WalletProvider({ children }: PropsWithChildren) {
  return (
    <StarknetConfig
      chains={[settlementChain, appchainChain]}
      connectors={controllerConnector ? [controllerConnector] : []}
      provider={provider}
    >
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}
