# Issue Workflow (Issue → Pull Request → Ready to Merge)

This is the entry point for handling development work based on a GitHub Issue. It defines the execution modes, the phases, and how scheduled runs choose their work. Read [common-rules.md](common-rules.md) before any phase.

## Execution modes

- **Manual mode**: I specify the issue or PR number. Ask questions in this session and wait for my answer.
- **Scheduled mode**: Started by an Orca automation. Pick the target yourself; post questions as Issue comments and end the run.

If the request does not explicitly say "scheduled mode", treat it as manual mode.

If the request names no issue or PR number, do not ask for one: choose the target yourself, following "Choosing a target" below. In manual mode, still ask questions and report in this session.

## Phases

The work is split into three phases, so that each can run in its own session, with its own model.

| Phase | Steps | File | Output for the next phase |
|---|---|---|---|
| `plan` | 1–2: understand the issue, decide the approach | [phase-plan.md](phase-plan.md) | A plan comment on the issue |
| `implement` | 3–5: implement, self-review, open the PR | [phase-implement.md](phase-implement.md) | A PR labeled `agent-pr` |
| `pr` | 6–7: handle CI and reviews, final report | [phase-pr.md](phase-pr.md) | A final report |

Read only the files for the phases you run, plus [common-rules.md](common-rules.md).

### Which phases to run

The request names the phases with `Phases: {list}`. Without it, run all phases (`Phases: all`).

- Manual mode examples:
  - `Follow docs/agent-workflows/issue-to-pr.md for Issue #{number}.` → all phases, in order
  - `Phases: plan. Follow docs/agent-workflows/issue-to-pr.md for Issue #{number}.`
  - `Phases: implement. Follow docs/agent-workflows/issue-to-pr.md for Issue #{number}.`
  - `Phases: pr. Follow docs/agent-workflows/issue-to-pr.md for PR #{number}.`
- Scheduled mode examples (one automation per line, each may use a different model):
  - `Scheduled mode. Follow docs/agent-workflows/issue-to-pr.md.` → all phases
  - `Scheduled mode. Phases: plan. Follow docs/agent-workflows/issue-to-pr.md.`
  - `Scheduled mode. Phases: implement. Follow docs/agent-workflows/issue-to-pr.md.`
  - `Scheduled mode. Phases: pr. Follow docs/agent-workflows/issue-to-pr.md.`

### Flows

Each issue follows one of two flows. In scheduled mode, its label decides which.

- **End-to-end flow** (`agent-ready`): plan and implement in one run, without waiting for my approval of the plan. The plan comment is still posted as a record
- **Split flow** (`agent-plan`): post the plan and stop. I approve it before implementation starts

In manual mode, the flow follows the phases I request: all phases is end-to-end, and a single phase is one step of a split flow.

---

## Scheduled Mode Rules

### Labels

| Label | Set by | Meaning |
|---|---|---|
| `agent-ready` | Me | End-to-end flow: start or resume the work |
| `agent-plan` | Me | Split flow: start or resume the plan phase |
| `agent-plan-review` | Agent | Split flow: the plan is posted and waits for my approval |
| `agent-implement` | Me | Split flow: the plan is approved; start or resume the implement phase |
| `agent-in-progress` | Agent | The agent is working on it, or its PR is in the PR phase |
| `agent-needs-input` | Agent | Waiting for my answer or decision |
| `agent-done` | Agent | Ready to merge (waiting for my merge) |
| `agent-pr` | Agent | PR created by the agent (applied to PRs) |

### Transitions

| When | Who | Issue label change |
|---|---|---|
| Starting or resuming work | Agent | `agent-ready`, `agent-plan`, or `agent-implement` → `agent-in-progress` |
| Plan posted (split flow) | Agent | `agent-in-progress` → `agent-plan-review` |
| Approving the plan | Me | `agent-plan-review` → `agent-implement` |
| Asking for a revised plan | Me | Comment on the issue, then `agent-plan-review` → `agent-plan` |
| PR opened | Agent | Stays `agent-in-progress` (the PR phase takes over) |
| Pausing for input | Agent | `agent-in-progress` → `agent-needs-input` |
| After answering | Me | `agent-needs-input` → the `resume-label` in the pause comment |
| Final report | Agent | `agent-in-progress` → `agent-done` |
| A reported PR no longer meets the final-report criteria | Agent | `agent-done` → `agent-in-progress` (see "Rechecking reported PRs") |
| Asking for more work after the report | Me | Comment on the PR or the issue, then `agent-done` → `agent-in-progress` (the PR phase picks it up) |

