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

## Possible shape

The initial tool may include:

- a small declarative format for user journeys;
- an initial session runner built around `agent-browser`;
- target, authentication, and mutation guardrails;
- structured findings with screenshots and traces;
- replay and regression verification;
- Markdown guide generation from verified runs; and
- local CLI and CI integrations.

The implementation language, public API, journey format, and package layout are
intentionally undecided. `agent-browser` is the initial execution path, while
its integration boundary should remain replaceable. The first design task is to
reduce the existing RangerTrac guide workflow to the smallest useful,
application-independent core.

## Status

Concept stage. This repository is a placeholder for extracting and developing
Crawlson as a dedicated open-source tool.

Start with [`TODO.md`](TODO.md) for the proposed extraction plan, first vertical
slice, open design decisions, and MVP acceptance criteria.
