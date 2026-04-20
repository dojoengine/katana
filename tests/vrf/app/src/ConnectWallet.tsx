import { useEffect, useState } from "react";
import { useAccount, useConnect, useDisconnect } from "@starknet-react/core";
import { ControllerConnector } from "@cartridge/connector";
import { Button } from "./components/ui/button";
import { Badge } from "./components/ui/badge";

export function ConnectWallet() {
  const { connect, connectors } = useConnect();
  const { disconnect } = useDisconnect();
  const { address } = useAccount();
  const controller = connectors[0] as ControllerConnector;
  const [username, setUsername] = useState<string>();

  useEffect(() => {
    if (!address) return;
    controller.username()?.then((n) => setUsername(n));
  }, [address, controller]);

  return (
    <div className="flex flex-row items-center gap-2 p-4">
      {address ? (
        <Button onClick={() => disconnect()}>Disconnect</Button>
      ) : (
        <Button onClick={() => connect({ connector: controller })}>
          Connect
        </Button>
      )}
      {address && (
        <div className="flex flex-row items-center gap-2">
          <Badge>{address}</Badge>
          {username && (
            <Badge
              variant="secondary"
              className="bg-blue-500 text-white dark:bg-blue-600"
            >
              {username}
            </Badge>
          )}
        </div>
      )}
    </div>
  );
}
