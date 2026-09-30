#!/usr/bin/env bash
# Fail if the given pull request still has unresolved review threads.
#
# Complements GitHub branch-protection `required_conversation_resolution`
# with an explicit merge-CI job so the gate is visible next to other checks.
#
# Usage:
#   scripts/check-pr-review-threads-resolved.sh <pr-number>
#   GH_REPO=owner/name scripts/check-pr-review-threads-resolved.sh <pr-number>
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <pr-number>" >&2
  exit 2
fi

PR_NUMBER="$1"
if ! [[ "$PR_NUMBER" =~ ^[0-9]+$ ]]; then
  echo "error: pr-number must be a positive integer, got: $PR_NUMBER" >&2
  exit 2
fi

REPO="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
if [[ -z "$REPO" ]]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
fi
if [[ -z "$REPO" || "$REPO" != */* ]]; then
  echo "error: set GH_REPO or GITHUB_REPOSITORY to owner/name" >&2
  exit 2
fi

OWNER="${REPO%%/*}"
NAME="${REPO#*/}"

QUERY='
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          isOutdated
          path
          comments(first: 1) {
            nodes { body author { login } }
          }
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
'

RESPONSE="$(gh api graphql \
  -f query="$QUERY" \
  -f owner="$OWNER" \
  -f name="$NAME" \
  -F number="$PR_NUMBER")"

# The report is jq's (#2283: no Python anywhere in the workspace).
PR_JSON="$(printf '%s' "$RESPONSE" | jq -c '.data.repository.pullRequest // empty')"
if [[ -z "$PR_JSON" ]]; then
  echo "ERROR: pull request not found or GraphQL returned no data" >&2
  printf '%s' "$RESPONSE" | jq '.errors // empty' >&2
  exit 2
fi
if [[ "$(jq -r '.reviewThreads.pageInfo.hasNextPage // false' <<<"$PR_JSON")" == "true" ]]; then
  echo "ERROR: PR has more than 100 review threads; extend pagination" >&2
  exit 2
fi
TOTAL="$(jq '.reviewThreads.nodes // [] | length' <<<"$PR_JSON")"
OPEN="$(jq '[.reviewThreads.nodes // [] | .[] | select(.isResolved | not)] | length' <<<"$PR_JSON")"
if [[ "$OPEN" == "0" ]]; then
  echo "OK: all ${TOTAL} review thread(s) resolved"
  exit 0
fi
echo "FAIL: ${OPEN} unresolved review thread(s) (of ${TOTAL} total):"
jq -r '
  .reviewThreads.nodes // [] | .[] | select(.isResolved | not)
  | ((.comments.nodes // [])[0]) as $first
  | "  - \(.path // "(no path)")\(if .isOutdated then " [outdated]" else "" end)"
    + " (@\($first.author.login // "?")): "
    + (($first.body // "") | gsub("\r"; "") | gsub("^\\s+|\\s+$"; "") | split("\n")[0] // "" | .[0:120])
' <<<"$PR_JSON"
exit 1
