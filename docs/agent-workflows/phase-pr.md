# Phase: PR (Steps 6–7)

Handle CI results and review feedback on an agent PR until it is ready to merge, then report. Read [common-rules.md](common-rules.md) first.

## Before starting
- Read the PR, its linked issue, and the latest plan comment on the issue (starting with `<!-- agent-workflow:plan -->`), if there is one
- Retrieve the PR state according to "Verification Rules" in [common-rules.md](common-rules.md)
- When pausing for input in this phase (scheduled mode), use `phase=pr`, the PR number, and `resume-label=agent-in-progress`

## 6. Handle CI and reviews
- Retrieve CI results and feedback according to "Verification Rules" in [common-rules.md](common-rules.md)
- If CI fails: identify the cause and fix it
- For each unhandled comment from me (see "Post Markers" in [common-rules.md](common-rules.md)), regardless of when it was posted:
  - Treat it as an instruction and follow it. If the intent is unclear, ask me (in scheduled mode, follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md))
  - Reply with what you changed, including the `Handled:` line. Resolve inline threads after replying, following "Before resolving a thread" in "Post Markers" in [common-rules.md](common-rules.md). For a review body or top-level PR comment, post the reply as a PR comment
- For each unhandled comment from Copilot or Orca Bot, regardless of when it was posted:
  - Do not accept it blindly; judge whether it is valid (reviewers are AI and may be wrong or excessive)
  - Valid: fix it and reply with what you changed. Resolve inline threads after replying, following "Before resolving a thread" in "Post Markers" in [common-rules.md](common-rules.md)
  - Not valid: do not change the code; reply with your reasoning. Leave inline threads unresolved (see the table in "Post Markers" in [common-rules.md](common-rules.md))
  - In every reply, include the `Handled:` line. For a review body or top-level PR comment, post the reply as a PR comment
  - Unsure: do not change the code; consult me
  - Copilot and Orca Bot contradict each other: do not change the code; consult me
  - If Orca Bot rejects your reasoning in the same thread twice, stop arguing and consult me
- After pushing fixes or replying, wait for re-review according to "Review Waiting Rules" below
- If fixing the same cause fails 3 times, stop and report the situation
  - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md)

## 7. Final report (do not merge)
When all of the following are true, report without merging and end the work. Each condition must be confirmed with results you actually retrieved (see "Verification Rules" in [common-rules.md](common-rules.md)). If any condition could not be confirmed, it is not met.
- All CI checks pass on the current PR head
- At least one review from Copilot or Orca Bot was submitted on the current head commit (the review's commit ID equals the PR's head SHA)
- Every feedback comment is handled (its ID is listed on a `Handled:` line), including comments in resolved threads
- No unresolved inline threads remain, except Copilot threads whose latest comment is your "not fixed" reasoning
  - An unresolved Orca Bot thread always blocks the final report, even if you replied. Wait for Orca Bot to resolve it or reply
- No conflicts with the base branch (resolve them if there are any)

Report contents:
- Summary of the changes
- PR URL
- Feedback you did not apply, and why
  - List unresolved Copilot threads separately with their links, as "Needs your decision before merging"
- Points you were unsure about, and remaining issues
- Evidence, copied from the actual command output and retrieved right before reporting:
  - PR head SHA
  - Every check run name and its conclusion for that SHA
  - The ID and commit SHA of the review on the current head
  - Number of unresolved threads and of unhandled feedback comments (must be 0, except the listed Copilot threads)

A report without this evidence, or with evidence for an older SHA, is not a valid final report.

Where to report:
- Manual mode: in this session
- Scheduled mode: as a PR comment (with an agent marker), and change the issue label from `agent-in-progress` to `agent-done`

---

## Review Waiting Rules

There are two reviewers, and they arrive at different times.
- **Copilot**: reviews automatically when the PR is created (usually within a few minutes)
- **Orca Bot**: starts reviewing at minute 0 of every hour
  - It posts from the same account as the agent and me (`saitomarch`). Identify its posts by the `<!-- orca-bot -->` marker (see "Post Markers" in [common-rules.md](common-rules.md))

How the two reviewers interact:
- If Copilot has already reviewed the latest SHA, Orca Bot usually skips its full review of that SHA
- Orca Bot checks new replies on its own unresolved threads every hour, even when the code has not changed
- Proceed as soon as either reviewer has reviewed the latest SHA. If both leave feedback, handle both

### Waiting in manual mode
1. After creating the PR or pushing, wait up to 15 minutes for Copilot's review of the latest SHA
2. If Copilot reviews, handle its feedback (step 6)
3. If no review arrives within 15 minutes, or Copilot leaves a comment saying it could not review, assume Copilot hit its usage limit and wait for Orca Bot
   - Wait until 30 minutes past the next top of the hour for Orca Bot's review
   - If it still does not arrive, report to me and end
4. After replying with "not fixed" reasoning on an Orca Bot thread, wait until 30 minutes past the next top of the hour for Orca Bot to resolve the thread or reply
   - If neither happens, report to me and end

### Re-review (after pushing)
- Copilot: if no re-review of the latest SHA arrives automatically, re-request it
- Orca Bot: wait for the next top-of-the-hour run (no re-request needed)

### Waiting in scheduled mode
- Do not wait. Handle the CI results and feedback available at that moment, then end the run
- The next run picks up where you left off (see "Choosing a target" in [issue-to-pr.md](issue-to-pr.md))
