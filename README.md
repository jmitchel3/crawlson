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

The MVP core and CLI are written in Rust. `agent-browser` is the initial
execution path, integrated through its supported process and JSON interface so
the boundary remains replaceable. The first versioned journey and report
contracts are deliberately narrow: v1 and v2 are read-only, while v3 adds one
explicitly authorized, deterministic same-origin link action and v4 adds one
secret-safe `agent-browser` state-file authentication provider. See
[`ADR 0001`](docs/architecture/decisions/0001-rust-runtime-and-agent-browser-boundary.md)
for the runtime comparison, adapter lifecycle, safety ownership, and exact
fallback criteria.

The Reference Project inventory has been reduced to an
application-independent journey, evidence, finding, and guide boundary.

## Status

The Rust 0.8.0 CLI provides an independently useful authenticated
action-and-guide vertical slice. It can run an explicitly authorized journey
through `agent-browser`, import bounded exact-origin browser storage without
retaining its source or values, verify the declared role through visible UI,
retain raw evidence, follow and verify a declared same-origin link, render
focused action images, and turn a completed run into either a verified guide or
evidence-backed deterministic findings. Multiple verified runs can be compiled
into a deterministic, navigable Markdown guide collection with a separate
findings review tree and read-only integrity audit. The repository includes a
self-contained disposable-session demo of that complete loop and a
non-publishing release path for validating bundles and managed installation. No
public 0.8.0 release exists yet: license selection, namespace reservation, and
production signing-key custody remain owner decisions. Autonomous agent
exploration, mutations, and model-judged observations remain later vertical
slices.

Build and exercise the current CLI from a Rust 1.92 environment:

```console
cargo build --locked --bins
./target/debug/crawlson version
./target/debug/clson version
./target/debug/crawlson doctor
./target/debug/crawlson upgrade --check
./target/debug/crawlson --json run examples/read-only-journey.toml \
  --allow-origin http://127.0.0.1:4173
./target/debug/crawlson --json run examples/follow-link-pass.toml \
  --allow-origin http://127.0.0.1:4173 \
  --allow-action demo.follow-link-pass@1:follow-continue
./target/debug/crawlson --json run examples/authenticated-pass.toml \
  --allow-origin http://127.0.0.1:4173 \
  --auth-state /absolute/private/state.json
./target/debug/crawlson render crawlson-runs/RUN_ID \
  --journey examples/follow-link-pass.toml
./target/debug/crawlson --json guides build ./crawlson-guides.toml \
  --output ./guide-site
./target/debug/crawlson --json guides check ./crawlson-guides.toml \
  --output ./guide-site
```

`crawlson` is the canonical executable. `clson` is a small launcher that
forwards every argument and exit status to the sibling `crawlson` executable,
so `clson doctor`, `clson guides`, and `clson upgrade` have the same behavior.

### Release bundles and managed installation

Crawlson 0.8.0 defines deterministic bundles for four targets: Apple Silicon
macOS, Intel macOS, x86-64 Windows, and x86-64 GNU/Linux. Each bundle contains
the `crawlson`, `clson`, and `crawlson-demo` binaries plus the complete demo
script and journey fixtures. The demo stays in the extracted bundle; managed
installation copies only `crawlson` and `clson`.

After extracting a bundle, its bundled canonical executable provides the
first-party install path:

```console
./bin/crawlson install --from-bundle "$PWD" --prefix /absolute/path/to/bin
```

`--prefix` is the absolute destination directory for the two CLI binaries.
Installation validates the target-specific bundle manifest and every payload
before changing the destination, writes the managed-install receipt required by
the updater, and rolls back the binaries and receipt together on failure. It
does not elevate privileges or install `agent-browser`.

Rollback covers failures observed by the installer. Abrupt process or machine
termination during the final renames is not yet journal-recoverable; any
inconsistent ownership state fails closed on the next operation.

The signed release inventory authenticates complete downloadable bundles. A
separate signed updater manifest deliberately lists only one raw `crawlson`
payload per target; each raw payload must be byte-for-byte identical to that
bundle's `bin/crawlson` member. Unix managed installs can replace that verified
payload atomically. Direct Windows self-replacement remains fail-closed.
Windows managed installations check and notify without downloading the raw
payload; a user upgrades by authenticating and extracting the new bundle, then
rerunning its bundled `crawlson install` command.

Release dry runs use test-only signing keys, retain their output only as CI
artifacts, and have no permission or command capable of publishing a release.
Those artifacts are not production releases and must not be promoted. See the
[`release v1 contract`](docs/architecture/release-v1.md) for bundle layout,
signing boundaries, installer behavior, and owner-gated publication work.

