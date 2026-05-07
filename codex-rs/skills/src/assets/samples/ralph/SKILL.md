---
name: "ralph"
description: "Use when creating, reviewing, or repairing Ralph autonomous-loop documentation in a repository, especially `.ralph/PROMPT.md`, `.ralph/@AGENT.md`, `.ralph/@fix_plan.md`, and `.ralph/@fix_progress.md`; teaches the agent how to keep the long-term plan, architecture constraints, progress refinement, completion ticks, decisions, and loop handoff notes coherent."
---

# Ralph

Use this skill when a user asks to set up or improve Ralph documents for an autonomous development loop.

## Before Long-Running Ralph

Before asking Ralph to start a long autonomous development loop, first discuss
requirements decomposition and the technical route with the user or current
agent. A wrong route makes later Ralph work wasteful.

Use a grill-me style interview before writing or finalizing the anchors:

- ask one question at a time;
- walk the decision tree branch by branch;
- resolve dependencies between decisions in order;
- provide a recommended answer for each question;
- if the answer can be discovered from the codebase, inspect the codebase
  instead of asking the user;
- stop only when the minimum deliverable, route, risks, and validation path are
  explicit enough to write stable anchors.

Confirm:

- the minimum deliverable for the current stage;
- the core architecture layers;
- which modules must be built first and which can wait;
- the highest-risk assumptions that should be validated early;
- whether the approach is testable, maintainable, and extensible.

Write the settled long-term decisions into `.ralph/@fix_plan.md`. Write the
derived milestones, refined tasks, validation gates, open questions, and next
handoff into `.ralph/@fix_progress.md`.

## Document Roles

- `.ralph/@fix_plan.md`: stable long-term plan and architecture constraint anchor. It is edited by the user before a run and treated as read-only during the loop.
- `.ralph/@fix_progress.md`: mutable plan-refinement anchor. It turns the plan into milestones, refined tasks, completion ticks, decisions, deviations, and per-loop handoff notes.
- `.ralph/PROMPT.md`: runtime instructions for Ralph. It must tell the agent to read the plan, work from progress, and update progress, not the plan.
- `.ralph/@AGENT.md`: project-specific build, test, run, and local engineering notes.

## Core Contract

- Plan is the durable source of direction: product goal, long-term milestones, architecture constraints, non-goals, and completion definition.
- Progress is the changing source of execution state: current refinement, task checklist, decisions/deviations, notes, and auto log.
- Do not tell Ralph to tick, rewrite, reorder, or append runtime progress to `@fix_plan.md`.
- Completion is based on `@fix_progress.md` showing that the plan has been refined and completed without violating plan constraints.
- A task is not complete until its relevant test or validation command has passed, or the progress file records why validation is not applicable.

## Testing Framework First

Ralph is intended for unattended development, so establish automatic validation
early, ideally before substantial feature work. Choose the layers that fit the
project:

- type checking;
- unit tests;
- integration tests;
- random/property tests;
- browser E2E tests;
- visual regression tests;
- long-running or durability tests;
- build verification.

Record the chosen validation commands in `.ralph/@AGENT.md` and mirror task
completion rules in `.ralph/@fix_progress.md`.

## Project Constraints

Write clear project constraints in `.ralph/` so Ralph does not drift during
automation. Capture at least:

- technology stack;
- project directory structure;
- architecture layers and dependency direction;
- modules that must not depend on each other;
- ownership of state;
- rules against deleting tests, lowering coverage, or bypassing validation;
- which docs and progress files must be updated after each loop.

## Creation Workflow

1. Read any user goal, specs, existing docs, and repo conventions.
2. Run a grill-me style interview to settle the minimum deliverable, architecture route, module order, highest-risk validations, and open decisions before writing long-loop docs.
3. Write `@fix_plan.md` as a stable planning artifact:
   - `Product Goal`
   - `Long-Term Milestones`
   - `Architecture Constraints`
   - `Non-Goals`
   - `Completion Definition`
4. Write `@fix_progress.md` as the active execution artifact:
   - `Current Refinement`
   - `Refined Task Checklist`
   - `Decisions And Deviations`
   - `Notes`
   - preserve `<!-- RALPH_WIDEX_AUTOLOG_START -->` and `<!-- RALPH_WIDEX_AUTOLOG_END -->`
5. Align `PROMPT.md` so each loop reads plan and progress, chooses the next refined task from progress, updates progress, and verifies relevant tests before marking work complete.
6. Align `@AGENT.md` with real project commands and validation layers. Avoid fake mandatory coverage, commit, push, or server-running requirements unless the project explicitly needs them.
7. Scan the four files for contradictions before finishing.

## Consistency Checklist

- `@fix_plan.md` never says Ralph should update it during a run.
- `@fix_progress.md` clearly owns task ticks, current state, decisions, deviations, and handoff notes.
- Grill-me outcomes are split correctly: durable decisions go to `@fix_plan.md`; derived tasks, gates, open questions, and next actions go to `@fix_progress.md`.
- `PROMPT.md` does not say "update fix_plan", "mark plan complete", or "choose next task from plan" when progress exists.
- `@AGENT.md` does not require commit/push or coverage thresholds unless they are real project requirements.
- `@AGENT.md` lists the real validation commands Ralph should use.
- `@fix_progress.md` says a refined task can be ticked only after relevant validation passes or an explicit exception is recorded.
- Auto-log markers in `@fix_progress.md` remain present and unchanged.

## Minimal Templates

Use these shapes when there is no stronger project-specific structure.

```markdown
# Ralph Long-Term Plan

## Product Goal

## Long-Term Milestones

## Architecture Constraints

## Non-Goals

## Completion Definition
```

```markdown
# Ralph Progress

## Current Refinement

## Refined Task Checklist (editable)

## Decisions And Deviations (editable)

## Notes (editable)

<!-- RALPH_WIDEX_AUTOLOG_START -->

## Auto log (managed by ralph-widex; do not edit)

<!-- RALPH_WIDEX_AUTOLOG_END -->
```
