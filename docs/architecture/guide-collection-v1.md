# Offline guide collection contract v1

Crawlson 0.7 adds an offline collection layer over the existing journey, run,
evidence, finding, and single-run render contracts:

> collection manifest + verified runs + exact journey sources -> guide wiki or review tree

The collection layer does not launch a browser, propose actions, or publish to
a hosted service. It composes results that the normal renderer can independently
revalidate. Presentation metadata belongs in the collection manifest so topic,
audience, slug, and ordering choices do not leak into the runner's behavior or
become a second action source.

## Commands

The canonical commands are:

```console
crawlson --json guides build crawlson-guides.toml --output ./guide-site
crawlson --json guides check crawlson-guides.toml --output ./guide-site
```

`clson guides ...` is equivalent. `build` creates a missing output directory,
accepts a byte-identical existing directory as an idempotent no-op, and rejects
a differing directory without changing it. `check` is strictly read-only,
including suppression of periodic updater network and state work. It
recomputes the expected collection from the manifest, raw run directories, and
journey sources, then compares and audits the existing output.

No `--force`, implicit scan, glob, timestamp winner, or deletion operation
exists in v1. To preserve an older collection and build a changed one, select a
new output directory. Destructive pruning and hosted activation remain adapter
concerns.

Exit codes follow the existing renderer precedence:

| Exit | Meaning |
| ---: | --- |
| 0 | every entry is guide-ready and the public collection is complete |
| 1 | current deterministic findings were retained for review |
| 2 | command-line usage error |
| 3 | a current entry is blocked/unavailable, or `check` found stale or missing output |
| 4 | invalid/tampered input, unsafe paths, broken output integrity, or I/O error |

Precedence is `4 > 3 > 1 > 0`; a mixed collection cannot become false green.

## Manifest

The strict TOML v1 shape is:

```toml
schema_version = 1

[collection]
id = "product-help"
title = "Product help"
description = "Verified workflows for the product."

[[topics]]
id = "getting-started"
title = "Getting started"
description = "First workflows for new users."
order = 10
audience = ["visitors"]

[[topics.guides]]
key = "continue"
order = 10
run = "runs/crawlson-run-example"
journey = "journeys/continue.toml"
```

Run and journey paths are portable paths relative to the manifest directory.
Absolute paths, leading/trailing separators, repeated separators, empty or
`.`/`..` components, backslashes, C0/C1 controls, escapes, and symlinked
components are rejected before any run is copied. Collection, topic, and guide
identifiers are bounded lowercase portable basenames. They begin with an ASCII
letter or digit, do not end in `.`, and reject the Windows device basenames
`con`, `prn`, `aux`, `nul`, `com1` through `com9`, and `lpt1` through `lpt9`,
including those names followed by an extension. Journey IDs retain their core
contract and may begin with `.`, `_`, or `-`; they are provenance values and
are never used directly as collection output basenames.

Topic order is unique across the collection; guide order is unique within a
topic. Topic IDs, guide keys, run IDs, and active journey IDs cannot be
ambiguous. A manifest contains at most 128 topics, each topic contains at most
1,024 guides, and the runtime additionally enforces a collection-wide maximum
of 1,024 guides. Draft 2020-12 JSON Schema cannot express a sum across nested
topic arrays, so the schema records that aggregate as `$comment` and the runtime
enforces it as a semantic manifest constraint.

The published schema is
[`guide-collection-manifest-v1.schema.json`](../../schemas/guide-collection-manifest-v1.schema.json).

## Verification boundary

Collection generation never trusts a run's existing `render/` directory. For
each entry it:

1. resolves the declared run and journey beneath the manifest workspace;
2. rejects symlinks and snapshots the raw run into a bounded temporary tree,
   excluding any prior `render/` output;
3. invokes the same offline renderer used by `crawlson render` against that
   snapshot and exact journey source;
4. rechecks every returned render output's size and digest; and
5. retains the verified bytes needed by the collection before discarding the
   snapshot.

Bounds are aggregate, not reset for each entry. One collection may inspect at
most 4,096 raw input files totaling 512 MiB and retain or generate at most
8,192 files totaling 768 MiB. Counts and bytes are checked before cloning,
retention, insertion, and staging so a many-entry manifest cannot multiply a
per-entry allowance into unbounded memory or disk use. Exceeding any aggregate
is an error and installs no output.

This preserves the single source of truth for action grants, postconditions,
cleanup, artifacts, focused-image sidecars, and journey provenance. The
collection adds no alternative notion of a passed step. It inherits the
single-run renderer's documented local concurrent-mutation/TOCTOU limit; v1
does not claim platform-specific open-beneath isolation.

## Honest output selection

Public output is all-or-nothing:

- If every current entry is `guide_ready`, Crawlson emits `index.md`, ordered
  topic indexes, navigable per-guide pages, focused PNGs copied byte-for-byte,
  and `guide-collection.json` as the neutral application adapter boundary.