`doctor` checks for a supported `agent-browser` without installing or changing
it. Pass `--json` for one machine-readable object on stdout. Operational
failures exit 1 and argument errors exit 2.

### Complete local demo

The fastest way to see the product loop is the self-contained loopback demo. It
requires Rust 1.92 and `agent-browser 0.26.x` with its browser runtime. One
supported installation path is:

```console
npm install --global --ignore-scripts agent-browser@0.26.0
agent-browser install
```

From the repository root, choose a new or empty artifact directory and run:

```console
scripts/demo.sh --output-dir ./crawlson-demo-output
```

The source-checkout command builds Crawlson. The same script included in an
extracted release bundle accepts the packaged binaries explicitly and does not
rebuild them:

```console
scripts/demo.sh \
  --crawlson-bin "$PWD/bin/crawlson" \
  --demo-bin "$PWD/bin/crawlson-demo" \
  --output-dir ./crawlson-demo-output
```

Both forms start the loopback application and run eight cases through the real
browser adapter, then compile and check the resulting guide collections:

- a passing journey that renders a Markdown guide;
- an intentional visible-text failure that renders JSON and Markdown findings,
  plus a missing-target-authorization attempt blocked before browser launch;
- a same-origin link action that is executed once, verified, and rendered as a
  guide;
- an action whose exact postcondition fails and is rendered as a finding;
- a missing-action-authorization attempt that is blocked before browser launch;
- an authenticated viewer journey whose disposable state is verified through
  visible UI and rendered as a guide; and
- a missing-state attempt that is blocked before browser launch.

The three successful journeys become one root/topic/guide Markdown tree with
byte-identical red-box/dimmed focused images. The two deterministic failures
become a separate linked review tree; that tree deliberately has no public root
guide index. Both trees are checked again without rewriting them.

The command exits successfully only when all eight produce their expected
outcomes. It prints the guide, findings, collection, and review paths and
preserves the JSON reports, raw viewport screenshots, red-box/dimmed focused
screenshots, focus metadata, browser traces, command logs, collection manifests,
guide indexes, and finding review indexes. It refuses a non-empty output
directory so a new run cannot overwrite earlier evidence.

`cargo test --workspace --all-targets --all-features --locked` keeps the real
browser integration ignored so the portable suite does not silently depend on
a local browser installation. To explicitly require the full integration, run:

```console
CRAWLSON_REAL_BROWSER=required \
  cargo test --test real_agent_browser --locked -- --ignored --nocapture
```

Set `AGENT_BROWSER_REAL_BIN` to an absolute executable path when
`agent-browser` is not on `PATH`. The required CI job runs this integration and
the documented demo, then uploads their reports, evidence, and logs even when a
step fails. See the [demo contract](docs/architecture/demo-v1.md) for its safety
and artifact guarantees.

### Journeys and action authorization

Journey v1 and v2 are strict TOML. Both support same-origin navigation, URL and
visible text checkpoints, and target capture without activating the target. V2
adds explicit capture-to-checkpoint evidence associations and render-safe
bounds. Journey v3 preserves those contracts and adds only `follow_link`: a
visible, enabled link with a declared exact same-origin destination. Journey v4
adds an external `agent-browser` state file plus a required visible role
checkpoint. Start with
[`examples/read-only-journey.toml`](examples/read-only-journey.toml), its
[`journey v2 JSON Schema`](schemas/journey-v2.schema.json), the preserved
[`journey v1 JSON Schema`](schemas/journey-v1.schema.json), the
[`follow-link example`](examples/follow-link-pass.toml), the
[`journey v3 JSON Schema`](schemas/journey-v3.schema.json), the v1
[`run-report JSON Schema`](schemas/run-report-v1.schema.json), the v2
[`action run-report JSON Schema`](schemas/run-report-v2.schema.json), the
[`authenticated example`](examples/authenticated-pass.toml), the
[`journey v4 JSON Schema`](schemas/journey-v4.schema.json), the v3
[`authenticated run-report JSON Schema`](schemas/run-report-v3.schema.json), the
[`authentication state-file contract`](docs/architecture/authentication-v1.md),
and the
[`authorized-link contract`](docs/architecture/journey-v3.md).

Every valid v1 journey contains at least one deterministic checkpoint and one
focused capture. Visible-text checkpoints and capture targets must also pass a
driver visibility check; hidden DOM text cannot produce a green result. After a
false checkpoint, remaining declared read-only steps still run to preserve the
requested evidence. A safety block or infrastructure error stops execution.

