// Wallet layer: lets the player pick how the L1 (settlement) identity is signed.
//
// Default is the hardcoded **dev account** (offline, one-click) — no connect
// needed. A runtime "Login" choice can switch the L1 signer to a **Cartridge
// Controller** account (buy + bank only; the appchain roll always uses the dev
// key). Controller is a hosted-keychain wallet, so it needs the stack started in
// Controller mode (`CONTROLLER=1 ./up.sh`) + a Controller login; see README.

import { createContext, useContext, useState, type PropsWithChildren } from "react";
import { type AccountInterface, constants } from "starknet";
import ControllerConnector from "@cartridge/connector/controller";
import { StarknetConfig, jsonRpcProvider, useAccount, useConnect, useDisconnect } from "@starknet-react/core";
import type { Chain } from "@starknet-react/chains";
import { SETTLEMENT_RPC, SCORE_REGISTRY, STORE, settlementAccount, shortHex } from "./chain.ts";

// The settlement chain as a starknet-react Chain (it runs --chain-id SN_SEPOLIA).
const settlementChain: Chain = {
  id: BigInt(constants.StarknetChainId.SN_SEPOLIA),
  network: "katana-settlement",
  name: "Katana Settlement",
  nativeCurrency: {
    name: "Stark",
    symbol: "STRK",
    decimals: 18,
    address: "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d",
  },
  rpcUrls: {
    default: { http: [SETTLEMENT_RPC] },
    public: { http: [SETTLEMENT_RPC] },
  },
  // starknet-react requires a paymaster provider per chain (its default reads
  // `paymasterRpcUrls.avnu.http`). We don't use a starknet-react paymaster — the
  // Controller handles its own — so just point it at the settlement RPC.
  paymasterRpcUrls: { avnu: { http: [SETTLEMENT_RPC] } },
};

const provider = jsonRpcProvider({ rpc: () => ({ nodeUrl: SETTLEMENT_RPC }) });

// Session policies: scope the Controller session to the demo's L1 entrypoints so
// buy/bank are gasless session calls (no per-tx popup).
const policies = {
  contracts: {
    [STORE]: { methods: [{ name: "Buy game", entrypoint: "buy_game" }] },
    [SCORE_REGISTRY]: { methods: [{ name: "Bank score", entrypoint: "claim_score" }] },
  },
};

// Created at module level (the connector warns against per-render instances).
export const controllerConnector = new ControllerConnector({
  chains: [{ rpcUrl: SETTLEMENT_RPC }],
  defaultChainId: constants.StarknetChainId.SN_SEPOLIA,
  // Hosted keychain by default; override for a self-hosted keychain.
  url: import.meta.env.VITE_KEYCHAIN_URL || undefined,
  policies,
});

export type WalletMethod = "dev" | "controller";

type WalletCtx = {
  method: WalletMethod;
  /** The active L1 signer for buy/bank — dev account, or Controller when connected. */
  l1Account: AccountInterface;
  l1Address: string;
  label: string;
  username?: string;
  connecting: boolean;
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
  // otherwise everything (incl. buy/bank) defaults to the dev account.
  const usingController = method === "controller" && !!ctrlAccount;
  const l1Account = (usingController ? ctrlAccount : settlementAccount) as AccountInterface;
  const l1Address = usingController ? (ctrlAddress ?? "") : settlementAccount.address;
  const label = usingController ? (username ?? shortHex(l1Address)) : "Dev account";

  return (
    <Ctx.Provider
      value={{ method, l1Account, l1Address, label, username, connecting, connectController, useDevAccount }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function WalletProvider({ children }: PropsWithChildren) {
  return (
    <StarknetConfig chains={[settlementChain]} connectors={[controllerConnector]} provider={provider}>
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}
