#!/usr/bin/env bash
# Merge a pull request by fast-forwarding main to its head commit, so the commits land exactly as they were reviewed
# and tested, with their own author and committer. GitHub's merge button would record the person merging instead.
#
#   scripts/merge-pr.sh <number>
#
# Refuses unless the pull request targets main, every check on its head commit passed, and its branch already contains
# main (rebase it first otherwise). Pushing to main directly is reserved for repository admins by the ruleset
# "main: pull requests and checks"; the ruleset "main: history" still blocks force-pushes and deletion for everyone.
set -euo pipefail
cd "$(dirname "$0")/.."

pr="${1:?usage: scripts/merge-pr.sh <pull request number>}"
remote="${REMOTE:-origin}"

read -r base head state < <(gh pr view "$pr" --json baseRefName,headRefOid,state --jq '"\(.baseRefName) \(.headRefOid) \(.state)"')
[ "$state" = OPEN ] || { echo "pull request #$pr is $state" >&2; exit 1; }
[ "$base" = main ] || { echo "pull request #$pr targets $base, not main" >&2; exit 1; }

bad=$(gh pr view "$pr" --json statusCheckRollup --jq '[.statusCheckRollup[] | select((.conclusion // .state) as $c | ["SUCCESS","SKIPPED","NEUTRAL"] | index($c) | not) | .name // .context] | join(", ")')
total=$(gh pr view "$pr" --json statusCheckRollup --jq '.statusCheckRollup | length')
[ "$total" -gt 0 ] || { echo "pull request #$pr has no checks yet" >&2; exit 1; }
[ -z "$bad" ] || { echo "checks not passed yet: $bad" >&2; exit 1; }

git fetch -q "$remote" main "$head"
git merge-base --is-ancestor "$remote/main" "$head" || {
  echo "the branch doesn't contain the latest main; rebase it onto main and wait for the checks again" >&2
  exit 1
}

git push "$remote" "$head:refs/heads/main"
echo "main is now $(git rev-parse --short "$head"); GitHub marks #$pr as merged"
