import { type Chain } from "@starknet-react/chains";
import {
	StarknetConfig,
	jsonRpcProvider,
	cartridge,
} from "@starknet-react/core";
import { ControllerConnector } from "@cartridge/connector";
import type { SessionPolicies } from "@cartridge/presets";

import { SIMPLE_CONTRACT_ADDRESS, VRF_PROVIDER_ADDRESS } from "./constants";

const STRK_TOKEN_ADDRESS =
	"0x4718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d";

const KATANA_CHAIN_ID = "0x534e5f4d41494e"; // SN_MAIN
const KATANA_RPC_URL = "http://localhost:5050";

const katana: Chain = {
	id: BigInt(KATANA_CHAIN_ID),
	name: "Katana",
	network: "katana",
	nativeCurrency: {
		address: STRK_TOKEN_ADDRESS,
		name: "STRK Fee Token",
		symbol: "STRK",
		decimals: 18,
	},
	rpcUrls: {
		default: { http: [KATANA_RPC_URL] },
		public: { http: [KATANA_RPC_URL] },
	},
	paymasterRpcUrls: {
		avnu: { http: [KATANA_RPC_URL] },
	},
	testnet: true,
};

// Define session policies
const policies: SessionPolicies = {
	contracts: {
		[VRF_PROVIDER_ADDRESS]: {
			methods: [
				{
					name: "Request random",
					entrypoint: "request_random",
				},
			],
		},
		[SIMPLE_CONTRACT_ADDRESS]: {
			methods: [
				{
					name: "Roll dice with Nonce",
					entrypoint: "roll_dice_with_nonce",
				},
				{
					name: "Roll dice with Salt",
					entrypoint: "roll_dice_with_salt",
				},
			],
		},
	},
};

// Initialize the connector
const connector = new ControllerConnector({
	policies,
	propagateSessionErrors: true,
	chains: [{ rpcUrl: KATANA_RPC_URL }],
	defaultChainId: KATANA_CHAIN_ID,
});

// Configure RPC provider
const provider = jsonRpcProvider({
	rpc: (_chain: Chain) => {
		return { nodeUrl: KATANA_RPC_URL };
	},
});

export function StarknetProvider({ children }: { children: React.ReactNode }) {
	return (
		<StarknetConfig
			autoConnect
			defaultChainId={BigInt(KATANA_CHAIN_ID)}
			chains={[katana]}
			provider={provider}
			connectors={[connector]}
			explorer={cartridge}
		>
			{children}
		</StarknetConfig>
	);
}
