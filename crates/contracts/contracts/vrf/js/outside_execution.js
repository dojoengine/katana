#!/usr/bin/env node
import { TypedDataRevision } from "starknet";
import { typedData } from "starknet";
import {
  ec,
  stark,
  Signer,
  hash,
  shortString,
  selector,
  constants,
  outsideExecution,
  OutsideExecutionVersion,
} from "starknet";

const main = async () => {
  const ANY_CALLER = shortString.encodeShortString("ANY_CALLER");
  const VRF_ACCOUNT = shortString.encodeShortString("VRF_ACCOUNT");
  const CONSUMER_ACCOUNT = shortString.encodeShortString("CONSUMER_ACCOUNT");
  const CONSUMER = shortString.encodeShortString("CONSUMER");

  // VRF_ACCOUNT
  const vrfAccountPrivateKey = "0x111";
  const vrfAccountPublicKey = ec.starkCurve.getStarkKey(vrfAccountPrivateKey);
  const vrfAccountSigner = new Signer(vrfAccountPrivateKey);

  // CONSUMER_ACCOUNT
  const consumerAccountPrivateKey = "0x222";
  const consumerAccountPublicKey = ec.starkCurve.getStarkKey(
    consumerAccountPrivateKey
  );
  const consumerAccountSigner = new Signer(consumerAccountPrivateKey);

  const requestRandomCall = {
    contractAddress: VRF_ACCOUNT,
    entrypoint: "request_random",
    calldata: [CONSUMER, 0x0, CONSUMER_ACCOUNT],
  };
  const consumerCall = {
    contractAddress: CONSUMER,
    entrypoint: "dice",
    calldata: [],
  };

  const callOptions = {
    caller: ANY_CALLER,
    execute_after: 0,
    execute_before: 999,
  };

  const nonce = 0;

  const message = outsideExecution.getTypedData(
    constants.StarknetChainId.SN_SEPOLIA,
    callOptions,
    nonce,
    [requestRandomCall, consumerCall],
    OutsideExecutionVersion.V2
  );

  console.log("message", JSON.stringify(message, 0, 2));

  const messageHash = typedData.getMessageHash(message, CONSUMER_ACCOUNT);

  console.log("messageHash", messageHash);

  const signature = await consumerAccountSigner.signMessage(
    message,
    CONSUMER_ACCOUNT
  );

  console.log(signature);
};

main();
