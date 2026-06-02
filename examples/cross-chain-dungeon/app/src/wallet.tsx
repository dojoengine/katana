// Wallet layer: who signs the settlement-layer (Sepolia) transactions.
//
// Default is the **operator account** (a real funded Sepolia account from
// deployments.json) — buy / enter / bank work with no login. A "Login" choice can
// swap in a **Cartridge Controller** instead. Unlike cross-chain-game, the
// Controller here only ever touches **Sepolia** (a network the hosted keychain
// knows), so there's no chain-switching: the appchain play actions are always
// signed by the local dev account in chain.ts. That sidesteps the
// hosted-keychain-can't-switch-to-a-local-appchain limitation entirely.

import { createContext, useContext, useState, type PropsWithChildren } from "react";
import { type AccountInterface, constants } from "starknet";
import ControllerConnector from "@cartridge/connector/controller";
import { StarknetConfig, jsonRpcProvider, useAccount, useConnect, useDisconnect } from "@starknet-react/core";
import type { Chain } from "@starknet-react/chains";
import {
  SEPOLIA_RPC,
  USDC,
  GAME_TOKEN,
  TOKEN_SALE,
  ENTRY,
  BANK_SYSTEM,
  operatorAccount,
  shortHex,
  type Signer,
} from "./chain.ts";

const SEPOLIA_CHAIN_ID = constants.StarknetChainId.SN_SEPOLIA;
const STRK = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

const sepoliaChain: Chain = {
  id: BigInt(SEPOLIA_CHAIN_ID),
  network: "sepolia",
  name: "Starknet Sepolia",
  nativeCurrency: { name: "Stark", symbol: "STRK", decimals: 18, address: STRK },
  rpcUrls: { default: { http: [SEPOLIA_RPC] }, public: { http: [SEPOLIA_RPC] } },
  // starknet-react requires a paymaster provider per chain (its default reads
  // paymasterRpcUrls.avnu.http); the Controller runs its own, so point it at the rpc.
  paymasterRpcUrls: { avnu: { http: [SEPOLIA_RPC] } },
};

const provider = jsonRpcProvider({ rpc: () => ({ nodeUrl: SEPOLIA_RPC }) });

// Session policies: scope the Controller session to the demo's Sepolia entrypoints
// so buy / enter / bank are gasless session calls (no per-tx popup).
const policies = {
  contracts: {
    [USDC]: { methods: [{ name: "Approve", entrypoint: "approve" }] },
    [GAME_TOKEN]: {
      methods: [{ name: "Approve", entrypoint: "approve" }, { name: "Dev mint", entrypoint: "dev_mint" }],
    },
    [TOKEN_SALE]: { methods: [{ name: "Buy GAME", entrypoint: "buy" }] },
    [ENTRY]: { methods: [{ name: "Enter dungeon", entrypoint: "enter" }] },
    [BANK_SYSTEM]: { methods: [{ name: "Bank GOLD", entrypoint: "bank" }] },
  },
};

// Built defensively: the connector probes the RPC synchronously at construction;
// if that throws (offline), the app still renders on the operator account.
function createControllerConnector(): ControllerConnector | null {
  try {
    return new ControllerConnector({
      chains: [{ rpcUrl: SEPOLIA_RPC }],
      defaultChainId: SEPOLIA_CHAIN_ID,
      url: import.meta.env.VITE_KEYCHAIN_URL || undefined,
      policies,
    });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("Cartridge Controller unavailable:", err);
    return null;
  }
}
export const controllerConnector = createControllerConnector();

export type WalletMethod = "operator" | "controller";

type WalletCtx = {
  method: WalletMethod;
  /** Settlement (Sepolia) signer: operator account, or the Controller. */
  l1Account: Signer;
  /** The player identity (L1 address) — also the appchain run key. */
  player: string;
  label: string;
  username?: string;
  connecting: boolean;
  controllerAvailable: boolean;
  connectController: () => Promise<void>;
  useOperator: () => Promise<void>;
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
  const [method, setMethod] = useState<WalletMethod>("operator");
  const [username, setUsername] = useState<string>();
  const [connecting, setConnecting] = useState(false);

  const connectController = async () => {
    if (!controllerConnector) throw new Error("Controller unavailable.");
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

  const useOperator = async () => {
    setMethod("operator");
    setUsername(undefined);
    try {
      await disconnectAsync();
    } catch {
      // not connected — fine
    }
  };

  const usingController = method === "controller" && !!ctrlAccount;
  const l1Account = (usingController ? ctrlAccount : operatorAccount) as unknown as Signer;
  const player = usingController ? (ctrlAddress ?? "") : operatorAccount.address;
  const label = usingController ? (username ?? shortHex(player)) : "Operator account";

  return (
    <Ctx.Provider
      value={{
        method,
        l1Account,
        player,
        label,
        username,
        connecting,
        controllerAvailable: !!controllerConnector,
        connectController,
        useOperator,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function WalletProvider({ children }: PropsWithChildren) {
  const connectors = controllerConnector ? [controllerConnector] : [];
  return (
    <StarknetConfig chains={[sepoliaChain]} connectors={connectors} provider={provider}>
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}

// Re-export so App can show the connected L1 account type if needed.
export type { AccountInterface };