- If any entry is `findings_ready`, Crawlson emits no root public index. It
  writes `review/index.md`, the renderer-produced structured findings and
  Markdown, and only the evidence files those findings reference.
- If any entry is blocked or otherwise not publishable, no public root index is
  emitted. `review/index.md` records every entry's honest state.
- Renderer errors, incomplete cleanup, unknown action effects, provenance
  conflicts, and tampered evidence are collected in deterministic manifest
  order. The resulting collection report has `status = error`,
  `publishable = false`, an empty `outputs` array, and one safe diagnostic per
  failed entry that could be identified. No public or review tree is installed.

Valid guide, finding, and blocked results are aggregated before status
precedence is applied. An error takes precedence over blocked and findings; a
blocked or otherwise unavailable entry takes precedence over findings; findings
take precedence over ready. A manifest/workspace failure that prevents safe
entry enumeration returns one collection-level diagnostic and no entries. A
failed entry never prevents already verified sibling states from appearing in
the machine-readable error report, but no partial bytes from that report are
installed as a collection.

A successful public tree has this deterministic shape:

```text
collection-report.json
guide-collection.json
index.md
topics/<topic>/index.md
topics/<topic>/<guide>/index.md
topics/<topic>/<guide>/001-focused.png
```

Guide pages include explicit topic, audience, previous, and next navigation.
Their instruction/evidence body is rendered from the same neutral ordered step
model recorded in `guide-collection.json`; that model is accepted only after
the single-run renderer returns `guide_ready`. A focused image remains the exact
PNG whose sidecar proves the vivid red target outline and translucent near-black
surrounding mask; the collection does not re-encode it.

A review tree is intentionally separate:

```text
collection-report.json
review/index.md
review/<topic>/<guide>/render/findings.json
review/<topic>/<guide>/render/findings.md
review/<topic>/<guide>/report.json
review/<topic>/<guide>/evidence/...
```

Applications should ingest a public tree only when the collection report says
`status = ready` and `publishable = true`. Review output can contain target
metadata, traces, and sensitive UI pixels; it is not public-release-safe by
default. Focused public screenshots can also contain sensitive UI pixels and
are not automatically redacted.

## Determinism and integrity audit

All generated files are first built in memory, written beneath a sibling
staging directory, and reread byte-for-byte. On supported macOS, Linux, and
Windows filesystems, a missing destination is installed by a same-filesystem,
create-only directory rename. The destination is checked again at commit: a
concurrently created or pre-existing destination is preserved and rejected,
never deleted or replaced. If the running platform/filesystem cannot provide
the required no-replace behavior, Crawlson fails closed before activation.
There is no copy-then-delete fallback. A failure leaves prior output unchanged,
and a byte-identical existing tree remains an idempotent no-op.

The report binds the manifest and snapshot digests, journey
ID/revision/source digest, run/report digest, renderer decision, and every
generated output digest. Output records for entry-owned guide pages, focused
images, findings, and evidence additionally carry their topic ID, entry key,
journey ID, and report digest; collection- and topic-level indexes remain bound
to the manifest/snapshot because they do not belong to one journey.
`guide-collection.json` repeats the manifest and snapshot digests. Every guide
records its topic identity/title/audience, page path/size/digest, run/report
digest, journey provenance, purpose, declared expected outcome, verification
scope, and image path/size/digest so an application does not need to infer
provenance from a pathname. It also carries the same ordered, renderer-verified
step model used to produce Markdown. Each step records its journey step ID,
number, title, authored instruction, alt text, focused-image record, and an
explicit claim. An `observed_next_action` claim means the area was observed but
the authored next action was not executed. An `executed_and_verified` claim
means Crawlson executed the declared action and verified its postcondition.
Applications can therefore render another presentation from the neutral JSON
without parsing Markdown or strengthening what the run proved.

Before installation and during `check`, Crawlson audits the complete managed
tree:

- every generated local Markdown and image link resolves within the tree;
- every Markdown page is reachable from `index.md` or `review/index.md`;
- every managed PNG is referenced;
- every expected file is present and byte-identical;
- unregistered files, symlinks, stale bytes, orphan images, dead links, and
  missing index entries receive stable diagnostic codes; and
- diagnostics contain only logical entry keys and portable output paths, never
  source absolute paths, target origins, sessions, or raw driver output.

The report and application adapter schemas are
[`guide-collection-report-v1.schema.json`](../../schemas/guide-collection-report-v1.schema.json)
and
[`guide-collection-v1.schema.json`](../../schemas/guide-collection-v1.schema.json).

## Out of scope

V1 does not add hosted publication, HTML or theme plugins, arbitrary manual
Markdown/assets, automatic run discovery, historical revision selection,
authentication, form controls, mutations, fixture orchestration, model-authored
explanations, severity assessment, remote link checking, screenshot redaction,
or destructive replacement/pruning. The JSON collection document and stable
Markdown tree are replaceable renderer boundaries for those later adapters.
