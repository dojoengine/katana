// Wallet layer: nothing is connected by default — the player must connect a
// wallet before any operation. Two choices: the hardcoded **dev accounts**
// (offline, one-click) or a **Cartridge Controller** (one identity that signs on
// BOTH chains — buy + bank on L1, roll on L2 — at the same address; a
// hosted-keychain wallet needing `CONTROLLER=1 ./up.sh` + a login; see README).

import { createContext, useContext, useEffect, useState, type PropsWithChildren } from "react";
import { constants, shortString } from "starknet";
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
//
// Only include contracts with a real address. An unconfigured deployment is `0x0`,
// and a 0x0 entry is a malformed policy: the keychain merklizes it inconsistently
// and the account's session validation then trips on a policy-count/proof-length
// mismatch ("session/length-mismatch").
const allContracts: Record<string, { methods: { name: string; entrypoint: string }[] }> = {
  [STORE]: { methods: [{ name: "Buy game", entrypoint: "buy_game" }] },
  [SCORE_REGISTRY]: { methods: [{ name: "Bank score", entrypoint: "claim_score" }] },
  [GAME]: { methods: [{ name: "Roll", entrypoint: "play_game" }] },
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
      // Normally the settlement chain. Set VITE_DEFAULT_APPCHAIN=1 to make the keychain
      // sit on the appchain — needed once to surface the keychain's account-UPGRADE screen
      // for the appchain account (on the settlement chain it already reads as up-to-date).
      defaultChainId: import.meta.env.VITE_DEFAULT_APPCHAIN === "1" ? APPCHAIN_CHAIN_ID : SETTLEMENT_CHAIN_ID,
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

// Persist the chosen signer across reloads — nothing is auto-connected on a first
// visit, but a prior connection is restored until the user disconnects.
const STORE_KEY = "ccg.wallet.method";
function loadMethod(): WalletMethod | null {
  try {
    const v = localStorage.getItem(STORE_KEY);
    return v === "dev" || v === "controller" ? v : null;
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
  /** null = no wallet connected (the default). */
  method: WalletMethod | null;
  /** True once a dev account or a Controller session is active. */
  connected: boolean;
  /** L1 signer (buy + bank): dev account or the Controller — null when disconnected. */
  l1Account: Signer | null;
  /** L2 signer (roll): dev appchain account or the Controller — null when disconnected. */
  l2Account: Signer | null;
  l1Address: string;
  /** Address that `play_game` records as the player (and that `claim_score` must
   *  consume the L2→L1 message for): the dev appchain account, or the Controller
   *  (same address on both chains). "" when disconnected. */
  playerAddress: string;
  /** Connected-account display: Controller → username, dev → short address. "" when disconnected. */
  label: string;
  username?: string;
  connecting: boolean;
  /** False when the Controller connector couldn't be built (stack down). */
  controllerAvailable: boolean;
  connectController: () => Promise<void>;
  useDevAccount: () => Promise<void>;
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
  // The local dev account restores synchronously here (no "login" flash); a persisted
  // Controller is reconnected by starknet-react's autoConnect in the effect below.
  const [method, setMethod] = useState<WalletMethod | null>(() => (loadMethod() === "dev" ? "dev" : null));
  const [username, setUsername] = useState<string>();
  const [connecting, setConnecting] = useState(false);

  const safeDisconnect = async () => {
    try {
      await disconnectAsync();
    } catch {
      // not connected — fine
    }
  };

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
      saveMethod("controller");
    } catch (err) {
      // The user dismissed the keychain, or connect failed. Reset to a clean disconnected
      // state so the chip shows "connect" and a retry works.
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

  const useDevAccount = async () => {
    setUsername(undefined);
    await safeDisconnect();
    setMethod("dev");
    saveMethod("dev");
  };

  const disconnect = async () => {
    setMethod(null);
    saveMethod(null);
    setUsername(undefined);
    await safeDisconnect();
  };

  // Restore the previously chosen signer on load — silently, with no keychain prompt.
  // The dev account is a local key (restored by the useState initializer above). A saved
  // Controller is reconnected by starknet-react's autoConnect (it reuses the session
  // without a popup); flip to "controller" once that account comes back.
  useEffect(() => {
    if (!ctrlAccount || method !== null) return;
    if (loadMethod() === "controller" && controllerConnector) {
      setMethod("controller");
      controllerConnector.username()?.then(
        (u) => setUsername(u),
        () => setUsername(undefined),
      );
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctrlAccount]);

  // Nothing is connected by default. The Controller is the active signer once a
  // method=controller session exists; the dev account once method=dev.
  const usingController = method === "controller" && !!ctrlAccount;
  const usingDev = method === "dev";
  const connected = usingController || usingDev;

  // L1 signer (buy + bank). For the Controller we switch it to the SETTLEMENT
  // chain before executing — symmetric with the roll switching to GAMECHAIN. A
  // prior roll (or any state) may have left the keychain on the appchain, where
  // the L1 store/score contracts aren't deployed, so we can't rely on the keychain
  // happening to be on settlement.
  const l1Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          const ctrl = controllerConnector!.controller;
          await ctrl.switchStarknetChain(SETTLEMENT_CHAIN_ID);
          return await (ctrl.account ?? ctrlAccount!).execute(calls);
        },
      }
    : usingDev
      ? settlementAccount
      : null;
  const l1Address = usingController ? (ctrlAddress ?? "") : usingDev ? settlementAccount.address : "";
  // The roll's caller: the Controller (same address on both chains) when connected,
  // else the dev appchain account. claim_score must consume the score message for
  // this address, not the L1 signer (which differs in dev mode).
  const playerAddress = usingController ? (ctrlAddress ?? "") : usingDev ? appchainAccount.address : "";
  // Connected-account display: Controller → username (fallback short address), dev → short address.
  const label = usingController ? (username ?? shortHex(l1Address)) : usingDev ? shortHex(l1Address) : "";

  // L2 signer. The Controller's default chain is the settlement layer (for
  // buy/bank), so for a roll we switch it to the appchain, execute, then switch
  // back — keeping the same Controller identity/address across both chains.
  //
  // We execute via the raw `controller.account` (a ControllerAccount), not the
  // `useAccount()` value: starknet-react's WalletAccount is bound to the
  // *settlement* RPC, so its client-side fee estimate hits :5050 and fails for
  // the appchain game contract. ControllerAccount.execute delegates straight to
  // the keychain, which runs against whatever chain we just switched it to.
  const l2Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          // usingController implies a live connection, so the connector exists.
          const ctrl = controllerConnector!.controller;
          // switchStarknetChain returns false (and stays on the settlement chain) when the
          // keychain can't reach the appchain RPC — e.g. the hosted x.cartridge.gg iframe
          // blocked from http://localhost by Chrome Private Network Access. If we proceed,
          // the controller account (pinned to the settlement RPC at connect time) runs the
          // tx on Sepolia, which then fails calling an appchain contract that doesn't exist
          // there. Fail loudly instead.
          const ok = await ctrl.switchStarknetChain(APPCHAIN_CHAIN_ID);
          if (!ok) {
            throw new Error(
              "Controller could not switch to the appchain (GAMECHAIN). The keychain can't reach " +
                `${APPCHAIN_RPC} — enable chrome://flags/#local-network-access-check, or use the ` +
                "self-hosted keychain (VITE_KEYCHAIN_URL).",
            );
          }
          // Do NOT switch back to the settlement chain afterward. The keychain's
          // balance-change preview re-simulates on whatever chain is active; switching back
          // to Sepolia here makes it re-run the appchain call against Sepolia (where the
          // game contract doesn't exist) and surface a bogus "not deployed" error. The L1
          // signer switches to settlement itself, so leaving the controller on the appchain
          // is correct.
          return await (ctrl.account ?? ctrlAccount!).execute(calls);
        },
      }
    : usingDev
      ? appchainAccount
      : null;

  return (
    <Ctx.Provider
      value={{
        method,
        connected,
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
        disconnect,
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
      autoConnect
    >
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}
