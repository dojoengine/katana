export const SIMPLE_CONTRACT_ADDRESS =
	"0x0481f591c76103dbdfa080bc29059860802d54c9724702aa9abb1c49766dd363";

export const SIMPLE_CONTRACT_ABI = [
	{
		type: "interface",
		name: "vrng_test::ISimple",
		items: [
			{
				type: "function",
				name: "get",
				inputs: [],
				outputs: [{ type: "core::felt252" }],
				state_mutability: "view",
			},
		],
	},
];

export const VRF_ACCOUNT_ADDRESS =
	"0x4da58dd0cf16f001b618f5461632cd3cb1d3506254a5c5c62dce6b037de7490";