The journey's `[target].origin` is not enough by itself: every invocation must
repeat the exact authorized HTTP(S) origin with `--allow-origin`. Crawlson
normalizes scheme, hostname, and effective port, rejects unsafe documents
before browser launch, and stops when an observed redirect leaves that origin.
Journey v4 requires the `agent-browser-state-file` provider and an external
`--auth-state` path. Target and action grants are checked before that path is
accessed. Crawlson accepts only a bounded regular state document whose browser
storage matches the exact target origin, loads a neutral private temporary
copy before tracing, suppresses the path-echoing driver output, and deletes the
copy immediately. The report binds only the public provider, role, and declared
visible verification step. Loading state is not proof of authentication: the
run remains blocked unless that role-specific UI checkpoint passes. Missing,
unsupported, invalid, load-failed, blocked, and verified outcomes remain
distinct; older authentication declarations remain explicitly blocked. Cookie
entries are rejected because `agent-browser 0.26` cannot prevent a
port-agnostic cookie from reaching another port on the same hostname.

A v3 link declaration is not permission to execute it. Every `follow_link`
step requires an exact runtime grant in the form
`--allow-action JOURNEY@REVISION:STEP`. Crawlson binds the grant to the journey
digest and target origin, then verifies the current origin, link visibility,
enabled state, and exact credential-free destination before preserving the
pre-action raw and focused evidence. It dispatches one click without retry and
passes only after the observed URL exactly matches the declaration. An
off-origin destination is blocked; an uncertain post-dispatch result is an
error with an unknown action effect. Generic clicks, buttons, typing, form
submission, scripts, uploads, automated login flows, and mutations remain
unavailable as journey capabilities. Internally, the driver click is constrained
to the declared CSS selector intersected with an anchor and its exact observed
href. Following a link can still invoke application behavior, so an action grant
must not be used as permission for a side-effecting production route.

The pinned `agent-browser` network allowlist is hostname-based, not an
exact-origin interceptor. Crawlson checks the full scheme, host, and port before
and after each step and blocks an observed escape, but it cannot prove that a
redirect did not transiently contact another port or scheme on the same host
before returning. Use v3 only where that upstream limitation is acceptable;
preventive exact-origin interception is a requirement for a future driver.

Each run creates a unique directory beneath `crawlson-runs/` (or
`--output-dir`) containing `report.json`, a required browser trace, and any
requested screenshots. A `capture` step preserves the raw viewport PNG and
creates a separate derivative with a red target outline and translucent
near-black surrounding mask. Reproducible sidecar metadata records the source
digest, adjacent box/screenshot command provenance, confirmed viewport scale,
padding, colors, pinned PNG settings, and derivative digest.

Driver commands default to a 20-second deadline and normal journey execution
to a five-minute deadline. Use `--action-timeout-seconds` (1–29) and
`--run-timeout-seconds` (30–3600) to narrow or extend them. Required evidence
finalization and owned-session close receive bounded per-command cleanup grace;
an agent-browser daemon idle reaper limits orphan lifetime if Crawlson is
abruptly interrupted.

Run outcomes and process exits are stable:

| Result | Exit |
| --- | ---: |
| `passed` | 0 |
| `failed` checkpoint | 1 |
| CLI usage error | 2 |
| safety or precondition `blocked` | 3 |
| runner, driver, evidence, or cleanup `error` | 4 |

Use `--json` for exactly one versioned run report on stdout. Driver stderr is
bounded and represented by length/digest metadata, not mixed into JSON. The
report keeps `execution_outcome` and `execution_reason` separate from a later
evidence or cleanup failure, and `upstream_success` means only that the generic
agent-browser envelope succeeded before capability-specific validation.
`clson run` forwards the same arguments and exit status.

### Findings and guides

`crawlson render RUN_DIRECTORY --journey JOURNEY` is an offline consumer of a
completed run; `clson render` is identical. It launches no browser and accepts
no arbitrary output location. The CLI-supplied run directory is the only file
authority. Before writing, Crawlson strictly validates report v1, v2, or v3,
matches the exact journey digest/identity/revision/origin, checks the complete
executed step sequence, and rehashes every registered artifact. Symlinks, path escapes,
missing evidence, source drift, focus-sidecar mismatches, incomplete cleanup,
and contradictory outcomes fail closed.

A final clean pass may produce `render/guide.md` plus deterministic local copies
of its focused images, but only for passed `capture` or `follow_link` steps that
declare `guide_instruction` and have a verified raw/focused/sidecar evidence
chain. The guide links the focus image with its vivid red action-area box and
dimmed near-black surrounding page. Read-only captures are labeled as the
reader's next action. A link step is described as executed only when its one
click and exact post-action URL were both verified.

