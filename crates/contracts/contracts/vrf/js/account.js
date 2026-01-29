#!/usr/bin/env node
import { ec, stark } from "starknet";

const main = async () => {
  const privateKey1 = "0x111";
  const starknetPublicKey1 = ec.starkCurve.getStarkKey(privateKey1);

  const privateKey2 = "0x222";
  const starknetPublicKey2 = ec.starkCurve.getStarkKey(privateKey2);

  console.log("starknetPublicKey1", starknetPublicKey1);
  console.log("starknetPublicKey2", starknetPublicKey2);

  // starknetPublicKey1 0x14584bef56c98fbb91aba84c20724937d5b5d2d6e5a49b60e6c3a19696fad5f
  // starknetPublicKey2 0x5cba218680f68130296ac34ed343d6186a98744c6ef66c39345fdaefe06c4d5
};

main();
