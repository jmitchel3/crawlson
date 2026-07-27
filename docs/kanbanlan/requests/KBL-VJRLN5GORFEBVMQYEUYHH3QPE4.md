# Inventory Reference Project guide lifecycle and define MVP boundaries

- Kanbanlan: `KBL-VJRLN5GORFEBVMQYEUYHH3QPE4`
- Canonical home: `github`
- Canonical request: [#1](https://github.com/jmitchel3/crawlson/issues/1)

## Request

## Outcome

Document the Reference Project guide lifecycle and derive the smallest application-independent Crawlson MVP boundary from direct source evidence.

## Context

Crawlson is at concept stage. AGENTS.md and TODO.md require review of the source guide system before selecting a runtime or scaffolding an implementation. No open issues or pull requests overlap this work as of 2026-07-26.

## Acceptance criteria

- [ ] Inventory define, authenticate, execute, verify, capture, render, index, and publish stages.
- [ ] Trace one public read-only verification, one authenticated role-specific workflow, and one mutating fixture workflow with cleanup.
- [ ] Record reusable concepts versus Reference Project-specific behavior and known failure modes.
- [ ] Recommend the smallest independently useful MVP core and adapter boundaries without selecting a language or framework.
- [ ] Update TODO.md to reflect completed Phase 0 discovery work.

## Scope boundaries

In scope: source-system analysis, architecture boundary recommendations, and documentation updates. Out of scope: runtime selection, journey schema finalization, package scaffolding, browser execution, or copying Reference Project fixtures and credentials.

## Likely touchpoints

README.md; TODO.md; docs/architecture; docs/kanbanlan/requests

## Dependencies and overlap

None. Live GitHub issues and pull requests were empty when scoped.

## Verification

Cross-check the inventory against the named Reference Project source files and representative capture/verify specs; inspect repository diffs and documentation links.

## Decisions

- Keep one executed journey as the source of truth for evidence, findings, and
  optional guide output; do not reproduce the Reference Project's paired capture/verify
  definitions.
- Put safety policy, explicit outcomes, ordered events, evidence provenance, and
  report data in the core.
- Keep browser drivers, authentication, application fixtures, agent runtimes,
  artifact storage, and renderers behind replaceable boundaries.
- Make the first executable slice public and read-only with `agent-browser`.
  Defer authentication, mutation, guide generation, and hosting until the core
  result can fail closed and preserve evidence.
- Defer language and framework selection to a separate architecture decision.

## Verification

- Cross-checked the lifecycle against the guide goal, drift skill, both
  Playwright configurations, mutation guard, screenshot helper, authentication
  setup, deployment ingestion path, and three representative workflows at
  the reviewed Reference Project revision.
- Confirmed no open Crawlson issues or pull requests overlapped this request
  before claim.
- `git diff --cached --check` passes.
- Confirmed every named primary source and representative workflow file exists
  at the reviewed Reference Project revision.
- Confirmed the new inventory and request record contain no copied credential
  variable names, test codes, or email addresses.
- Confirmed the README and TODO links resolve to the new inventory file.

## Delivered result

- Added `docs/architecture/reference-project-guide-lifecycle.md` with the lifecycle
  inventory, three traces, reusable/application-specific split, failure modes,
  and the smallest useful core/adapter boundary.
- Updated `TODO.md` to mark only the completed Phase 0 discovery items.
- Linked the source inventory from `README.md`.
- Remaining Phase 0 work is intentionally separate: preserve sanitized design
  fixtures and choose the MVP language/runtime through an architecture decision.
