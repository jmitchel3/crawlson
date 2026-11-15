# AGENTS.md

This file gives repository-specific guidance to AI coding agents working on
Crawlson.

## Product intent

Crawlson runs agent-driven browser sessions that behave like real users. Its
primary purpose is to surface user-facing bugs with reproducible evidence.
Verified guides are a useful output of successful sessions, not the central
product.

Keep the product model clear:

> Journey -> agent-run browser session -> evidence -> findings and guides

Do not reframe Crawlson as a conventional crawler, generic browser automation
library, screenshot tool, or prose-first guide authoring system.

## Current phase

The project is at the concept stage. Before scaffolding an implementation:

1. Review `README.md` and the Reference Project guide system.
2. Review `TODO.md`; keep it current as decisions are made and work lands.
3. Define the smallest independently useful MVP.
4. Decide which behavior belongs in the core and which belongs in adapters.
5. Record important architecture and safety decisions in the repository.

Do not select a language, framework, package structure, or hosted-service model
merely because the source system happens to use it. The explicit exception is
the first browser execution path: start with `agent-browser`, as stated in the
product concept, while keeping the boundary replaceable.

## Product principles

- Test observable user behavior through the visible UI.
- Preserve screenshots, traces, findings, and reproducible steps as evidence.
- Treat skipped, blocked, unauthenticated, and incomplete runs as explicit
  outcomes. They must not become false-green passes.
- Keep journey definitions and generated guides from drifting apart.
- Keep browser drivers, agent runtimes, authentication, application fixtures,
  and renderers behind replaceable boundaries.
- Use `agent-browser` for the first working vertical slice. Keep the integration
  behind a boundary so later drivers do not change the journey or report model.
- Prefer deterministic checks where the expected behavior is known and agent
  judgment where exploration or usability assessment is valuable.
- Do not claim that Crawlson fixes bugs unless a verified repair loop has
  actually been implemented.

## Browser-session safety

- Operate only against targets the user has explicitly authorized.
- Default to read-only sessions.
- Require an allowlisted target before any authenticated or mutating run.
- Fail closed when target validation, authentication, or safety configuration is
  missing.
- Use disposable users and fixtures for journeys that create, update, submit, or
  delete data.
- Make mutations and cleanup visible in the journey definition and run report.
- Never weaken authentication, authorization, anti-abuse, or security controls
  to make a journey pass.
- Never run a mutating journey against production without explicit user
  authorization for that exact target and operation.

## Working conventions

- Keep changes small enough to verify and review.
- Add tests for implemented behavior and failure modes.
- Update the README when the product contract or public workflow changes.
- Document assumptions instead of silently encoding application-specific
  Reference Project behavior in the core.
- Preserve user work and avoid destructive repository or browser actions unless
  explicitly authorized.

## Reference Project review

The system that motivated Crawlson is a private Reference Project. Its checkout
location, repository identity, commit history, secrets, credentials, customer
data, and application-specific fixtures must not be recorded in this repository.

Begin with these files and directories:

- `.agents/skills/guides-drift/SKILL.md`
- `docs/guides/goal-parameters.md`
- `docs/guides/todo.md`
- `docs/guides/wiki/`
- `e2e/guides/`
- `e2e/guides/playwright.guides.config.ts`
- `e2e/guides/playwright.verify.config.ts`
- `e2e/guides/stage-mutation-guard.ts`

Treat the Reference Project as a case study and compatibility fixture, not as
Crawlson's architecture. Do not move secrets, credentials, customer data, or
application-specific fixtures into this repository.

Known source-system problems that Crawlson must solve deliberately:

- verification steps and Markdown guides can drift because they have separate,
  hard-coded sources of truth;
- missing authentication can cause suites to self-skip and appear green;
- mutation guards are not uniformly applied;
- capture and verification behavior are duplicated across paired specs;
- guide verification is not a required CI signal; and
- guide status is implicit rather than represented by a clear result model.

<!-- kanbanlan:start -->
## Request Coordination Workflow

Requests live on [jmitchel3 Project 2](https://github.com/users/jmitchel3/projects/2).
Details: `docs/workflow/kanbanlan.md`.

- Before coding, run `kanbanlan start <ref-or-title> --touchpoints "..."` and
  work only in the worktree it prints.
- If `start` refuses, or lists another claim that conflicts semantically with
  this work, stop and ask.
- One card per session. Do not expand its scope into another outcome.
- Write decisions and verification in the pull request body.
- When the work is ready, run `kanbanlan done` (it opens or finds the pull
  request). `kanbanlan drop` abandons the card.
- Status questions are read-only. "Remember this" means `kanbanlan admin capture`.
<!-- kanbanlan:end -->
