// Wallet layer: who signs the demo's transactions.
//
// Default is the **operator account** (a real funded Sepolia account from
// deployments.json) — buy / enter / play / bank work with no login. A "Login" choice
// swaps in a **Cartridge Controller**: ONE identity that signs on BOTH chains — buy /
// enter / bank on Sepolia AND the dungeon play actions on the local appchain — at the
// same address. The appchain leg needs `CONTROLLER=1 ./up.sh` + a self-hosted keychain
// (the hosted keychain can't reach a local appchain); see docs/controller.md.

import { createContext, useContext, useEffect, useState, type PropsWithChildren } from "react";
import { type AccountInterface, constants, shortString } from "starknet";
import ControllerConnector from "@cartridge/connector/controller";
import { StarknetConfig, jsonRpcProvider, useAccount, useConnect, useDisconnect } from "@starknet-react/core";
import type { Chain } from "@starknet-react/chains";
import {
  SETTLEMENT_RPC,
  APPCHAIN_RPC,
  SETTLEMENT_CHAIN_ID,
  SETTLEMENT_NETWORK,
  SETTLEMENT_NAME,
  USDC,
  GAME_TOKEN,
  TOKEN_SALE,
  ENTRY,
  BANK_SYSTEM,
  GAME_SYSTEM,
  operatorAccount,
  appchainAccount,
  shortHex,
  type Signer,
} from "./chain.ts";

// Chain ids (felt). Settlement is the configured network (Sepolia by default, or
// mainnet); the appchain runs `katana init rollup --id DUNGEON`. switchStarknetChain
// takes these strings.
const CHAIN_ID = SETTLEMENT_CHAIN_ID === "SN_MAIN" ? constants.StarknetChainId.SN_MAIN : constants.StarknetChainId.SN_SEPOLIA;
const APPCHAIN_CHAIN_ID = shortString.encodeShortString("DUNGEON");
const STRK = "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

// starknet-react requires a paymaster provider per chain (its default reads
// paymasterRpcUrls.avnu.http); the Controller runs its own, so point it at the rpc.
const mkChain = (id: string, name: string, network: string, rpc: string): Chain => ({
  id: BigInt(id),
  network,
  name,
  nativeCurrency: { name: "Stark", symbol: "STRK", decimals: 18, address: STRK },
  rpcUrls: { default: { http: [rpc] }, public: { http: [rpc] } },
  paymasterRpcUrls: { avnu: { http: [rpc] } },
});
const settlementChain = mkChain(CHAIN_ID, SETTLEMENT_NAME, SETTLEMENT_NETWORK, SETTLEMENT_RPC);
const appchainChain = mkChain(APPCHAIN_CHAIN_ID, "Dungeon Appchain", "dungeon-appchain", APPCHAIN_RPC);

const provider = jsonRpcProvider({
  rpc: (c: Chain) => ({ nodeUrl: c.id === appchainChain.id ? APPCHAIN_RPC : SETTLEMENT_RPC }),
});

// Session policies: scope the Controller session to the demo's entrypoints — buy /
// enter / bank on Sepolia and the play actions on the appchain — so they're session
// calls rather than per-tx popups.
const policies = {
  contracts: {
    [USDC]: { methods: [{ name: "Approve", entrypoint: "approve" }] },
    [GAME_TOKEN]: {
      methods: [{ name: "Approve", entrypoint: "approve" }, { name: "Dev mint", entrypoint: "dev_mint" }],
    },
    [TOKEN_SALE]: { methods: [{ name: "Buy GAME", entrypoint: "buy" }] },
    [ENTRY]: { methods: [{ name: "Enter dungeon", entrypoint: "enter" }] },
    [BANK_SYSTEM]: { methods: [{ name: "Bank GOLD", entrypoint: "bank" }] },
    [GAME_SYSTEM]: {
      methods: [
        { name: "Move", entrypoint: "move_room" },
        { name: "Attack", entrypoint: "attack" },
        { name: "Loot", entrypoint: "loot" },
        { name: "Use item", entrypoint: "use_item" },
        { name: "Extract", entrypoint: "extract" },
        { name: "Withdraw", entrypoint: "withdraw" },
      ],
    },
  },
};

