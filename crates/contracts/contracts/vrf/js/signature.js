#!/usr/bin/env node
import { ec, stark, Signer, shortString } from "starknet";

const main = async () => {

  const vrfAccountPrivateKey = "0x111";
  const msgHash = "0x123";

  const signature = ec.starkCurve.sign(msgHash, vrfAccountPrivateKey);
  console.log("signature", signature);
};

main();
