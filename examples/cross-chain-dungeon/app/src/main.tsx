import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App.tsx";
import { WalletProvider } from "./wallet.tsx";
import { RootErrorBoundary } from "./error-boundary.tsx";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <RootErrorBoundary>
      <WalletProvider>
        <App />
      </WalletProvider>
    </RootErrorBoundary>
  </React.StrictMode>,
);
