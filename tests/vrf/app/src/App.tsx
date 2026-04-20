import { StarknetProvider } from "./StarknetProvider";
import { ConnectWallet } from "./ConnectWallet";
import { Dice } from "./Dice";

import "./globals.css";

export default function App() {
  return (
    <StarknetProvider>
      <ConnectWallet />
      <Dice />
    </StarknetProvider>
  );
}
