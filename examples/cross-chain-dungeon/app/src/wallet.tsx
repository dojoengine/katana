// Wallet layer: who signs the demo's transactions.
//
// Nothing is connected by default. A "login" choice picks a signer:
//   - **Cartridge Controller** (the primary option): ONE identity that signs on BOTH
//     chains — buy / enter / bank on Sepolia AND the play actions on the local appchain.
//   - **Argent X / Braavos** (injected): your real Sepolia wallet signs buy/enter/bank;
//     the appchain play actions fall back to the dev key (only the Controller can sign the
//     local appchain). See docs/controller.md.

import { createContext, useContext, useEffect, useState, type PropsWithChildren } from "react";
import { type AccountInterface, constants, shortString } from "starknet";
import ControllerConnector from "@cartridge/connector/controller";
import {
  StarknetConfig,
  argent,
  braavos,
  jsonRpcProvider,
  useAccount,
  useConnect,
  useDisconnect,
  type Connector,
} from "@starknet-react/core";
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
//
// Only include contracts with a real address. An unconfigured deployment is `0x0`
// (e.g. USDC, which this demo doesn't use — GAME is dev-minted), and a 0x0 entry is a
// malformed policy: the keychain merklizes it inconsistently and the account's session
// validation then trips on a policy-count/proof-length mismatch ("session/length-mismatch").
const allContracts: Record<string, { methods: { name: string; entrypoint: string }[] }> = {
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
};
const policies = {
  contracts: Object.fromEntries(
    Object.entries(allContracts).filter(([addr]) => {
      try {
        return BigInt(addr) !== 0n;
      } catch {
        return false;
      }
    }),
  ),
};

