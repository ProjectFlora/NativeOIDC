# Phase: Plan (Steps 1–2)

Investigate the issue and write a plan concrete enough that another agent, possibly a different model in a new session, can implement it without asking. Read [common-rules.md](common-rules.md) first.

This phase changes no code and opens no PR.

## Before starting
- In scheduled mode, claim the issue first (see "Claiming an issue" in [issue-to-pr.md](issue-to-pr.md)), with `phase=plan`

## 1. Understand the issue
- Read the issue body and comments with `gh issue view {number} --comments`, and review the related code
- If an earlier plan comment exists, read it and every comment from me after it (see "Post Markers" in [common-rules.md](common-rules.md)). My comments after a plan are change requests for the next plan
- Identify the goal, the acceptance criteria, and the scope of impact

## 2. Decide on the approach and post the plan
- If something cannot be decided on your own, or requires a specification decision, present options (pros, cons, recommendation) and do not post a plan until you get an answer
  - Manual mode: ask in this session and wait for my answer
  - Scheduled mode: follow "Pausing for input" in [issue-to-pr.md](issue-to-pr.md)
- Post the plan as an issue comment, starting with `<!-- agent-workflow:plan flow={end-to-end or split} -->`, in this format:

```markdown
<!-- agent-workflow:plan flow={end-to-end or split} -->
## Plan for #{number}

### Goal
{One or two sentences}

### Acceptance criteria
- {Each condition that must be true when the work is done}

### Approach
{The chosen approach, and why. Name the options you rejected if a choice was made}

### Changes
| File | Change |
|---|---|
| `{path}` | {What changes} |

### Steps
1. {Concrete step, in implementation order}

### Tests
- {Tests to add or update, and the commands to run them}

### Effort estimate
{Very High, High, Medium, Low, or Very Low}: {one line on why, such as the number of files and steps, and the risk}

### Out of scope
- {Related problems you noticed but will not fix}
```

- Estimate the Effort from the plan itself: about `Very Low` for a change to one file with no new tests, up to `Very High` for changes across modules, public API, or security-sensitive logic. A smaller model may implement plans estimated `Low` or `Very Low`, so keep those plans short, with exact files and steps
- A plan is ready only when it has no open questions. Do not leave "TBD" or "to be decided during implementation"
- Each new plan comment replaces the previous one. The implement phase uses the latest plan (see "Reading plan comments" in [common-rules.md](common-rules.md))

## When the phase ends
- Manual mode: share the plan in this session and stop. I start the implement phase separately, or continue in this session if I asked for all phases
- Scheduled mode: see "Ending each phase in scheduled mode" in [issue-to-pr.md](issue-to-pr.md)
  - Split flow: change the label from `agent-in-progress` to `agent-plan-review` and end the run
  - End-to-end flow: continue to [phase-implement.md](phase-implement.md) in the same run