A final deterministic checkpoint failure produces `render/findings.json` and
`render/findings.md`. Each finding is `untriaged` rather than assigning invented
impact, includes the executed reproduction prefix, and links the verified run
report and trace. A later focused capture is attached only when that capture
explicitly declares v2 `evidence_for = ["earlier-checkpoint-id"]`; chronology alone
never creates evidence provenance.

`render/render-report.json` records the deterministic outcome and output
digests. Output is staged and committed as one renderer-owned directory;
repeating the same render is a byte-identical no-op, while conflicting prior
output is preserved and rejected. Guide-ready exits 0, findings-ready exits 1,
usage exits 2, valid non-publishable runs exit 3, and invalid/incomplete render
inputs exit 4. See the published
[`render report`](schemas/render-report-v1.schema.json),
[`findings v1`](schemas/findings-v1.schema.json),
[`action findings v2`](schemas/findings-v2.schema.json), and
[`render contract`](docs/architecture/render-v1.md).

Artifact hashing establishes consistency with the local run report, not
cryptographic authenticity: a party able to replace the whole unsigned bundle
could replace its hashes too. Focused screenshots and authenticated traces can
contain sensitive UI; Crawlson does not describe them as redacted or
automatically publish-safe.

### Guide collections

`crawlson guides build MANIFEST --output DIRECTORY` compiles explicitly ordered
topics and entries into one offline wiki tree. Every manifest run/journey path
must be portable and relative to the manifest. Crawlson rejects escapes,
symlinks, duplicate identities, ambiguous ordering, and overlapping input/output
roots before generation.

Collection builds do not trust or mutate an input run's prior `render/`
directory. Each raw run is copied into a bounded temporary snapshot and passed
through the normal offline renderer with its exact journey source. Only when
every entry is `guide_ready` does Crawlson emit a public `index.md`, topic
indexes, navigable per-guide pages, byte-identical focused images, and the
versioned `guide-collection.json` application boundary. That neutral document
contains ordered instructions, honest observed-versus-executed claims,
topic/audience context, and page/image provenance, so another application can
render the guide without parsing Markdown.

If any current run has deterministic findings, is blocked, or is unavailable,
Crawlson emits no partial public index. A separate `review/index.md` records the
current state; findings retain their structured document and linked evidence.
The overall exit is 1 for findings, 3 for unavailable entries, and 4 for invalid
or tampered inputs.

`crawlson guides check MANIFEST --output DIRECTORY` recomputes the expected tree
and reads the existing output without rewriting it or starting periodic updater
work. It reports stale or missing
files, dead local links, orphaned images, missing index reachability, unexpected
files, digest changes, and symlinks with stable codes. A build accepts identical
output as a no-op and preserves/rejects conflicting output; v1 has no destructive
overwrite flag. See the
[`guide collection contract`](docs/architecture/guide-collection-v1.md),
[`manifest schema`](schemas/guide-collection-manifest-v1.schema.json),
[`application schema`](schemas/guide-collection-v1.schema.json), and
[`collection report schema`](schemas/guide-collection-report-v1.schema.json).

### Updates

`crawlson upgrade --check` checks the stable channel; `crawlson upgrade`
installs a newer stable release only for an exact first-party managed-install
receipt. Cargo, Homebrew, Nix, unknown, downgrade, and prerelease cases fail
closed with an appropriate package-manager or reinstall instruction. Crawlson
never elevates privileges or upgrades `agent-browser`.

First-party managed Unix installations default to automatic compatible upgrades.
Successful checks run in a separate process no more than weekly, with persisted
per-install jitter; transient failures retry after at least a day. Foreground
commands never wait for updater network or worker completion, and worker failure
cannot change their exit status or JSON. Before 1.0, automatic replacement is
limited to patch releases in the current minor line. Other installation types
receive a cached notice only.

Release metadata must be immutable and bind each asset to GitHub's SHA-256
digest. Crawlson additionally requires a Minisign signature over the exact
raw-payload update manifest and verifies the downloaded binary before
same-directory atomic replacement on supported Unix installations. Direct
Windows self-replacement fails closed. Windows resolves even an explicit `auto`
policy to notify-only, avoids the raw payload download, and directs users to the
authenticated bundle installer so replacement and rollback happen outside the
installed executable.
Development and dry-run builds fail closed against the stable channel until a
production release public key is embedded and owner-approved signed assets are
published.

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
