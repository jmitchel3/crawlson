# Crawlson

> **Guided agent-browser sessions make for bug squashing and guide building.**
>
> His name is Robert Crawlson.

Crawlson runs browser agents through intentional, stateful user journeys. It
surfaces user-facing bugs, preserves reproducible evidence, and can turn
verified sessions into guides grounded in how the product actually works.

> Journey -> agent-run browser session -> evidence -> findings and guides

## The idea

Most automated tests prove that code behaves as expected. Crawlson approaches
the product from the other direction: it uses the interface like a person and
reports what that person would encounter.

A Crawlson session should be able to:

- act as a specific user or role in a real browser;
- complete an intentional workflow across multiple pages;
- notice broken behavior, confusing states, dead ends, and regressions;
- save screenshots, traces, observations, and reproducible steps;
- distinguish verified behavior from inferred or untested behavior; and
- render a successful journey as a human-readable guide when useful.

Guides are an output, not the product. The source of truth is an executable,
observable user journey.

## What Crawlson is not

- A conventional breadth-first web crawler or SEO indexer
- A screenshot recorder presented as a testing system
- A prose-first guide generator that invents steps it has not performed
- A replacement for unit, integration, accessibility, or security testing
- An excuse to let an agent make uncontrolled changes to production data

## Principles

1. **Behave like a user.** Exercise visible UI paths instead of calling private
   application internals to manufacture success.
2. **Show the evidence.** Every finding should include enough context to inspect
   and reproduce it.
3. **Fail closed.** Missing credentials, unsafe targets, and incomplete sessions
   are failures or explicit blocked results, never silent passes.
4. **Separate observation from mutation.** Read-only verification should be the
   default. Mutating journeys require explicit authorization and disposable
   data with cleanup.
5. **Keep artifacts honest.** A generated guide must describe a journey Crawlson
   actually completed.
6. **Stay runner-agnostic.** Browser drivers, agent runtimes, report formats, and
   application adapters should be replaceable.

## Initial shape

The initial tool may include:

- a small declarative format for user journeys;
- an initial session runner built around `agent-browser`;
- target, authentication, and mutation guardrails;
- structured findings with screenshots and traces;
- replay and regression verification;
- Markdown guide generation from verified runs; and
- local CLI and CI integrations.

The MVP core and CLI will be written in Rust. `agent-browser` is the initial
execution path, integrated through its supported process and JSON interface so
the boundary remains replaceable. The public API, journey format, and package
layout are still intentionally undecided. See
[`ADR 0001`](docs/architecture/decisions/0001-rust-runtime-and-agent-browser-boundary.md)
for the runtime comparison, adapter lifecycle, safety ownership, and exact
fallback criteria.

The first design task is to reduce the existing Reference Project guide
workflow to the smallest useful, application-independent core.

## Status

Early development. The Rust 0.1.0 command foundation is working; the safe
journey runner and guide renderer are the next vertical slices.

Build and exercise the current CLI from a Rust 1.92 environment:

```console
cargo build --bins
./target/debug/crawlson version
./target/debug/clson version
./target/debug/crawlson doctor
./target/debug/crawlson upgrade --check
```

`crawlson` is the canonical executable. `clson` is a small launcher that
forwards every argument and exit status to the sibling `crawlson` executable,
so `clson doctor` and `clson upgrade` have the same behavior.

`doctor` checks for a supported `agent-browser` without installing or changing
it. Pass `--json` for one machine-readable object on stdout. Operational
failures exit 1 and argument errors exit 2.

### Updates

`crawlson upgrade --check` checks the stable channel; `crawlson upgrade`
installs a newer stable release only for an exact first-party managed-install
receipt. Cargo, Homebrew, Nix, unknown, downgrade, and prerelease cases fail
closed with an appropriate package-manager or reinstall instruction. Crawlson
never elevates privileges or upgrades `agent-browser`.

First-party managed installations default to automatic compatible upgrades.
Successful checks run in a separate process no more than weekly, with persisted
per-install jitter; transient failures retry after at least a day. Foreground
commands never wait for updater network or worker completion, and worker failure
cannot change their exit status or JSON. Before 1.0, automatic replacement is
limited to patch releases in the current minor line. Other installation types
receive a cached notice only.

Release metadata must be immutable and bind each asset to GitHub's SHA-256
digest. Crawlson additionally requires a Minisign signature over the exact
update manifest and verifies the downloaded binary before same-directory atomic
replacement on supported Unix installations. Direct Windows self-replacement
fails closed until rollback is proven; Windows upgrades use the installer.
Development builds fail closed until a release public key is embedded and
signed assets are published.

Periodic update work is disabled by `CI`, `DO_NOT_TRACK=1`,
`CRAWLSON_NO_UPDATE_CHECK=1`, `CRAWLSON_OFFLINE=1`, or
`CRAWLSON_AUTO_UPGRADE=0`. Set `CRAWLSON_UPDATE_POLICY` to `auto`, `notify`, or
`off`; the equivalent config is `[updates] mode = "..."`. Update requests go
only to the fixed Crawlson GitHub release endpoints with a version user agent;
they contain no journey, target, finding, credential, or host telemetry.

Start with [`TODO.md`](TODO.md) for the proposed extraction plan, first vertical
slice, open design decisions, and MVP acceptance criteria. The
[`Reference Project guide lifecycle inventory`](docs/architecture/reference-project-guide-lifecycle.md)
records the source-system findings and the recommended core/adapter boundary for
that first slice.
