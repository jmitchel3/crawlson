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
contracts are deliberately narrow and read-only. See
[`ADR 0001`](docs/architecture/decisions/0001-rust-runtime-and-agent-browser-boundary.md)
for the runtime comparison, adapter lifecycle, safety ownership, and exact
fallback criteria.

The Reference Project inventory has been reduced to an
application-independent journey, evidence, finding, and guide boundary.

## Status

The Rust 0.5.1 CLI provides the first independently useful read-only vertical
slice. It can run an explicitly authorized journey through `agent-browser`,
retain raw evidence, render focused action images, and turn a completed run
into either a verified guide or evidence-backed deterministic findings. The
repository includes a credential-free demo of that complete loop and a
non-publishing release path for validating bundles and managed installation.
No public 0.5.1 release exists yet: license selection, namespace reservation,
and production signing-key custody remain owner decisions. Autonomous agent
exploration, authentication execution, mutations, and model-judged observations
remain later vertical slices.

Build and exercise the current CLI from a Rust 1.92 environment:

```console
cargo build --locked --bins
./target/debug/crawlson version
./target/debug/clson version
./target/debug/crawlson doctor
./target/debug/crawlson upgrade --check
./target/debug/crawlson --json run examples/read-only-journey.toml \
  --allow-origin http://127.0.0.1:4173
./target/debug/crawlson render crawlson-runs/RUN_ID \
  --journey examples/read-only-journey.toml
```

`crawlson` is the canonical executable. `clson` is a small launcher that
forwards every argument and exit status to the sibling `crawlson` executable,
so `clson doctor` and `clson upgrade` have the same behavior.

### Release bundles and managed installation

Crawlson 0.5.1 defines deterministic bundles for four targets: Apple Silicon
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

Both forms start the read-only loopback application and run three cases through
the real browser adapter:

- a passing journey that renders a Markdown guide;
- an intentional visible-text failure that renders JSON and Markdown findings;
  and
- a missing-authorization attempt that is blocked before browser launch.

The command exits successfully only when all three produce their expected
outcomes. It prints the guide and findings paths and preserves the JSON reports,
raw viewport screenshots, red-box/dimmed focused screenshots, focus metadata,
browser traces, and command logs. It refuses a non-empty output directory so a
new run cannot overwrite earlier evidence.

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

### Read-only journeys

Journey v1 and v2 are strict TOML. Both support same-origin navigation, URL and visible
text checkpoints, and target capture without activating the target. V2 adds
explicit capture-to-checkpoint evidence associations and render-safe bounds.
Start with
[`examples/read-only-journey.toml`](examples/read-only-journey.toml), its
[`journey v2 JSON Schema`](schemas/journey-v2.schema.json), the preserved
[`journey v1 JSON Schema`](schemas/journey-v1.schema.json), the
[`run-report JSON Schema`](schemas/run-report-v1.schema.json), and the
[`journey/report contract`](docs/architecture/journey-v1.md).

Every valid v1 journey contains at least one deterministic checkpoint and one
focused capture. Visible-text checkpoints and capture targets must also pass a
driver visibility check; hidden DOM text cannot produce a green result. After a
false checkpoint, remaining declared read-only steps still run to preserve the
requested evidence. A safety block or infrastructure error stops execution.

The journey's `[target].origin` is not enough by itself: every invocation must
repeat the exact authorized HTTP(S) origin with `--allow-origin`. Crawlson
normalizes scheme, hostname, and effective port, rejects unsafe documents
before browser launch, and stops when an observed redirect leaves that origin.
An authentication requirement is explicit `blocked` until a replaceable
authentication adapter exists; it is never skipped or treated as passing.

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
authority. Before writing, Crawlson strictly validates report v1, matches the
exact journey digest/identity/revision/origin, checks the complete executed step
sequence, and rehashes every registered artifact. Symlinks, path escapes,
missing evidence, source drift, focus-sidecar mismatches, incomplete cleanup,
and contradictory outcomes fail closed.

A final clean pass may produce `render/guide.md` plus deterministic local copies
of its focused images, but only for passed `capture`
steps that declare `guide_instruction` and have a verified raw/focused/sidecar
evidence chain. The guide links the focus image with its red action-area box and
dimmed surrounding page. Because read-only journey v1 observes but never clicks
or types, the authored instruction is labeled as the reader's next action; the
guide does not claim Crawlson executed it.

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
[`findings`](schemas/findings-v1.schema.json), and
[`render contract`](docs/architecture/render-v1.md).

Artifact hashing establishes consistency with the local run report, not
cryptographic authenticity: a party able to replace the whole unsigned bundle
could replace its hashes too. Focused screenshots can contain sensitive pixels;
Crawlson does not describe them as redacted or automatically publish-safe.

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
