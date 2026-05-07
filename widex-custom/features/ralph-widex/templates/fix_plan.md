# Ralph Long-Term Plan

This file is Ralph's stable long-term plan and architecture constraint anchor.
After the user finishes editing it, Ralph treats it as read-only across all
loops. Do not tick, rewrite, reorder, or append progress here during an
autonomous run.

Use `.ralph/@fix_progress.md` to refine this plan into milestones, concrete
tasks, completion ticks, decisions, and handoff notes.

## Product Goal

- Define the durable outcome Ralph is working toward.
- Keep this section strategic, not per-loop or per-task.

## Long-Term Milestones

- Milestone 1:
- Milestone 2:
- Milestone 3:

## Architecture Constraints

- Preserve existing public APIs unless the plan explicitly allows a breaking change.
- Prefer small, cohesive modules over broad rewrites.
- Keep state, configuration, and side effects explicit.
- Add dependencies only when they reduce real complexity and fit the project.
- Keep tests focused on behavior and changed surfaces.

## Non-Goals

- Do not add features outside this plan.
- Do not refactor unrelated code for style-only reasons.
- Do not treat temporary progress notes as architecture decisions.

## Completion Definition

Ralph may consider the plan complete only when:

- the long-term milestones above have corresponding completed items in
  `.ralph/@fix_progress.md`;
- architecture constraints have not been violated, or approved deviations are
  recorded in `.ralph/@fix_progress.md`;
- relevant tests and documentation are updated for the completed scope.
