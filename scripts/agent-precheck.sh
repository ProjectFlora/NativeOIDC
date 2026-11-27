#!/usr/bin/env bash
# Precheck for Orca automations that follow docs/agent-workflows/issue-to-pr.md.
#
# Exits 0 when the scheduled run may have work, and non-zero to let Orca skip
# the run. Pass the same phases and filters as the automation prompt:
#
#   scripts/agent-precheck.sh [--phases plan,implement,pr]
#     [--priorities "Urgent,High"] [--efforts "Very High"]
#     [--implement-efforts "High,Medium,none"]
#
# Filter values are the issue field option names (for example "Very High"),
# or "none" for an unset field; omitted filters accept every value. The check
# stays broad: when in doubt (for example, the issue fields cannot be read) it
# lets the run start, and the agent decides with the workflow rules.

set -uo pipefail

phases="plan,implement,pr"
priorities=""
efforts=""
implement_efforts=""
while [ $# -gt 0 ]; do
  case "$1" in
    --phases) phases="${2:-}"; shift 2 ;;
    --phases=*) phases="${1#*=}"; shift ;;
    --priorities) priorities="${2:-}"; shift 2 ;;
    --priorities=*) priorities="${1#*=}"; shift ;;
    --efforts) efforts="${2:-}"; shift 2 ;;
    --efforts=*) efforts="${1#*=}"; shift ;;
    --implement-efforts) implement_efforts="${2:-}"; shift 2 ;;
    --implement-efforts=*) implement_efforts="${1#*=}"; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "precheck: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -z "$implement_efforts" ] && implement_efforts="$efforts"

has_phase() {
  case ",$phases," in
    *",$1,"*) return 0 ;;
    *) return 1 ;;
  esac
}

# in_list "Very High" "High, Very High" -> success. An empty list accepts all.
in_list() {
  local value="$1" list="$2" item
  local -a items
  [ -z "$list" ] && return 0
  IFS=',' read -r -a items <<< "$list"
  for item in "${items[@]}"; do
    item="$(echo "$item" | sed 's/^ *//; s/ *$//')"
    [ "$item" = "$value" ] && return 0
  done
  return 1
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

filters_set=0
[ -n "$priorities$efforts$implement_efforts" ] && filters_set=1

# Prints "Priority|Effort" for an issue, "none" for unset fields.
issue_values() {
  gh api -H 'X-GitHub-Api-Version: 2026-03-10' \
    "repos/$repo/issues/$1/issue-field-values" \
    --jq '(map(select(.issue_field_name == "Priority"))[0].single_select_option.name // "none")
      + "|" + (map(select(.issue_field_name == "Effort"))[0].single_select_option.name // "none")'
}

# candidate <issue number> <effort filter: efforts|implement|either> <reason>
# Reports work when the issue matches the filters. If the fields cannot be
# read, it lets the run start so that the agent can report the problem.
candidate() {
  local number="$1" kind="$2" reason="$3" values priority effort
  if [ "$filters_set" -eq 0 ]; then
    work_found "$reason"
  fi
  if ! values="$(issue_values "$number" 2>/dev/null)"; then
    work_found "$reason (issue fields of #$number could not be read; the run will report it)"
  fi
  priority="${values%%|*}"
  effort="${values#*|}"
  in_list "$priority" "$priorities" || return 0
  case "$kind" in
    efforts) in_list "$effort" "$efforts" || return 0 ;;
    implement) in_list "$effort" "$implement_efforts" || return 0 ;;
    either) in_list "$effort" "$efforts" || in_list "$effort" "$implement_efforts" || return 0 ;;
  esac
  work_found "$reason (Priority $priority, Effort $effort)"
}

list_issues() {
  gh issue list -R "$repo" --state open --label "$1" --limit 100 --json number --jq '.[].number'
}

# Issues waiting to be started or resumed.
if has_phase plan; then
  for number in $(list_issues agent-plan); do
    candidate "$number" efforts "agent-plan issue #$number"
  done
fi
if has_phase implement; then
  for number in $(list_issues agent-implement); do
    candidate "$number" implement "agent-implement issue #$number"
  done
fi
if has_phase plan && has_phase implement; then
  for number in $(list_issues agent-ready); do
    candidate "$number" efforts "agent-ready issue #$number"
  done
fi

# Open agent PRs with the issue each one closes.
prs="$(gh pr list -R "$repo" --state open --label agent-pr --limit 100 \
  --json number,mergeable,closingIssuesReferences \
  --jq '.[] | "\(.number) \(.mergeable) \(.closingIssuesReferences[0].number // "")"')"
pr_issues="$(awk '{ if ($3 != "") print $3 }' <<< "$prs")"

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

# The PR filters use the linked issue; a PR without one cannot be filtered.
pr_candidate() {
  local pr="$1" issue="$2" reason="$3"
  if [ -z "$issue" ]; then
    work_found "$reason (no linked issue to filter on)"
  fi
  candidate "$issue" efforts "$reason"
}

if has_phase pr && [ -n "$prs" ]; then
  while read -r number mergeable issue; do
    [ -z "$number" ] && continue
    if [ "$mergeable" = "CONFLICTING" ]; then
      pr_candidate "$number" "$issue" "PR #$number conflicts with the base branch"
      continue
    fi
    gh pr checks "$number" -R "$repo" >/dev/null 2>&1
    if [ $? -eq 1 ]; then
      pr_candidate "$number" "$issue" "PR #$number has failing checks"
      continue
    fi
    last_agent="$(newest_post "$number" true)"
    last_other="$(newest_post "$number" false)"
    if [ -n "$last_other" ] && [[ "$last_other" > "$last_agent" ]]; then
      pr_candidate "$number" "$issue" "PR #$number has activity after the last agent post"
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
      candidate "$number" efforts "issue #$number has a comment after the last agent comment"
    fi
  done
fi

# Stalled work: agent-in-progress issues without an open agent PR and without
# updates for 2 hours. It may resume the plan or the implement phase, so
# either Effort filter may apply.
if has_phase plan || has_phase implement; then
  stalled="$(gh issue list -R "$repo" --state open --label agent-in-progress --limit 100 \
    --json number,updatedAt \
    --jq '.[] | select((now - (.updatedAt | fromdateiso8601)) > 7200) | .number')"
  for number in $stalled; do
    if ! grep -qx "$number" <<< "$pr_issues"; then
      candidate "$number" either "issue #$number looks stalled"
    fi
  done
fi

echo "precheck: no work for phases $phases and the given filters; skipping the run"
exit 1
