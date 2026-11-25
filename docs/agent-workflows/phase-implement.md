# Phase: Implement (Steps 3–5)

Implement the latest plan comment on the issue, review your own work, and open a PR. Read [common-rules.md](common-rules.md) first.

## Before starting
- Read the issue, all of its comments, and the latest plan (see "Reading plan comments" in [common-rules.md](common-rules.md)). This plan is your instruction for this phase
- Read every comment from me posted after that plan (see "Post Markers" in [common-rules.md](common-rules.md)). They take priority over the plan
- If the issue branch `issue-{number}-*` already exists (a resumed or stalled run), continue on it: check which plan steps its commits already cover, and start from the first step that is not done
- Record that implementation started or resumed, the branch name, and the plan step you start from:
  - Manual mode: post an issue comment (with an agent marker)
  - Scheduled mode, when this run already owns the claim (it was transferred from the plan phase in the same run): edit that claim comment to add the branch and the step. Do not post a new claim
  - Scheduled mode, otherwise: post the claim (see "Claiming an issue" in [issue-to-pr.md](issue-to-pr.md)), with `phase=implement`, and include the branch and the step in it
- If there is no plan comment, stop: this phase cannot start without one
  - Exception: an issue resumed from a pause posted before the phase split runs the plan phase first (see "Pauses posted before the phase split" in [issue-to-pr.md](issue-to-pr.md))
  - Manual mode: report in this session
  - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md), with `phase=plan` and the plan-phase `resume-label` for this flow

## 3. Implement
- Name the branch `issue-{number}-{short-description}`
- Follow the plan's steps. Keep changes within the plan and the issue (report other problems you notice; do not fix them)
- Add or update tests as the plan says, and as needed
- Commit and push to the issue branch after each completed plan step. Pushed commits are the record that a later run resumes from
- In scheduled mode, keep your claim alive as "Claiming an issue" in [issue-to-pr.md](issue-to-pr.md) says
- If the plan cannot be followed (it is wrong, incomplete, or conflicts with the code), do not change the approach on your own. Stop and describe what does not work
  - Manual mode: report in this session
  - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md). Use `phase=plan` if the plan needs rewriting, or `phase=implement` if a decision from me is enough, and the `resume-label` that table gives for that phase and this flow

## 4. Self-review
Check the following. If anything fails, go back to step 3.
- All acceptance criteria in the plan and the issue are met
- Every step in the plan is done, or the PR description explains why not
- No issues that could cause bugs (edge cases, error handling, impact on existing features)
- Tests, lint, and type checks pass locally
  - If a check cannot run in this environment (missing toolchain, OS restriction, unavailable service), do not stop and do not wait for it. Record which check could not run and why, and leave it to CI (see step 5). A check that ran and failed is not this case: fix it
- The diff contains no unnecessary changes (debug code, unrelated formatting, etc.)

## 5. Create the pull request
- Add the `agent-pr` label to the PR
- Include the following in the PR description (starting with `<!-- agent-workflow -->`):
  - `Closes #{number}`
  - A link to the plan comment
  - Changes / How to verify / Scope of impact
  - Any deviation from the plan, and why
  - Local checks: which ran and passed, and which could not run and why

## How the phase ends
This phase ends in exactly one of two ways. A summary of changes alone is neither, so it is not an ending.

1. **PR opened**: the PR exists, carries the `agent-pr` label, and your report includes its URL
   - Manual mode: report the PR URL in this session. If I asked for all phases, continue to [phase-pr.md](phase-pr.md) and wait for reviews according to "Review Waiting Rules" there
   - Scheduled mode: keep the issue labeled `agent-in-progress` and end the run. The PR phase picks the PR up in a later run
2. **Paused**: something only I can resolve blocks the work
   - Manual mode: report in this session what blocks the work and what you need from me
   - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md), and include the URL of the pause comment in your final message

Before ending either way, commit and push every change to the issue branch. Nothing may stay only in the local working tree.

If a push fails, retry once. If it still fails (branch protection, missing permission, network), end **Paused**, and record in the pause report:
- The branch name and the SHAs of the commits that were not pushed
- The push error
- Where the commits remain (the machine and the worktree path, as far as you know)
- What I need to do (for example, restore the permission, then resume)

Commit every change before pausing this way; only unpushed commits may remain, never uncommitted changes. A run that resumes this work checks whether the recorded commits are on the branch. If they are not and the recorded worktree cannot be reached, it pauses again and says so.

If you are about to end without either outcome (for example, after summarizing your changes), go back: open the PR (step 5), or pause.
