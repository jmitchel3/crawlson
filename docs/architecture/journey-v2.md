# Read-only journey contract v2

Journey v2 preserves every v1 read-only action and safety rule. It advances the
document version so 0.2 readers continue to reject new fields rather than
silently misinterpreting them. Crawlson 0.3 accepts both v1 and v2; the run
report remains schema v1 because its evidence and outcome shape is unchanged.

V2 adds `evidence_for` to capture steps. Each value must uniquely identify an
earlier `check_url` or `check_text` step. This digest-verified relation lets an
offline renderer attach a later focused screenshot to a failed checkpoint
without guessing from timing, selector text, or titles. Non-capture steps and
self/future references are rejected.

V2 also bounds guide instructions and excludes query/fragment-bearing action
paths. Run reports intentionally redact URL query and fragment values, so a
query-sensitive URL checkpoint cannot produce an honest, reproducible finding.
V1 remains executable for compatibility, but the renderer declines to publish
such an ambiguous legacy URL journey.

See [`journey-v2.schema.json`](../../schemas/journey-v2.schema.json) and the
[`read-only example`](../../examples/read-only-journey.toml).
