#!/usr/bin/env node
//
// bench-percent-diff.js
// =====================
//
// Compares a current run of `cargo bench` (bencher output format) against the
// latest baseline stored in the repo's gh-pages `data.js` (as produced by
// benchmark-action/github-action-benchmark) and emits a markdown table that
// highlights each benchmark's percentage delta vs. the baseline.
//
// Intended use
// ------------
// Invoked from the `benchmark` GitHub Actions workflow on pull requests. The
// workflow:
//   1. Runs `cargo bench ... -- --output-format bencher` and filters to
//      `output.txt` (one `test <name> ... bench: <value> ns/iter (+/- <n>)`
//      line per benchmark).
//   2. Fetches `data.js` from the `gh-pages` branch into `baseline.js`.
//   3. Runs this script to produce `diff.md`.
//   4. Uses `actions/github-script` to post/update a sticky PR comment
//      containing `diff.md`.
//
// It can also be run locally to preview the markdown for a given pair of
// files.
//
// Usage
// -----
//   node scripts/bench-percent-diff.js <baseline.js> <output.txt> [out.md]
//
//   <baseline.js>   Path to a `data.js` file (the one published to gh-pages
//                   by github-action-benchmark). Must assign to
//                   `window.BENCHMARK_DATA`. If the file has no entries, all
//                   benchmarks are reported with a `n/a` baseline.
//   <output.txt>    Path to the current bench run output in bencher format.
//                   Only lines matching
//                     `test <name> ... bench: <value> ns/iter (+/- <n>)`
//                   are considered; other lines are ignored.
//   [out.md]        Optional output path. If omitted, the markdown is
//                   written to stdout.
//
// Output
// ------
// A markdown document starting with an HTML marker comment
// (`<!-- benchmark-percent-diff -->`) used by the workflow to find and
// update the sticky PR comment, followed by a table:
//
//   | Benchmark | Baseline (ns) | Current (ns) | Δ |
//
// `Δ` is `((current - baseline) / baseline) * 100` formatted with two
// decimals and a leading sign (e.g. `+3.14%`, `-0.42%`). Benchmarks with no
// matching baseline entry show `n/a` for both the baseline value and Δ.
// If no benchmark lines are parsed from <output.txt>, the body reports
// "_No benchmark results parsed._" instead of a table.
//
// Exit codes
// ----------
//   0  on success (including the "no results parsed" case)
//   1  if required CLI args are missing

const fs = require('fs');

const [, , baselinePath, outputPath, outPath] = process.argv;
if (!baselinePath || !outputPath) {
  console.error('Usage: bench-percent-diff.js <baseline.js> <output.txt> [out.md]');
  process.exit(1);
}

const window = {};
eval(fs.readFileSync(baselinePath, 'utf8'));
const entries = (window.BENCHMARK_DATA || {}).entries || {};
const suite = Object.keys(entries)[0];
const latest = suite ? entries[suite][entries[suite].length - 1] : null;
const baseline = {};
if (latest) for (const b of latest.benches) baseline[b.name] = b.value;

const rows = [];
for (const line of fs.readFileSync(outputPath, 'utf8').split('\n')) {
  const m = line.match(/^test\s+(\S+)\s+\.\.\.\s+bench:\s+([\d,]+)\s+ns\/iter/);
  if (!m) continue;
  const name = m[1];
  const current = parseFloat(m[2].replace(/,/g, ''));
  const prev = baseline[name];
  let delta = 'n/a';
  if (prev != null) {
    const pct = ((current - prev) / prev) * 100;
    const sign = pct >= 0 ? '+' : '';
    delta = `${sign}${pct.toFixed(2)}%`;
  }
  rows.push({ name, current, prev: prev ?? 'n/a', delta });
}

const marker = '<!-- benchmark-percent-diff -->';
let body = `${marker}\n### Codec benchmark diff vs \`main\`\n\n`;
if (rows.length === 0) {
  body += '_No benchmark results parsed._\n';
} else {
  body += '| Benchmark | Baseline (ns) | Current (ns) | Δ |\n|---|---:|---:|---:|\n';
  for (const r of rows) {
    body += `| \`${r.name}\` | ${r.prev} | ${r.current} | ${r.delta} |\n`;
  }
}

if (outPath) fs.writeFileSync(outPath, body);
else process.stdout.write(body);
