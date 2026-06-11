// GitHub permalinks to the demo's on-chain source, so devs can cross-reference
// the real contract code while clicking through the app.
//
// Pinned to a commit SHA (a true permalink: exact lines, survives the PR merge /
// branch deletion). The Cairo contract files this maps into are stable, so the
// line numbers stay accurate. If the contracts change, bump SOURCE_REF and
// re-check the line numbers below.

// The demo's own contracts, pinned to a katana commit (see note above).
export const SOURCE_REF = "2e36ba5ae08b2f7c07e6e6a458464995e1d59a25";
const DEMO_BASE = `https://github.com/dojoengine/katana/blob/${SOURCE_REF}/examples/cross-chain-game`;

// The piltover core is an external contract (cartridge-gg/piltover), pulled in
// by katana as a submodule. Pinned to the commit katana builds against.
const PILTOVER_REF = "ebb714b3a0e63da8088ea4f371bcca2a1a3f74f0";
const PILTOVER_BASE = `https://github.com/cartridge-gg/piltover/blob/${PILTOVER_REF}`;

// Symbol (as shown in the UI) -> its source. `repo: "piltover"` points at the
// external piltover contract; everything else is the demo's own code.
const SYMBOLS: Record<string, { file: string; line: number; repo?: "piltover" }> = {
  // appchain `game` world
  play_game: { file: "cairo/game/src/lib.cairo", line: 114 },
  mint_game: { file: "cairo/game/src/lib.cairo", line: 94 },
  send_message_to_l1: { file: "cairo/game/src/lib.cairo", line: 131 },
  Stats: { file: "cairo/game/src/lib.cairo", line: 37 },
  GamePlayed: { file: "cairo/game/src/lib.cairo", line: 72 },
  // settlement `store` world — the L1 storefront
  buy_game: { file: "cairo/store/src/lib.cairo", line: 69 },
  // settlement `score` world
  claim_score: { file: "cairo/score/src/lib.cairo", line: 89 },
  consume_message_from_appchain: { file: "cairo/score/src/lib.cairo", line: 99 },
  Leaderboard: { file: "cairo/score/src/lib.cairo", line: 42 },
  ScoreClaimed: { file: "cairo/score/src/lib.cairo", line: 73 },
  // piltover core (external) — the L1 messaging mailbox + settled state
  send_message_to_appchain: { file: "src/messaging/component.cairo", line: 165, repo: "piltover" },
  MessageSent: { file: "src/messaging/component.cairo", line: 88, repo: "piltover" },
  update_state: { file: "src/appchain.cairo", line: 140, repo: "piltover" },
};

/** GitHub permalink for a referenced symbol, or null if it isn't one we map
 *  (e.g. the `--messaging.enabled` flag, or `poseidon` pseudocode). */
export function sourceUrl(symbol: string): string | null {
  const key = symbol.trim().replace(/\(\)$/, ""); // "play_game()" -> "play_game"
  const hit = SYMBOLS[key];
  if (!hit) return null;
  const base = hit.repo === "piltover" ? PILTOVER_BASE : DEMO_BASE;
  return `${base}/${hit.file}#L${hit.line}`;
}
