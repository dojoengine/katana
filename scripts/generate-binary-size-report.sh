#!/usr/bin/env bash
set -euo pipefail

# This script formats binary size comparison results into a markdown comment for GitHub PRs
# Usage: ./generate-binary-size-report.sh <base_branch> <base_size_bytes> <pr_branch> <pr_size_bytes>
# Example: ./generate-binary-size-report.sh main 10485760 feature-branch 11010048

if [ $# -ne 4 ]; then
  echo "Error: Requires 4 arguments"
  echo "Usage: $0 <base_branch> <base_size_bytes> <pr_branch> <pr_size_bytes>"
  exit 1
fi

BASE_BRANCH="$1"
BASE_SIZE_BYTES="$2"
PR_BRANCH="$3"
PR_SIZE_BYTES="$4"

# Calculate the difference and percentage
DIFF_BYTES=$((PR_SIZE_BYTES - BASE_SIZE_BYTES))
DIFF_PERCENT=$(awk "BEGIN {printf \"%.2f\", ($DIFF_BYTES/$BASE_SIZE_BYTES)*100}")

# Convert to human-readable sizes
BASE_SIZE_HUMAN=$(numfmt --to=iec-i --suffix=B --format="%.2f" "$BASE_SIZE_BYTES")
PR_SIZE_HUMAN=$(numfmt --to=iec-i --suffix=B --format="%.2f" "$PR_SIZE_BYTES")

# Add warning if size increase is significant
WARNING=""
if (( $(echo "$DIFF_PERCENT > 5" | bc -l) )); then
  WARNING="⚠️ _**Warning:** Binary size increased by more than the specified threshold ( 5% )_"
fi

# Format the percentage change for display
if [ "$DIFF_BYTES" -gt 0 ]; then
  CHANGE_TEXT="( +$DIFF_PERCENT% )"
elif [ "$DIFF_BYTES" -lt 0 ]; then
  CHANGE_TEXT="( $DIFF_PERCENT% )"
else
  CHANGE_TEXT=""
fi

# Output the markdown
if [ -n "$WARNING" ]; then
  cat << EOF
## Binary size report 📊

| Branch | Size |
|--------|------|
| \`$BASE_BRANCH\` | $BASE_SIZE_HUMAN   |
| \`$PR_BRANCH\` | $PR_SIZE_HUMAN $CHANGE_TEXT |

$WARNING
EOF
else
  cat << EOF
## Binary size report 📊

| Branch | Size |
|--------|------|
| \`$BASE_BRANCH\` | $BASE_SIZE_HUMAN   |
| \`$PR_BRANCH\` | $PR_SIZE_HUMAN $CHANGE_TEXT |
EOF
fi
