import { useCallback, useState } from "react";
import { useAccount } from "@starknet-react/core";

import { Button } from "./components/ui/button";

import {
  SIMPLE_CONTRACT_ABI,
  SIMPLE_CONTRACT_ADDRESS,
  VRF_PROVIDER_ADDRESS,
} from "./constants";
import { Spinner } from "./components/ui/spinner";
import { CallData, Contract } from "starknet";

export function Dice() {
  const { account } = useAccount();
  const [submitted, setSubmitted] = useState<boolean>(false);
  const [txnHash, setTxnHash] = useState<string>();
  const [value, setValue] = useState<number | null>(null);

  const rollDiceWithNonce = useCallback(async () => {
    if (!account) return;
    setSubmitted(true);
    setTxnHash(undefined);

    try {
      const result = await account.execute([
        {
          contractAddress: VRF_PROVIDER_ADDRESS,
          entrypoint: "request_random",
          calldata: CallData.compile({
            caller: SIMPLE_CONTRACT_ADDRESS,
            source: { type: 0, address: account.address },
          }),
        },
        {
          contractAddress: SIMPLE_CONTRACT_ADDRESS,
          entrypoint: "roll_dice_with_nonce",
          calldata: [],
        },
      ]);
      setTxnHash(result.transaction_hash);
    } catch (e) {
      console.error(e);
    } finally {
      setSubmitted(false);
    }
  }, [account]);

  const rollDiceWithSalt = useCallback(async () => {
    if (!account) return;
    setSubmitted(true);
    setTxnHash(undefined);

    try {
      const result = await account.execute([
        {
          contractAddress: VRF_PROVIDER_ADDRESS,
          entrypoint: "request_random",
          calldata: CallData.compile({
            caller: SIMPLE_CONTRACT_ADDRESS,
            source: { type: 1, salt: 42 },
          }),
        },
        {
          contractAddress: SIMPLE_CONTRACT_ADDRESS,
          entrypoint: "roll_dice_with_salt",
          calldata: [],
        },
      ]);
      setTxnHash(result.transaction_hash);
    } catch (e) {
      console.error(e);
    } finally {
      setSubmitted(false);
    }
  }, [account]);

  const readValue = useCallback(async () => {
    const myContract = new Contract({
      abi: SIMPLE_CONTRACT_ABI,
      address: SIMPLE_CONTRACT_ADDRESS,
      providerOrAccount: account,
    });

    try {
      const value = (await myContract.get_value()) as number;
      setValue(value);
    } catch (e) {
      console.error(e);
    }
  }, [account]);

  return (
    <>
      <div className="p-4 mt-8 flex flex-row gap-2">
        <Button
          className="w-48 bg-blue-500 text-white dark:bg-blue-600"
          onClick={rollDiceWithNonce}
        >
          Roll Dice (Nonce)
        </Button>
        <Button
          className="w-48 bg-green-500 text-white dark:bg-green-600"
          onClick={rollDiceWithSalt}
        >
          Roll Dice (Salt)
        </Button>
        <Button
          className="w-48 bg-orange-500 text-white dark:bg-orange-600"
          onClick={readValue}
        >
          Read value
        </Button>
      </div>
      <div className="p-4">
        {submitted && (
          <div className="flex flex-row gap-2 items-center">
            <span>Processing ...</span>
            <Spinner />
          </div>
        )}
        {value !== null && <div>Value: {value}</div>}
      </div>
      <div>{txnHash && <div>Transaction Hash: {txnHash}</div>}</div>
    </>
  );
}
