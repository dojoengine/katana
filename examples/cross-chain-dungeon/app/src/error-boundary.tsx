import { Component, type ErrorInfo, type ReactNode } from "react";

// Last line of defense: if anything throws during render, show a readable
// "start the stack" screen instead of a blank page.
export class RootErrorBoundary extends Component<{ children: ReactNode }, { error?: Error }> {
  state: { error?: Error } = {};

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error("render error:", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="boundary">
          <h1>// stack offline</h1>
          <p>The dungeon client failed to render. Is the stack up?</p>
          <pre>cp .env.example .env  # fill in Sepolia accounts + USDC{"\n"}./up.sh</pre>
          <pre className="err">{String(this.state.error.message || this.state.error)}</pre>
        </div>
      );
    }
    return this.props.children;
  }
}
