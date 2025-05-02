#!/usr/bin/env bash
set -euo pipefail

# This script formats binary size comparison results into a markdown comment for GitHub PRs
# Usage: ./binary-size-formatter.sh <base_size_bytes> <pr_size_bytes>
# Example: ./binary-size-formatter.sh 10485760 11010048

if [ $# -ne 2 ]; then
  echo "Error: Requires 2 arguments"
  echo "Usage: $0 <base_size_bytes> <pr_size_bytes>"
  exit 1
fi

BASE_SIZE_BYTES="$1"
PR_SIZE_BYTES="$2"

# Calculate the difference and percentage
DIFF_BYTES=$((PR_SIZE_BYTES - BASE_SIZE_BYTES))
DIFF_PERCENT=$(awk "BEGIN {printf \"%.2f\", ($DIFF_BYTES/$BASE_SIZE_BYTES)*100}")

# Convert to human-readable sizes
BASE_SIZE_HUMAN=$(numfmt --to=iec-i --suffix=B --format="%.2f" "$BASE_SIZE_BYTES")
PR_SIZE_HUMAN=$(numfmt --to=iec-i --suffix=B --format="%.2f" "$PR_SIZE_BYTES")

# Format the change description
if [ "$DIFF_BYTES" -gt 0 ]; then
  CHANGE_INDICATOR="📈 Increased"
  # Format with thousands separator
  FORMATTED_DIFF=$(printf "%'d" "$DIFF_BYTES")
  CHANGE_DESCRIPTION="\`$DIFF_PERCENT%\`"
elif [ "$DIFF_BYTES" -lt 0 ]; then
  CHANGE_INDICATOR="📉 Decreased"
  # Remove minus sign for display but keep the minus in the formatted number
  FORMATTED_DIFF=$(printf "%'d" "$DIFF_BYTES")
  CHANGE_DESCRIPTION="\`$FORMATTED_DIFF bytes ($DIFF_PERCENT%)\`"
else
  CHANGE_INDICATOR="🟰 Unchanged"
  CHANGE_DESCRIPTION="\`No change\`"
fi

# Add warning if size increase is significant
WARNING=""
if (( $(echo "$DIFF_PERCENT > 5" | bc -l) )); then
  WARNING="⚠️ **Warning:** Binary size increased by more than the specified threshold ( 5% )"
fi

# Output the markdown
cat << EOF
## Binary Size Check 📊
**Base branch:** \`$BASE_SIZE_HUMAN\` ( $BASE_SIZE_BYTES bytes )
**PR branch:** \`$PR_SIZE_HUMAN\` ( $PR_SIZE_BYTES bytes )
**Change:** $CHANGE_INDICATOR by $CHANGE_DESCRIPTION

$WARNING
EOF