### Choosing a target (handle only one per run)
Check the following in order, skipping lines for phases this run does not handle, and handle the first match.

0. **Reported PR that regressed** (needs `pr`): see "Rechecking reported PRs" below
1. **PR phase** (needs `pr`): an open PR labeled `agent-pr` whose linked issue is labeled `agent-in-progress`, and that meets any of the following
   - The issue's last agent-marker comment is a `needs-input` marker with `phase=pr` for this PR (I answered and set the label back)
   - The issue or the PR has a comment from me posted after your last agent-marker post on either of them (for example, more work requested after the final report)
   - Has unhandled feedback (including new replies on threads you already replied to)
   - Has failing CI
   - Has conflicts with the base branch
   - Meets the final-report criteria (step 7)
   → Follow [phase-pr.md](phase-pr.md) from step 6 (or go to step 7 if only the last condition applies)
2. **Stalled work** (needs `plan` or `implement`, as below): the oldest open issue labeled `agent-in-progress` that has no open PR, and whose claim has expired
   - Expired: the latest active claim (not withdrawn or released) has an `until` in the past, and no agent activity (agent-marker comment or commit on its issue branch `issue-{number}-*`) came after that time
   - An issue with no claim comment (started before claims existed) is stalled when its last agent activity is more than 2 hours old
   - Decide what to resume from the expired claim's `phase` and `flow` (see "Reading plan comments" in [common-rules.md](common-rules.md) for which plan counts):
     | Expired claim | Plan posted after that claim? | Resume |
     |---|---|---|
     | `phase=plan`, `flow=split` | Yes | The plan was not handed over. Change the label to `agent-plan-review` and end; no approval exists yet |
     | `phase=plan`, `flow=split` | No | Plan phase again (needs `plan`) |
     | `phase=plan`, `flow=end-to-end` | Yes | Implement phase (needs `implement`) |
     | `phase=plan`, `flow=end-to-end` | No | Plan phase again (needs `plan`), then implement |
     | `phase=implement` (either flow) | — | Implement phase (needs `implement`), continuing on the existing issue branch. The implement claim itself shows that the plan was approved (split) or did not need approval (end-to-end) |
     | No claim (started before claims existed) | — | If the issue has a plan, its approval is unknown: change the label to `agent-plan-review` and end. Otherwise, plan phase as the split flow (needs `plan`) |
   - Before resuming, claim the issue (see "Claiming an issue" below) with the same `flow`, and say in the claim comment that the earlier run stalled and you are resuming it
3. **Implement phase** (needs `implement`): the oldest open issue labeled `agent-implement`
   → Follow [phase-implement.md](phase-implement.md)
4. **End-to-end issue** (needs `plan` and `implement`): the oldest open issue labeled `agent-ready`
   - If it has no agent-marker comment: follow [phase-plan.md](phase-plan.md), then [phase-implement.md](phase-implement.md)
   - If its last agent-marker comment is a `needs-input` marker: resume from its `phase` and `step`, continuing its `pr` if one is recorded
   - Otherwise, if it has a linked open `agent-pr`: change the label to `agent-in-progress` and end the run, so that the PR phase picks it up
   - Otherwise: resume from the plan phase, reusing the latest plan comment unless my later comments ask for changes
5. **Plan phase** (needs `plan`): the oldest open issue labeled `agent-plan`
   → Follow [phase-plan.md](phase-plan.md)

If nothing matches, end without doing anything.

### Rechecking reported PRs
A final report can stop being true before I merge: a review arrives late, CI starts failing after `main` moves, or a conflict appears. A run with `pr` checks open PRs labeled `agent-pr` whose linked issue is labeled `agent-done`, oldest first.

- Retrieve the PR state according to "Verification Rules" in [common-rules.md](common-rules.md), and compare it with the final-report criteria (step 7 in [phase-pr.md](phase-pr.md))
- The PR has **regressed** if any of the following is true:
  - A check on the current head failed
  - There is unhandled feedback, or an unresolved Orca Bot thread
  - It conflicts with the base branch
  - The head SHA differs from the one in your last final report (the PR changed after the report)
- Checks still pending are not a regression. Leave the PR for a later run
- If the PR regressed:
  1. Post a PR comment (with an agent marker) listing what no longer holds, with the retrieved evidence
  2. Change the issue label from `agent-done` to `agent-in-progress`
  3. Continue with [phase-pr.md](phase-pr.md) from step 6 in the same run, and post a new final report when the criteria hold again
