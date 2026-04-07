export const SIMPLE_CONTRACT_ADDRESS =
  "0x072428447f3c8176901c3256ae1b0877943cdb5eac5c85baea24396efff48d8a";

export const SIMPLE_CONTRACT_ABI = [
  {
    type: "interface",
    name: "vrng_test::ISimple",
    items: [
      {
        type: "function",
        name: "get_value",
        inputs: [],
        outputs: [{ type: "core::felt252" }],
        state_mutability: "view",
      },
    ],
  },
];

export const VRF_PROVIDER_ADDRESS =
  "0x15f542e25a4ce31481f986888c179b6e57412be340b8095f72f75a328fbb27b";
