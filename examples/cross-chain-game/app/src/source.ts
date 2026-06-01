// GitHub permalinks to the demo's on-chain source, so devs can cross-reference
// the real contract code while clicking through the app.
//
// Pinned to a commit SHA (a true permalink: exact lines, survives the PR merge /
// branch deletion). The two Cairo contract files this maps into are not edited
// after this point, so the line numbers stay accurate. If the contracts change,
// bump SOURCE_REF and re-check the line numbers below.

const REPO = "dojoengine/katana";
const DEMO = "examples/cross-chain-game";
export const SOURCE_REF = "279073a3d4fd6e99ada6ec40bd5c3e1f9bd28bbc";

// Symbol (as shown in the UI) -> its declaration in the demo contracts.
const SYMBOLS: Record<string, { file: string; line: number }> = {
  // appchain `game` world
  play_game: { file: "cairo/game/src/lib.cairo", line: 114 },
  mint_game: { file: "cairo/game/src/lib.cairo", line: 94 },
  send_message_to_l1: { file: "cairo/game/src/lib.cairo", line: 131 },
  Stats: { file: "cairo/game/src/lib.cairo", line: 37 },
  GamePlayed: { file: "cairo/game/src/lib.cairo", line: 72 },
  // settlement `score` world
  claim_score: { file: "cairo/score/src/lib.cairo", line: 89 },
  consume_message_from_appchain: { file: "cairo/score/src/lib.cairo", line: 99 },
  Leaderboard: { file: "cairo/score/src/lib.cairo", line: 42 },
  ScoreClaimed: { file: "cairo/score/src/lib.cairo", line: 73 },
};

/** GitHub permalink for a referenced symbol, or null if it isn't one of ours
 *  (e.g. piltover's `send_message_to_appchain`, the `--messaging.enabled` flag). */
export function sourceUrl(symbol: string): string | null {
  const key = symbol.trim().replace(/\(\)$/, ""); // "play_game()" -> "play_game"
  const hit = SYMBOLS[key];
  if (!hit) return null;
  return `https://github.com/${REPO}/blob/${SOURCE_REF}/${DEMO}/${hit.file}#L${hit.line}`;
}
