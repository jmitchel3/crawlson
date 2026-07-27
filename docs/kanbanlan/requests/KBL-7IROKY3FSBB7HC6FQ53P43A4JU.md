# Choose Rust-first MVP runtime and agent-browser boundary

- Kanbanlan: `KBL-7IROKY3FSBB7HC6FQ53P43A4JU`
- Canonical home: `github`
- Canonical request: [#4](https://github.com/jmitchel3/crawlson/issues/4)

## Request

## Outcome

Select the Crawlson MVP runtime with Rust as the preferred choice, provided the agent-browser integration remains first-class, observable, and replaceable.

## Context

The installed agent-browser 0.26.0 and its official documentation describe a native Rust client/daemon with JSON output, JSON batch input, isolated sessions, safety policies, screenshots, traces, HAR capture, console errors, and non-zero process failures. This strongly favors a Rust orchestrator without coupling Crawlson to agent-browser internals. Browser and network time will dominate end-to-end latency, so the decision must distinguish runtime efficiency from page-interaction speed.

Fallback order requested by the product owner: Rust, then Python, then TypeScript.

## Acceptance criteria

- [x] Write an ADR comparing Rust, Python, and TypeScript against the MVP constraints.
- [x] Choose Rust unless a concrete agent-browser lifecycle, JSON protocol, cancellation, evidence, installation, or portability gap makes it unsuitable.
- [x] Define a process-level agent-browser adapter contract using structured input/output and isolated per-run sessions.
- [x] Define version pinning, availability checks, timeouts, cancellation, cleanup, and error mapping.
- [x] Record which safety checks stay in Crawlson core even when agent-browser offers defense-in-depth flags.
- [x] Record the exact fallback trigger that would select Python, then TypeScript.

## Scope boundaries

In scope: architecture decision and adapter contract. Out of scope: crate scaffolding, final journey/report schemas, demo application, browser execution implementation, authentication, mutation support, hosted services, or package publication.

## Likely touchpoints

docs/architecture/decisions; TODO.md; README.md if the runtime becomes part of the public contract

## Dependencies and overlap

Depends on issue #1 and PR #3, which establish the core/adapter boundary. No other open request or pull request overlaps this decision.

## Verification

Cross-check the ADR against the installed agent-browser CLI and current official agent-browser documentation. Verify every claimed command and lifecycle capability exists. Review the final diff for accidental coupling to private application details.

## Decisions

- Rust is the MVP core and CLI runtime. Browser and network work will dominate
  journey latency, so this is a packaging, resource, and correctness decision,
  not a claim that Rust accelerates web applications.
- `agent-browser` is integrated only through its supported executable and JSON
  protocol. The initial tested compatibility line is 0.26.x.
- Per-run sessions, typed errors, bounded cancellation, partial evidence, and
  core-owned safety checks are mandatory adapter behavior.
- Focused guide images are reproducible derivatives: the raw screenshot remains
  evidence, while the derivative uses a red target outline and near-black
  surrounding mask.
- Python, then TypeScript, may replace the runtime boundary only under the exact
  demonstrated compatibility triggers recorded in ADR 0001.

## Verification

- Cross-checked local `agent-browser 0.26.0` help for JSON output, JSON batch
  input, isolated sessions, screenshots, traces, target bounding boxes, network
  and console diagnostics, domain restrictions, action policies, and explicit
  session cleanup.
- Reviewed the version-matched core skill and tagged implementation details for
  response envelopes, confirmation handling, hostname-oriented restrictions,
  action-policy names, bounded output, and daemon cancellation limitations.
- Reviewed the ADR, README, TODO, and request record for private project names,
  paths, identifiers, credentials, and application-specific fixtures.
- `git diff --cached --check` passed; the new relative documentation links
  resolve to repository files, and a repository documentation search found no
  private project name or path variants.

## Delivered result

- Added ADR 0001 with the runtime comparison, process adapter contract, evidence
  lifecycle, error mapping, safety ownership, and fallback triggers.
- Updated the public product status and implementation backlog to record the
  decision and the focused-action screenshot requirement.
- Follow-up work remains intentionally separate: CLI scaffolding and upgrades,
  journey/report schemas, adapter implementation, demo journeys, and releases.