// Built defensively: the connector probes each RPC synchronously at construction; if
// that throws (a node offline), the app still renders with the other options.
function createControllerConnector(): ControllerConnector | null {
  try {
    return new ControllerConnector({
      chains: [{ rpcUrl: SETTLEMENT_RPC }, { rpcUrl: APPCHAIN_RPC }],
      // Normally the settlement chain. Set VITE_DEFAULT_APPCHAIN=1 to make the keychain
      // sit on the appchain — needed once to surface the keychain's account-UPGRADE
      // screen for the appchain account (its upgrade gate reads the controller's current
      // chain; on the settlement chain the account already reads as up-to-date).
      defaultChainId: import.meta.env.VITE_DEFAULT_APPCHAIN === "1" ? APPCHAIN_CHAIN_ID : CHAIN_ID,
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

// Other Starknet wallets (browser extensions). They sign Sepolia (buy/enter/bank); the
// appchain play actions fall back to the local dev account (only the Controller signs the
// local appchain). Each is usable only if its extension is installed.
const argentConnector = argent();
const braavosConnector = braavos();
export type InjectedKind = "argent" | "braavos";

export type WalletMethod = "controller" | "injected";

// Persist the chosen signer across reloads — nothing is auto-connected on a first visit,
// but a prior connection is restored until the user disconnects.
const STORE_KEY = "ccd.wallet.method";
function loadMethod(): WalletMethod | null {
  try {
    const v = localStorage.getItem(STORE_KEY);
    return v === "controller" || v === "injected" ? v : null;
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
  /** null = nothing connected; pick a method to act. */
  method: WalletMethod | null;
  /** True only when a Cartridge Controller is actually connected. */
  connected: boolean;
  /** Settlement (Sepolia) signer — null when disconnected. */
  l1Account: Signer | null;
  /** Appchain signer for the play actions: the Controller (switched to the appchain) or
   *  the local dev key (operator + injected wallets) — null when disconnected. */
  l2Account: Signer | null;
  /** The player identity (Sepolia address) — also the appchain run/vault key. "" when
   *  disconnected. */
  player: string;
  label: string;
  username?: string;
  connecting: boolean;
  controllerAvailable: boolean;
  connectController: () => Promise<void>;
  connectInjected: (which: InjectedKind) => Promise<void>;
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
  const { account: ctrlAccount, address: ctrlAddress, connector: activeConnector } = useAccount();
  // Nothing connected by default. A persisted Controller / injected wallet is
  // reconnected by autoConnect in the mount effect below.
  const [method, setMethod] = useState<WalletMethod | null>(null);
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
      // The user dismissed the keychain, or connect failed. Reset to a clean disconnected
      // state so the chip shows "login" and a retry works.
      // eslint-disable-next-line no-console
      console.warn("Controller connect cancelled/failed:", err);
      await safeDisconnect();
      setMethod(null);
      saveMethod(null);
      setUsername(undefined);
    } finally {
      setConnecting(false);
    }
  };

  // Connect a browser-extension wallet (Argent X / Braavos). It signs Sepolia; the play
  // actions use the dev key (l2Account below).
  const connectInjected = async (which: InjectedKind) => {
    const connector = which === "argent" ? argentConnector : braavosConnector;
    setConnecting(true);
    try {
      await connectAsync({ connector });
      setMethod("injected");
      saveMethod("injected");
      setUsername(undefined); // the label uses the live connector name
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn(`${which} connect cancelled/failed:`, err);
      await safeDisconnect();
      setMethod(null);
      saveMethod(null);
    } finally {
      setConnecting(false);
    }
  };

  const safeDisconnect = async () => {
    try {
      await disconnectAsync();
    } catch {
      // wasn't connected — fine
    }
  };

  // Fully disconnect: no signer at all. The header shows "login" and the action handlers
  // prompt to reconnect (open the wallet modal) until a method is picked.
  const disconnect = async () => {
    setMethod(null);
    saveMethod(null);
    setUsername(undefined);
    await safeDisconnect();
  };

  // Restore the previously chosen signer on load — silently, with NO keychain prompt.
  // The Controller / injected wallet is reconnected by starknet-react's autoConnect (it
  // reuses the session without a popup); we flip to the saved method once that account
  // comes back.
  useEffect(() => {
    if (!ctrlAccount || method !== null) return;
    const saved = loadMethod();
    if (saved === "controller" && controllerConnector) {
      setMethod("controller");
      controllerConnector.username()?.then(
        (u) => setUsername(u),
        () => setUsername(undefined),
      );
    } else if (saved === "injected") {
      setMethod("injected");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctrlAccount]);

  const usingController = method === "controller" && !!ctrlAccount;
  const usingInjected = method === "injected" && !!ctrlAccount;

  // L1 signer (buy / enter / bank). The Controller switches to the settlement chain first
  // (a prior play may have left the keychain on the appchain); an injected wallet signs
  // Sepolia directly.
  const l1Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          const ctrl = controllerConnector!.controller;
          await ctrl.switchStarknetChain(CHAIN_ID);
          return await (ctrl.account ?? ctrlAccount!).execute(calls);
        },
      }
    : usingInjected
      ? (ctrlAccount ?? null)
      : null;

  // L2 signer (play). The Controller switches to the appchain, executes, then switches
  // back to settlement for the next L1 op (via the raw controller.account, NOT
  // starknet-react's account — the latter is pinned to the settlement RPC, so its appchain
  // fee estimate would hit Sepolia and fail). Injected wallets can't sign the local
  // appchain, so they use the dev key.
  const l2Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          const ctrl = controllerConnector!.controller;
          // switchStarknetChain returns false (and reverts to the settlement chain) when
          // the keychain can't reach the appchain RPC — e.g. the hosted x.cartridge.gg
          // iframe being blocked from http://localhost by Chrome Private Network Access.
          // If we proceed anyway the controller account (pinned to the settlement RPC at
          // connect time) runs the tx on Sepolia, which then fails calling an appchain
          // contract that doesn't exist there. Fail loudly instead.
          const ok = await ctrl.switchStarknetChain(APPCHAIN_CHAIN_ID);
          if (!ok) {
            throw new Error(
              "Controller could not switch to the appchain (DUNGEON). The keychain can't reach " +
                "http://localhost:5070 — enable chrome://flags/#local-network-access-check, or use " +
                "the self-hosted keychain (VITE_KEYCHAIN_URL=https://localhost:3010).",
            );
          }
          // Do NOT switch back to the settlement chain afterward. The keychain's
          // balance-change preview re-simulates on whatever chain is active; switching
          // back to Sepolia here makes it re-run the appchain call against Sepolia
          // (where the game contract doesn't exist) and surface a bogus "not deployed"
          // error. The L1 signer switches to the settlement chain itself, so leaving
          // the controller on the appchain is correct.
          return await (ctrl.account ?? ctrlAccount!).execute(calls);
        },
      }
    : usingInjected
      ? appchainAccount
      : null;

  // The player: the connected wallet's address. The L1 `enter` mints the run for this
  // address, and play/withdraw key on it.
  const player = usingController || usingInjected ? (ctrlAddress ?? "") : "";
  const label = usingController
    ? (username ?? shortHex(player))
    : usingInjected
      ? (activeConnector?.name ?? shortHex(player))
      : "not connected";

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
        connectInjected,
        disconnect,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function WalletProvider({ children }: PropsWithChildren) {
  const connectors = [controllerConnector, argentConnector, braavosConnector].filter((c) => c != null) as Connector[];
  return (
    <StarknetConfig chains={[settlementChain, appchainChain]} connectors={connectors} provider={provider} autoConnect>
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}

// Re-export so App can show the connected account type if needed.
export type { AccountInterface };
