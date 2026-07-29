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

UNRESOLVED="$(printf '%s' "$RESPONSE" | python3 -c '
import json, sys
data = json.load(sys.stdin)
pr = (data.get("data") or {}).get("repository") or {}
pr = pr.get("pullRequest")
if pr is None:
    print("ERROR: pull request not found or GraphQL returned no data", file=sys.stderr)
    if data.get("errors"):
        print(json.dumps(data["errors"], indent=2), file=sys.stderr)
    sys.exit(2)
threads = ((pr.get("reviewThreads") or {}).get("nodes")) or []
page = (pr.get("reviewThreads") or {}).get("pageInfo") or {}
if page.get("hasNextPage"):
    print("ERROR: PR has more than 100 review threads; extend pagination", file=sys.stderr)
    sys.exit(2)
open_threads = [t for t in threads if not t.get("isResolved")]
if not open_threads:
    print(f"OK: all {len(threads)} review thread(s) resolved")
    sys.exit(0)
print(f"FAIL: {len(open_threads)} unresolved review thread(s) (of {len(threads)} total):")
for t in open_threads:
    path = t.get("path") or "(no path)"
    nodes = ((t.get("comments") or {}).get("nodes")) or []
    author = "?"
    snippet = ""
    if nodes:
        author = ((nodes[0].get("author") or {}).get("login")) or "?"
        body = (nodes[0].get("body") or "").strip().splitlines()
        snippet = body[0][:120] if body else ""
    outdated = " [outdated]" if t.get("isOutdated") else ""
    print(f"  - {path}{outdated} (@{author}): {snippet}")
sys.exit(1)
')"

printf '%s\n' "$UNRESOLVED"