- If a result cannot be retrieved, change no label. Post a PR comment saying what could not be confirmed, unless you already posted the same note for the same head SHA
- If nothing regressed, change nothing and continue with line 1 of "Choosing a target"

When resuming any issue, read the comments posted after your last agent-marker comment. Only comments identified as "Me" (see "Post Markers" in [common-rules.md](common-rules.md)) count as my answers or instructions. If there is none, treat the label change itself as "continue as is".

Whenever you start working on a chosen issue, change its label to `agent-in-progress` first, then claim it.

### Claiming an issue
A claim keeps two runs from working on the same issue, and tells later runs whether the worker is still active. Plan and implement work in scheduled mode always runs under a claim.

1. Post an issue comment starting with the claim marker (see "Post Markers" in [common-rules.md](common-rules.md)), with `until` set to 2 hours from now and the issue's `flow`
2. Read the issue comments again. Among the **active** claims (not withdrawn, not released, and `until` not yet past), the one with the smallest comment ID wins
3. If yours did not win, edit it to start with `<!-- agent-workflow:claim-withdrawn -->`, do no work on this issue, and continue with the next line of "Choosing a target"
4. While you work, keep the claim alive: edit your claim comment to move `until` to 2 hours from now after each pushed plan step, and before any action you expect to take more than 30 minutes. If one action may take longer than 2 hours, set `until` far enough to cover it
5. When your phase ends, hand the claim over in one of two ways:
   - **Release** it when the run ends the phase and stops working on the issue (PR opened, split-flow plan posted, or paused): edit it to start with `<!-- agent-workflow:claim-released -->` before changing the label, so the next run's claim is not rejected by yours
   - **Transfer** it when the same run continues into the next phase (end-to-end: plan, then implement): edit the same claim comment to `phase=implement` and move `until` to 2 hours from now. Do not post a second claim

### Pauses posted before the phase split
Older `needs-input` markers have only `step` and `pr`, without `phase` or `resume-label`. Those issues were all in the end-to-end flow, and I resume them with `agent-ready`. When a target's last agent-marker comment is such a marker:

- Take the phase from the marker: `pr` recorded or `step` 6–7 → `pr`; `step` 3–5 → `implement`; `step` 1–2 → `plan`
- If the phase is `pr`: the PR phase handles it. A run with `pr` also treats an issue labeled `agent-ready` with such a marker as a "PR phase" target (line 1 above), and changes the label to `agent-in-progress` first
- Otherwise: handle it as an "End-to-end issue" (line 4 above), resuming from that phase and `step`
  - Older steps did not require a plan comment. If the phase is `implement` and the issue has no plan comment, first run [phase-plan.md](phase-plan.md) to record one, then continue with [phase-implement.md](phase-implement.md) in the same run. The plan must reflect work already pushed to the issue branch, if any, and say which steps are done
- New pauses always use the current marker format

### Ending each phase in scheduled mode

| Phase | Flow | End of run |
|---|---|---|
| `plan` | Split | Plan posted; label `agent-plan-review` |
| `plan` | End-to-end | Continue to `implement` in the same run |
| `implement` | Both | PR opened (label stays `agent-in-progress`) or paused; never end otherwise (see "How the phase ends" in [phase-implement.md](phase-implement.md)) |
| `pr` | Both | Handle what is available now and end the run; on the final report, label `agent-done` |

### Pausing for input
Use this both when you need a question or decision answered and when you stop early (for example, 3 failed fixes, a review that never arrives, or a plan that cannot be followed).

- Post an issue comment starting with the `needs-input` marker (see "Post Markers" in [common-rules.md](common-rules.md))
  - For a question: include the options with pros, cons, and a recommendation
  - For an early stop: describe the situation and what you need from me
  - End the comment with the line: ``To resume: reply here, then change the label to `{resume-label}`.``
- Choose `resume-label` by the phase that should pick the work up again and the flow of the issue:

| Phase to resume | End-to-end flow | Split flow |
|---|---|---|
| `plan` | `agent-ready` | `agent-plan` |
| `implement` | `agent-ready` | `agent-implement` |
| `pr` | `agent-in-progress` | `agent-in-progress` |

- The flow is given by the label this run took the issue from: `agent-ready` is end-to-end; `agent-plan` and `agent-implement` are split
  - A run that took the issue from `agent-in-progress` ("Stalled work") uses the `flow` of the latest claim or plan comment. If neither records a flow, use split, so that a plan approval is never skipped

- Change the label from `agent-in-progress` to `agent-needs-input` and end the run

---

If anything is unclear, stop and ask.
