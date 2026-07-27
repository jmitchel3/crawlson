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

1. Review `README.md` and the source RangerTrac guide system.
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
  RangerTrac behavior in the core.
- Preserve user work and avoid destructive repository or browser actions unless
  explicitly authorized.

## Source-system reference

The system that motivated Crawlson currently lives at:

`/Users/jmitch/Clients/rangertrac.org`

Begin with these files and directories:

- `.agents/skills/guides-drift/SKILL.md`
- `docs/guides/goal-parameters.md`
- `docs/guides/todo.md`
- `docs/guides/wiki/`
- `e2e/guides/`
- `e2e/guides/playwright.guides.config.ts`
- `e2e/guides/playwright.verify.config.ts`
- `e2e/guides/stage-mutation-guard.ts`

Treat RangerTrac as a case study and compatibility fixture, not as Crawlson's
architecture. Do not move secrets, credentials, customer data, or
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

The canonical kanban home is `github`. GitHub Issues currently
store canonical requests, and [jmitchel3 Project
3](https://github.com/users/jmitchel3/projects/3) is their projection. Repository policy
and durable delivery records remain versioned here. Follow
`docs/workflow/kanbanlan.md`.

- At session start run `kanbanlan ensure`. Before mutations run
  `kanbanlan reconcile` and inspect all open cards and pull requests for
  semantic overlap. If live coordination state is unavailable, do not start
  potentially overlapping implementation.
- Status questions are read-only. “Remember this” creates an Inbox card.
  “Let's work on this” authorizes a live overlap check and one claim.
- Create or reuse one request per independently reviewable outcome. Each request
  has a provider-independent Kanbanlan ID. One session
  may own exactly one `status:in-progress` card, and one card may have exactly
  one active session. Claim with
  `kanbanlan claim <kanbanlan-id-or-provider-ref> --touchpoints ...`.
- Use a dedicated request branch and worktree. Block semantic conflicts even when
  filenames differ. Do not expand a claimed card into another useful outcome.
- Run `kanbanlan record <kanbanlan-id-or-provider-ref>` in the implementation
  worktree and complete its durable decisions, verification, and delivered result.
- A pull request closes its issue and moves it to In review. Ownership lasts
  until merge, explicit release, or handoff. Project Done means delivered to
  `main`; production readiness still requires staging review.
<!-- kanbanlan:end -->
