// Wallet layer: who signs the demo's transactions.
//
// Nothing is connected by default. The **Cartridge Controller** is the one and only
// login: ONE identity that signs on BOTH chains — buy / enter / bank on Sepolia AND
// the play actions on the local appchain. See docs/controller.md.

import { createContext, useContext, useEffect, useState, type PropsWithChildren } from "react";
import { type AccountInterface, constants, shortString } from "starknet";
import ControllerConnector from "@cartridge/connector/controller";
import { StarknetConfig, jsonRpcProvider, useAccount, useConnect, useDisconnect, type Connector } from "@starknet-react/core";
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

export type WalletMethod = "controller";

// Persist the chosen signer across reloads — nothing is auto-connected on a first visit,
// but a prior connection is restored until the user disconnects.
const STORE_KEY = "ccd.wallet.method";
function loadMethod(): WalletMethod | null {
  try {
    return localStorage.getItem(STORE_KEY) === "controller" ? "controller" : null;
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
  /** Appchain signer for the play actions: the Controller, switched to the appchain —
   *  null when disconnected. */
  l2Account: Signer | null;
  /** The player identity (Sepolia address) — also the appchain run/vault key. "" when
   *  disconnected. */
  player: string;
  label: string;
  username?: string;
  connecting: boolean;
  controllerAvailable: boolean;
  connectController: () => Promise<void>;
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
  // Nothing connected by default. A persisted Controller is reconnected by autoConnect
  // in the mount effect below.
  const [method, setMethod] = useState<WalletMethod | null>(null);
  const [username, setUsername] = useState<string>();
  const [connecting, setConnecting] = useState(false);

  // Resolves once the keychain modal (div#controller, injected by @cartridge/controller)
  // has been seen open and then closed again. Dismissing the keychain without logging in
  // leaves the connect() promise PENDING FOREVER (the iframe RPC never settles and the
  // library exposes no cancel signal), so the modal's DOM state is the only way to tell
  // the user backed out — without it the UI is stuck on "Connecting…".
  const keychainDismissed = (stop: { current: boolean }) =>
    new Promise<void>((resolve) => {
      let wasOpen = false;
      const timer = setInterval(() => {
        if (stop.current) {
          clearInterval(timer);
          return; // connect settled — never resolve
        }
        const el = document.getElementById("controller");
        const open = !!el && el.style.display !== "none" && el.style.opacity !== "0";
        if (open) wasOpen = true;
        else if (wasOpen) {
          clearInterval(timer);
          resolve();
        }
      }, 200);
    });

  const connectController = async () => {
    if (!controllerConnector) throw new Error("Controller unavailable.");
    if (connecting) return; // login is auto-prompted from several places — one keychain at a time
    setConnecting(true);
    const stop = { current: false };
    try {
      const connect = connectAsync({ connector: controllerConnector });
      let settled = false;
      connect.then(
        () => (settled = true),
        () => (settled = true),
      );
      const dismissed = await Promise.race([connect.then(() => false), keychainDismissed(stop).then(() => true)]);
      if (dismissed) {
        // A SUCCESSFUL login also closes the modal moments before connect resolves —
        // grace-wait before declaring it a user cancel.
        await new Promise((r) => setTimeout(r, 1500));
        if (!settled) throw new Error("keychain dismissed without logging in");
      }
      await connect;
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
      stop.current = true;
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
  // re-prompt the Controller login until connected.
  const disconnect = async () => {
    setMethod(null);
    saveMethod(null);
    setUsername(undefined);
    await safeDisconnect();
  };

  // Auto-reconnect the previously connected Controller on load — silently, with NO
  // keychain prompt — unless the user explicitly disconnected (which clears the saved
  // method). starknet-react's own autoConnect can't do this: its ready() check probes
  // wallet_getPermissions, which the Controller provider doesn't implement, so it
  // always bails. Instead probe the keychain directly (no modal); if a session is
  // live, finish the connect — controller.connect() returns the probed account
  // immediately. No session → stay logged out; the Login button connects manually.
  // GOTCHA: the probe only works when the keychain initializes against a PUBLICLY
  // reachable rpc. With VITE_DEFAULT_APPCHAIN=1 (the one-time appchain-upgrade flag)
  // the keychain pins to http://localhost:5070, and a hidden, gesture-less iframe
  // can't reach localhost under Chrome's Local Network Access rules — the probe then
  // reports no session even though one exists.
  useEffect(() => {
    if (loadMethod() !== "controller" || !controllerConnector) return;
    let stale = false;
    setConnecting(true);
    // Don't let a hung keychain handshake wedge the UI on "connecting…".
    const timeout = new Promise<undefined>((r) => setTimeout(() => r(undefined), 10_000));
    Promise.race([controllerConnector.controller.probe(), timeout])
      .then((acct) => (!stale && acct ? connectAsync({ connector: controllerConnector! }) : undefined))
      .catch(() => undefined) // no live session — stay logged out
      .finally(() => !stale && setConnecting(false));
    return () => {
      stale = true;
    };
    // mount only
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Flip to the saved method once the reconnected account lands (also fetches the
  // username for the chip label).
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

  const usingController = method === "controller" && !!ctrlAccount;

  // L1 signer (buy / enter / bank). The Controller switches to the settlement chain first
  // (a prior play may have left the keychain on the appchain).
  const l1Account: Signer | null = usingController
    ? {
        execute: async (calls) => {
          const ctrl = controllerConnector!.controller;
          await ctrl.switchStarknetChain(CHAIN_ID);
          return await (ctrl.account ?? ctrlAccount!).execute(calls);
        },
      }
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
    : null;

  // The player: the connected Controller's address. The L1 `enter` mints the run for
  // this address, and play/withdraw key on it.
  const player = usingController ? (ctrlAddress ?? "") : "";
  const label = usingController ? (username ?? shortHex(player)) : "not connected";

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
        disconnect,
      }}
    >
      {children}
    </Ctx.Provider>
  );
}

export function WalletProvider({ children }: PropsWithChildren) {
  const connectors = [controllerConnector].filter((c) => c != null) as Connector[];
  return (
    <StarknetConfig chains={[settlementChain, appchainChain]} connectors={connectors} provider={provider} autoConnect>
      <WalletInner>{children}</WalletInner>
    </StarknetConfig>
  );
}

// Re-export so App can show the connected account type if needed.
export type { AccountInterface };
