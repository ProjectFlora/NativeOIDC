#!/usr/bin/env bash
# Precheck for Orca automations that follow docs/agent-workflows/issue-to-pr.md.
#
# Exits 0 when the scheduled run may have work for the given phases, and
# non-zero to let Orca skip the run. The check is deliberately broad: it never
# reads the Priority field, and the agent still decides with the workflow
# rules. A false "work found" only costs a short run; a false "no work" would
# leave work waiting.
#
# Usage: scripts/agent-precheck.sh [--phases plan,implement,pr]

set -uo pipefail

phases="plan,implement,pr"
while [ $# -gt 0 ]; do
  case "$1" in
    --phases) phases="${2:-}"; shift 2 ;;
    --phases=*) phases="${1#*=}"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "precheck: unknown argument: $1" >&2; exit 2 ;;
  esac
done

has_phase() {
  case ",$phases," in
    *",$1,"*) return 0 ;;
    *) return 1 ;;
  esac
}

work_found() {
  echo "precheck: work found: $1"
  exit 0
}

if ! gh auth status >/dev/null 2>&1; then
  echo "precheck: gh is not authenticated; skipping the run" >&2
  exit 1
fi

repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)" || {
  echo "precheck: could not resolve the repository; skipping the run" >&2
  exit 1
}

count_issues() {
  gh issue list -R "$repo" --state open --label "$1" --limit 1 --json number --jq 'length'
}

# Issues waiting to be started or resumed.
if has_phase plan && [ "$(count_issues agent-plan)" -gt 0 ]; then
  work_found "agent-plan issue"
fi
if has_phase implement && [ "$(count_issues agent-implement)" -gt 0 ]; then
  work_found "agent-implement issue"
fi
if has_phase plan && has_phase implement && [ "$(count_issues agent-ready)" -gt 0 ]; then
  work_found "agent-ready issue"
fi

# Open agent PRs and the issues they close.
prs="$(gh pr list -R "$repo" --state open --label agent-pr --limit 100 \
  --json number,mergeable --jq '.[] | "\(.number) \(.mergeable)"')"
pr_issues="$(gh pr list -R "$repo" --state open --label agent-pr --limit 100 \
  --json closingIssuesReferences --jq '.[].closingIssuesReferences[].number')"

# Newest creation time of agent posts (marker = true) or of other posts
# (marker = false) on a PR: issue comments, review comments, and reviews.
newest_post() {
  local number="$1" marker="$2" filter
  filter="[.[] | select(((.body // \"\") | startswith(\"<!-- agent-workflow\")) == $marker)"
  {
    gh api "repos/$repo/issues/$number/comments?per_page=100" \
      --jq "$filter | .created_at] | max // empty"
    gh api "repos/$repo/pulls/$number/comments?per_page=100" \
      --jq "$filter | .created_at] | max // empty"
    gh api "repos/$repo/pulls/$number/reviews?per_page=100" \
      --jq "$filter | .submitted_at] | max // empty"
  } | sort | tail -n 1
}

if has_phase pr && [ -n "$prs" ]; then
  while read -r number mergeable; do
    [ -z "$number" ] && continue
    if [ "$mergeable" = "CONFLICTING" ]; then
      work_found "PR #$number conflicts with the base branch"
    fi
    gh pr checks "$number" -R "$repo" >/dev/null 2>&1
    if [ $? -eq 1 ]; then
      work_found "PR #$number has failing checks"
    fi
    last_agent="$(newest_post "$number" true)"
    last_other="$(newest_post "$number" false)"
    if [ -n "$last_other" ] && [[ "$last_other" > "$last_agent" ]]; then
      work_found "PR #$number has activity after the last agent post"
    fi
  done <<< "$prs"
fi

# Answers on the issues of PRs in progress (for example, after a PR-phase
# pause): a comment on the issue newer than the last agent comment there.
if has_phase pr && [ -n "$pr_issues" ]; then
  for number in $pr_issues; do
    last_agent="$(gh api "repos/$repo/issues/$number/comments?per_page=100" \
      --jq '[.[] | select((.body // "") | startswith("<!-- agent-workflow")) | .created_at] | max // empty')"
    last_other="$(gh api "repos/$repo/issues/$number/comments?per_page=100" \
      --jq '[.[] | select(((.body // "") | startswith("<!-- agent-workflow")) | not) | .created_at] | max // empty')"
    if [ -n "$last_other" ] && [[ "$last_other" > "$last_agent" ]]; then
      work_found "issue #$number has a comment after the last agent comment"
    fi
  done
fi

# Stalled work: agent-in-progress issues without an open agent PR and without
# updates for 2 hours. The agent applies the exact claim rules.
if has_phase plan || has_phase implement; then
  stalled="$(gh issue list -R "$repo" --state open --label agent-in-progress --limit 100 \
    --json number,updatedAt \
    --jq '.[] | select((now - (.updatedAt | fromdateiso8601)) > 7200) | .number')"
  for number in $stalled; do
    if ! grep -qx "$number" <<< "$pr_issues"; then
      work_found "issue #$number looks stalled"
    fi
  done
fi

echo "precheck: no work for phases $phases; skipping the run"
exit 1