// Built defensively: the connector probes each RPC synchronously at construction; if
// that throws (a node offline), the app still renders on the operator account.
function createControllerConnector(): ControllerConnector | null {
  try {
    return new ControllerConnector({
      chains: [{ rpcUrl: SETTLEMENT_RPC }, { rpcUrl: APPCHAIN_RPC }],
      defaultChainId: CHAIN_ID,
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

// Persist the chosen signer across reloads — nothing is auto-connected on a first
// visit, but a prior connection is restored until the user disconnects.
const STORE_KEY = "ccd.wallet.method";
function loadMethod(): WalletMethod | null {
  try {
    const v = localStorage.getItem(STORE_KEY);
    return v === "operator" || v === "controller" ? v : null;
  } catch {
    return null;
  }
}
function saveMethod(m: WalletMethod | null) {
  try {
    if (m) localStorage.setItem(STORE_KEY, m);
    else localStorage.removeItem(STORE_KEY);
  } catch {
    // storage unavailable — fine, just no persistence
  }
}

type WalletCtx = {
  /** null = nothing connected (the user disconnected); pick a method to act again. */
  method: WalletMethod | null;
  /** True only when a Controller is actually connected (an account exists). The
   *  operator default is NOT "connected" for the login toggle's purposes. */
  connected: boolean;
  /** Settlement (Sepolia) signer — null when disconnected. */
  l1Account: Signer | null;
  /** Appchain signer for play actions (move/attack/loot/use/extract/withdraw):
   *  the local dev account, or the Controller switched to the appchain — null when
   *  disconnected. */
  l2Account: Signer | null;
  /** The player identity (Sepolia address) — also the appchain run/vault key. "" when
   *  disconnected. */
  player: string;
  label: string;
  username?: string;
  connecting: boolean;
  controllerAvailable: boolean;
  connectController: () => Promise<void>;
  useOperator: () => Promise<void>;
  disconnect: () => Promise<void>;
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
  // Nothing connected by default. A persisted operator restores synchronously here (no
  // "login" flash); a persisted Controller is reconnected in the mount effect below.
  const [method, setMethod] = useState<WalletMethod | null>(() => (loadMethod() === "operator" ? "operator" : null));
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
      saveMethod("controller");
    } catch (err) {
      // The user dismissed the keychain, or connect failed. Reset to a clean
      // disconnected state so the chip shows "login" and a retry works.
      // eslint-disable-next-line no-console
      console.warn("Controller connect cancelled/failed:", err);
      try {
        await disconnectAsync();
      } catch {
        // wasn't connected — fine
      }
      setMethod(null);
      saveMethod(null);
      setUsername(undefined);
    } finally {
      setConnecting(false);
    }
  };

  const useOperator = async () => {
    setMethod("operator");
    saveMethod("operator");
    setUsername(undefined);
    try {
      await disconnectAsync();
    } catch {
      // not connected — fine
    }
  };

  // Fully disconnect: no operator, no Controller. The header shows "login" and the
  // action handlers prompt to reconnect (open the wallet modal) until a method is picked.
  const disconnect = async () => {
    setMethod(null);
    saveMethod(null);
    setUsername(undefined);
    try {
      await disconnectAsync();
    } catch {
      // wasn't connected — fine
    }
  };

  // Restore the previously chosen signer on load — silently, with NO keychain prompt.
  // The operator is a local key, so set it directly. The Controller is reconnected by
  // starknet-react's autoConnect (it reuses the keychain session without a popup); we
  // flip to "controller" once that account comes back. A first visit, or an explicit
  // disconnect, restores nothing → the chip stays on "login".
  useEffect(() => {
    if (loadMethod() === "operator") setMethod("operator");
    // mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    const cc = controllerConnector;
    if (!cc) return;
    if (loadMethod() !== "controller" || !ctrlAccount || method !== null) return;
    setMethod("controller");
    cc.username()?.then(
      (u) => setUsername(u),
      () => setUsername(undefined),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctrlAccount]);

  const usingController = method === "controller" && !!ctrlAccount;

  // L1 signer (buy / enter / bank). For the Controller, switch to the settlement chain
  // first — a prior play action may have left the keychain on the appchain, where the
  // L1 contracts aren't deployed.
  const l1Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          const ctrl = controllerConnector!.controller;
          await ctrl.switchStarknetChain(CHAIN_ID);
          return await (ctrl.account ?? ctrlAccount!).execute(calls);
        },
      }
    : method === "operator"
      ? operatorAccount
      : null;

  // L2 signer (play). Switch the Controller to the appchain, execute, then switch back
  // to settlement (the default) for the next L1 op. Execute via the raw
  // controller.account, NOT starknet-react's account — the latter is bound to the
  // settlement RPC, so its client-side fee estimate for the appchain game contract
  // would hit Sepolia and fail.
  const l2Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          const ctrl = controllerConnector!.controller;
          await ctrl.switchStarknetChain(APPCHAIN_CHAIN_ID);
          try {
            return await (ctrl.account ?? ctrlAccount!).execute(calls);
          } finally {
            await ctrl.switchStarknetChain(CHAIN_ID);
          }
        },
      }
    : method === "operator"
      ? appchainAccount
      : null;

  // The player: the Controller (same address on both chains) when connected, else the
  // operator. The L1 `enter` mints the run for this address, and play/withdraw key on
  // it — so a Controller plays and banks the runs it entered.
  const player = usingController ? (ctrlAddress ?? "") : method === "operator" ? operatorAccount.address : "";
  const label = usingController ? (username ?? shortHex(player)) : method === "operator" ? "Operator account" : "not connected";

  return (
    <Ctx.Provider
      value={{
        method,
        connected: usingController,
        l1Account,
        l2Account,
        player,
        label,
        username,
        connecting,
        controllerAvailable: !!controllerConnector,
        connectController,
        useOperator,
        disconnect,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function WalletProvider({ children }: PropsWithChildren) {
  const connectors = controllerConnector ? [controllerConnector] : [];
  return (
    <StarknetConfig chains={[settlementChain, appchainChain]} connectors={connectors} provider={provider} autoConnect>
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}

// Re-export so App can show the connected account type if needed.
export type { AccountInterface };
