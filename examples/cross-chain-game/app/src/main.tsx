import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App.tsx";
import { WalletProvider } from "./wallet.tsx";
import { RootErrorBoundary } from "./error-boundary.tsx";
import "./index.css";

// RootErrorBoundary is the last line of defense: if anything throws during
// render, the user gets a readable "start the stack" screen instead of a blank
// page. (Module-load throws can't be caught here — the data/wallet layers guard
// those — but this covers render-time failures.)
ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <RootErrorBoundary>
      <WalletProvider>
        <App />
      </WalletProvider>
    </RootErrorBoundary>
  </React.StrictMode>,
);
