#!/usr/bin/env bash
set -euo pipefail

# This script formats binary size comparison results into a markdown report for weekly GitHub issues
# Usage: ./generate-weekly-binary-size-report.sh <release_tag> <release_size_bytes> <main_branch> <main_size_bytes> <main_commit>
# Example: ./generate-weekly-binary-size-report.sh v0.7.21 10485760 11010048 abc123def

if [ $# -ne 4 ]; then
  echo "Error: Requires 4 arguments"
  echo "Usage: $0 <release_tag> <release_size_bytes> <main_size_bytes> <main_commit>"
  exit 1
fi

RELEASE_TAG="$1"
RELEASE_SIZE_BYTES="$2"
MAIN_SIZE_BYTES="$3"
MAIN_COMMIT="$4"

# Calculate the difference and percentage
DIFF_BYTES=$((MAIN_SIZE_BYTES - RELEASE_SIZE_BYTES))
DIFF_PERCENT=$(awk "BEGIN {printf \"%.2f\", ($DIFF_BYTES/$RELEASE_SIZE_BYTES)*100}")

# Convert to human-readable sizes
RELEASE_SIZE_HUMAN=$(numfmt --to=iec-i --suffix=B --format="%.2f" "$RELEASE_SIZE_BYTES")
MAIN_SIZE_HUMAN=$(numfmt --to=iec-i --suffix=B --format="%.2f" "$MAIN_SIZE_BYTES")

# Determine trend and add appropriate emoji
if [ "$DIFF_BYTES" -gt 0 ]; then
  CHANGE_TEXT="📈 +$DIFF_PERCENT%"
  TREND_DESC="increased"
elif [ "$DIFF_BYTES" -lt 0 ]; then
  CHANGE_TEXT="📉 $DIFF_PERCENT%"
  TREND_DESC="decreased"
else
  CHANGE_TEXT="➡️ No change"
  TREND_DESC="remained the same"
fi

# Add warning if size increase is significant
WARNING=""
if (( $(echo "$DIFF_PERCENT > 10" | bc -l) )); then
  WARNING="⚠️ **Note:** Binary size has increased by more than 10% since the last release."
elif (( $(echo "$DIFF_PERCENT > 5" | bc -l) )); then
  WARNING="ℹ️ **Note:** Binary size has increased by more than 5% since the last release."
fi

# Generate the report date
REPORT_DATE=$(date -u +"%Y-%m-%d")

# Convert absolute diff to human-readable (handle negative values)
if [ "$DIFF_BYTES" -lt 0 ]; then
  DIFF_BYTES_ABS=$(( -DIFF_BYTES ))
  DIFF_HUMAN="-$(numfmt --to=iec-i --suffix=B --format="%.2f" "$DIFF_BYTES_ABS")"
elif [ "$DIFF_BYTES" -gt 0 ]; then
  DIFF_HUMAN="+$(numfmt --to=iec-i --suffix=B --format="%.2f" "$DIFF_BYTES")"
else
  DIFF_HUMAN="0B"
fi

# Output the markdown report
cat << EOF
# 📊 Weekly Binary Size Report - $REPORT_DATE

## Summary
The Katana binary size has **$TREND_DESC** by **$DIFF_PERCENT%** since the last release ($RELEASE_TAG).

## Binary Size Comparison

| Version | Size | Change |
|---------|------|--------|
| Release [($RELEASE_TAG)](https://github.com/dojoengine/katana/releases/tag/$RELEASE_TAG) | $RELEASE_SIZE_HUMAN | - |
| Main [(main)](https://github.com/dojoengine/katana/commit/$MAIN_COMMIT) | $MAIN_SIZE_HUMAN | $CHANGE_TEXT |

## Details
- **Absolute Change:** $DIFF_HUMAN
- **Relative Change:** $DIFF_PERCENT%

$WARNING

---
*This is an automated report.*
EOF
