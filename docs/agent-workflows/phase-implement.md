# Phase: Implement (Steps 3–5)

Implement the latest plan comment on the issue, review your own work, and open a PR. Read [common-rules.md](common-rules.md) first.

## Before starting
- Read the issue, all of its comments, and the latest comment starting with `<!-- agent-workflow:plan -->`. This plan is your instruction for this phase
- Read every comment from me posted after that plan (see "Post Markers" in [common-rules.md](common-rules.md)). They take priority over the plan
- If there is no plan comment, stop: this phase cannot start without one
  - Manual mode: report in this session
  - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md), with `phase=plan` and the plan-phase `resume-label` for this flow

## 3. Implement
- Name the branch `issue-{number}-{short-description}`
- Follow the plan's steps. Keep changes within the plan and the issue (report other problems you notice; do not fix them)
- Add or update tests as the plan says, and as needed
- If the plan cannot be followed (it is wrong, incomplete, or conflicts with the code), do not change the approach on your own. Stop and describe what does not work
  - Manual mode: report in this session
  - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md). Use `phase=plan` if the plan needs rewriting, or `phase=implement` if a decision from me is enough, and the `resume-label` that table gives for that phase and this flow

## 4. Self-review
Check the following. If anything fails, go back to step 3.
- All acceptance criteria in the plan and the issue are met
- Every step in the plan is done, or the PR description explains why not
- No issues that could cause bugs (edge cases, error handling, impact on existing features)
- Tests, lint, and type checks pass locally
- The diff contains no unnecessary changes (debug code, unrelated formatting, etc.)

## 5. Create the pull request
- Add the `agent-pr` label to the PR
- Include the following in the PR description (starting with `<!-- agent-workflow -->`):
  - `Closes #{number}`
  - A link to the plan comment
  - Changes / How to verify / Scope of impact
  - Any deviation from the plan, and why

## When the phase ends
- Manual mode: report the PR URL in this session. If I asked for all phases, continue to [phase-pr.md](phase-pr.md) and wait for reviews according to "Review Waiting Rules" there
- Scheduled mode: keep the issue labeled `agent-in-progress` and end the run. The PR phase picks the PR up in a later run
