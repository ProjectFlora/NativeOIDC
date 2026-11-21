# Issue Workflow (Issue → Pull Request → Ready to Merge)

This is the entry point for handling development work based on a GitHub Issue. It defines the execution modes, the phases, and how scheduled runs choose their work. Read [common-rules.md](common-rules.md) before any phase.

## Execution modes

- **Manual mode**: I specify the issue or PR number. Ask questions in this session and wait for my answer.
- **Scheduled mode**: Started by an Orca automation. Pick the target yourself; post questions as Issue comments and end the run.

If the request does not explicitly say "scheduled mode", treat it as manual mode.

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
| Asking for more work after the report | Me | Comment on the PR or the issue, then `agent-done` → `agent-ready` |

### Choosing a target (handle only one per run)
Check the following in order, skipping lines for phases this run does not handle, and handle the first match.

1. **PR phase** (needs `pr`): an open PR labeled `agent-pr` whose linked issue is labeled `agent-in-progress`, and that meets any of the following
   - The issue's last agent-marker comment is a `needs-input` marker with `phase=pr` for this PR (I answered and set the label back)
   - Has unhandled feedback (including new replies on threads you already replied to)
   - Has failing CI
   - Has conflicts with the base branch
   - Meets the final-report criteria (step 7)
   → Follow [phase-pr.md](phase-pr.md) from step 6 (or go to step 7 if only the last condition applies)
2. **Implement phase** (needs `implement`): the oldest open issue labeled `agent-implement`
   → Follow [phase-implement.md](phase-implement.md)
3. **End-to-end issue** (needs `plan` and `implement`): the oldest open issue labeled `agent-ready`
   - If it has no agent-marker comment: follow [phase-plan.md](phase-plan.md), then [phase-implement.md](phase-implement.md)
   - If its last agent-marker comment is a `needs-input` marker: resume from its `phase` and `step`, continuing its `pr` if one is recorded
   - Otherwise, if it has a linked open `agent-pr`: resume that PR from step 6 (needs `pr`; otherwise skip this issue)
   - Otherwise: resume from the plan phase, reusing the latest plan comment unless my later comments ask for changes
4. **Plan phase** (needs `plan`): the oldest open issue labeled `agent-plan`
   → Follow [phase-plan.md](phase-plan.md)

If nothing matches, end without doing anything.

When resuming any issue, read the comments posted after your last agent-marker comment. Only comments identified as "Me" (see "Post Markers" in [common-rules.md](common-rules.md)) count as my answers or instructions. If there is none, treat the label change itself as "continue as is".

Whenever you start working on a chosen issue, change its label to `agent-in-progress` first (to prevent duplicate work).

### Ending each phase in scheduled mode

| Phase | Flow | End of run |
|---|---|---|
| `plan` | Split | Plan posted; label `agent-plan-review` |
| `plan` | End-to-end | Continue to `implement` in the same run |
| `implement` | Both | PR opened; label stays `agent-in-progress`; end the run |
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

- Change the label from `agent-in-progress` to `agent-needs-input` and end the run

---

If anything is unclear, stop and ask.
