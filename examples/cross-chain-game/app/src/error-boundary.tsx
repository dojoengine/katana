import { Component, type ErrorInfo, type ReactNode } from "react";
import { PlugZap } from "lucide-react";

// React error boundaries must be class components. This catches render-time
// throws anywhere in the tree and shows a readable recovery screen instead of a
// blank page — most often when the local stack isn't running.
type Props = { children: ReactNode };
type State = { error: Error | null };

export class RootErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error("App crashed:", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex min-h-screen items-center justify-center bg-background p-6 text-foreground">
        <div className="w-full max-w-md rounded-2xl border bg-card p-7 text-center shadow-sm">
          <div className="mx-auto mb-4 grid size-12 place-items-center rounded-xl bg-amber-500/15 text-amber-600">
            <PlugZap className="size-6" />
          </div>
          <h1 className="text-lg font-bold tracking-tight">The demo couldn't start</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            This usually means the local stack isn't running. Start it, then reload this page:
          </p>
          <pre className="mt-4 rounded-lg bg-muted px-3 py-2 text-left font-mono text-xs">
            cd examples/cross-chain-game{"\n"}./up.sh
          </pre>
          <p className="mt-3 text-xs text-muted-foreground">
            It brings up the settlement node (<span className="font-mono">:5050</span>), appchain (
            <span className="font-mono">:5051</span>) and Torii (<span className="font-mono">:8081/:8082</span>).
          </p>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="mt-5 inline-flex h-10 items-center justify-center rounded-full bg-primary px-5 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90"
          >
            Reload
          </button>
          <details className="mt-4 text-left">
            <summary className="cursor-pointer text-xs text-muted-foreground">Error detail</summary>
            <pre className="mt-2 max-h-40 overflow-auto rounded-md bg-muted/60 p-2 text-[11px] break-words whitespace-pre-wrap">
              {this.state.error.message}
            </pre>
          </details>
        </div>
      </div>
    );
  }
}
