# Release and managed-install contract v1

Status: defined for Crawlson 0.6.0. The repository can exercise this contract
with non-publishing dry-run artifacts, but no public 0.6.0 release exists yet.

## Purpose and boundaries

Release v1 makes the exact bytes used for first-party installation and
self-update inspectable without granting CI authority to publish them. It has
three related but deliberately separate contracts:

1. A bundle manifest records the payload files inside one target archive.
2. A signed release inventory records the complete archive set and its raw
   updater payloads.
3. A separately signed update manifest authorizes only the raw `crawlson`
   executable that an already-managed Unix installation may replace itself
   with.

The release inventory is the distribution trust surface. The update manifest
is the narrower self-update trust surface. An archive is never treated as an
executable update payload, and the updater never installs the alias, demo, or
fixtures. This separation also prevents an otherwise valid demo or installer
change from silently expanding what self-update may replace.

This contract does not select a public license, reserve package names, create a
production key, publish a GitHub release, or publish to an external registry.
Those actions remain owner-gated.

## Supported targets and artifact names

Release v1 contains exactly these four native targets in canonical lexical
order:

| Rust target | Archive | Bundle asset |
| --- | --- | --- |
| `aarch64-apple-darwin` | `tar.gz` | `crawlson-vVERSION-aarch64-apple-darwin.tar.gz` |
| `x86_64-apple-darwin` | `tar.gz` | `crawlson-vVERSION-x86_64-apple-darwin.tar.gz` |
| `x86_64-pc-windows-msvc` | `zip` | `crawlson-vVERSION-x86_64-pc-windows-msvc.zip` |
| `x86_64-unknown-linux-gnu` | `tar.gz` | `crawlson-vVERSION-x86_64-unknown-linux-gnu.tar.gz` |

`VERSION` is the stable Cargo package version without prerelease or build
metadata. The archive has one root directory named
`crawlson-vVERSION-TARGET`. Windows executable names use `.exe`; the other
targets have no executable suffix.

Windows ARM64 and Linux ARM64 are not release-v1 targets. Adding a target is a
versioned release-contract change and requires native execution coverage, not
only a successful cross-compilation.

## Bundle payload

Every root contains this application-independent layout:

```text
crawlson-vVERSION-TARGET/
  crawlson-bundle.json
  README.md
  bin/
    crawlson[.exe]
    clson[.exe]
    crawlson-demo[.exe]
  examples/
    demo-pass.toml
    demo-fail.toml
    follow-link-pass.toml
    follow-link-fail.toml
  scripts/
    demo.sh
```

The three binaries are built for the archive target. The examples and script
are the credential-free, loopback-only demonstration; they contain no private
application fixtures or identifiers. The bundle-local script can find the
adjacent examples using the same relative layout used in a source checkout.

`crawlson-bundle.json` schema v1 contains the stable version, exact target, and
a canonical path-sorted list of every payload file other than the manifest
itself. Each entry binds its normalized relative path, nonzero byte size, and
lowercase SHA-256 digest. Absolute paths, `.` or `..` components, backslashes,
duplicates, symlinks, hard links, device entries, missing files, unlisted files,
and target/name mismatches are invalid.

Packaging the same staged bytes must produce the same archive bytes. Packaging
uses a canonical entry order, normalized timestamps, stable owner/group fields,
and fixed file modes: `0755` for `bin/*` and `scripts/demo.sh`, and
non-executable `0644` for manifests, the README, and examples. No checkout path, runner
path, credential, signing key, or build cache is included. This claim is about
deterministic packaging of an already-built stage; cross-machine bit-for-bit
compiler reproducibility must not be claimed without a separate rebuild proof.

## Signed release inventory

`crawlson-release.json` schema v1 is the canonical distribution inventory and
`crawlson-release.json.minisig` signs its exact bytes. It contains one
canonically ordered entry per supported target and binds:

- archive format, asset name, size, and SHA-256 digest;
- the archive's canonical bundle-file inventory; and
- the corresponding raw updater asset name, size, and SHA-256 digest.

The inventory must reject a missing or duplicate target, unexpected asset,
unsafe name, noncanonical order, unsupported schema, unstable version, empty or
oversized file, and malformed digest. Its raw updater size and digest must equal
the `bin/crawlson[.exe]` entry in the corresponding bundle inventory.

The bundle manifest provides internal consistency after extraction. It does not
authenticate itself. Before public installation, authenticity begins by
verifying the signed release inventory and using its archive digest. The dry
run exercises the same structure with a clearly identified test-only key, which
proves mechanics but provides no production authenticity.

## Signed updater manifest

`crawlson-update.json` schema v1 and
`crawlson-update.json.minisig` remain independent from the release inventory.
The manifest contains exactly one raw payload for each supported target:

```text
crawlson-update-vVERSION-TARGET[.exe]
```

It contains no archive, `clson`, demo, script, fixture, installer, receipt, or
release-inventory entry. Each raw payload published beside it is copied from the
already-staged `bin/crawlson[.exe]`; generating or rebuilding a second binary is
forbidden. Release verification compares the raw bytes, size, and SHA-256 with
the bundled member before either signed manifest is accepted.

The updater additionally requires a stable immutable release, exact version/tag
agreement, the fixed first-party release URL, GitHub's asset digest, the signed
size and digest, and an exact build-target match. Verification failure occurs
before replacement and cannot become a notice-only or green result.

## Managed bundle installation

The public first-party command is:

```text
crawlson install --from-bundle ROOT --prefix ABSOLUTE_BIN_DIR
```

`ROOT` is an extracted bundle root, and the command must be run by that
bundle's `bin/crawlson[.exe]`. `--prefix` names the absolute directory that will
contain the installed `crawlson[.exe]` and `clson[.exe]`; it is not a package
root and is never inferred from the current directory. The installer does not
elevate privileges, edit a shell profile, change `PATH`, or install or upgrade
`agent-browser`.

Before writing, installation validates the bundle manifest, target, version,
canonical file set, regular-file types, sizes, and digests. It rejects symlinks,
path escapes, a source executable outside `ROOT`, a relative prefix, a prefix
that resolves to the bundle itself, and any mismatched or incomplete pair.

Installation stages `crawlson` and `clson` in the destination filesystem, syncs
the staged bytes, and commits them with the schema-v1 standalone receipt
required by the updater. The receipt binds the exact build target, canonical
installed `crawlson` path, and a nonempty installation identifier.
`crawlson-demo`, the journeys, and `scripts/demo.sh` remain bundle-local.

The two binaries and receipt are one rollback unit for errors observed by the
installer. A failure while staging, validating, replacing, or writing the
receipt attempts to restore the prior managed pair and receipt and removes
installer-owned temporary files. Any install or rollback failure produces a
nonzero result; a failed rollback is reported explicitly and exact ownership
checks fail closed on the next operation.

Release v1 does not claim crash recovery between filesystem renames. An abrupt
process kill, operating-system crash, or power loss can leave an incomplete
pair; the receipt and exact-file ownership checks then fail closed instead of
guessing. A durable transaction journal and startup recovery are explicit
follow-up work.

## Upgrade behavior

Only an exact valid first-party receipt grants self-update ownership. Cargo,
Homebrew, Nix, unknown, malformed-receipt, wrong-target, and wrong-binary cases
fail closed with their existing reinstall or package-manager guidance.

On supported Unix targets, `crawlson upgrade` downloads the raw target payload,
verifies the signed manifest, size, and SHA-256, stages it in the installed
binary's directory, and performs same-directory atomic replacement. It does not
modify `clson`: the stable launcher continues forwarding to the sibling
canonical executable.

Direct Windows self-replacement remains disabled because rollback of the
running executable is not yet proven. Windows resolves automatic policy to
notify-only, persists a signed release notice on the success cadence, and never
downloads the raw updater payload. Manual `crawlson upgrade` also blocks before
payload download; the user authenticates and extracts the new bundle, then
reruns its `crawlson install --from-bundle ... --prefix ...` command against the
existing managed prefix.

## Packaged demo proof

The bundle proves more than binary startup. With a supported independent
`agent-browser 0.26.x` installation, run from the extracted bundle root:

```console
scripts/demo.sh \
  --crawlson-bin "$PWD/bin/crawlson" \
  --demo-bin "$PWD/bin/crawlson-demo" \
  --output-dir ./crawlson-demo-output
```

On Windows, use the corresponding `.exe` paths from a Bash environment. The two
binary overrides are an inseparable pair, must name executable files, and skip
the source `cargo build`. All existing target authorization and nonempty-output
guards remain in force.

The proof requires a passing read-only observation, an intentional visible
failure, a blocked missing-origin grant, a verified same-origin link action, an
intentional post-action mismatch, and a blocked missing-action grant. It
verifies the run reports, trace, raw screenshot, focused screenshot, focus
metadata, guide, and findings. The focused evidence must retain the vivid red
action outline and dimmed surrounding page; merely producing a PNG is not
sufficient.

## Non-publishing dry run

The release dry run builds, packages, installs, and exercises bundle HTTP
startup on all four native targets but cannot publish. The complete packaged
six-outcome real-browser demo remains a required release proof and is not yet a
cross-target dry-run gate. Its workflow and token use read-only repository
permissions. It has no release, tag, package, attestation, deployment, or
external-registry write step and receives no production signing secret.

Signatures in a dry run use an explicitly test-only key whose private material
is scoped to the bounded test job and carries no production authority.
Artifacts, manifests, logs, and names identify the test-key scope. The job
uploads short-lived workflow artifacts for review; it does not create release
assets. A production workflow must rebuild and sign after owner approval rather
than promote dry-run output.

Dry-run verification must fail on a missing target, alias mismatch, raw/bundled
payload mismatch, modified fixture, unsafe archive entry, bad digest or test
signature, incomplete receipt, wrong-target install, partial replacement,
rollback failure, or packaged-server startup failure. Passing the current dry
run proves that clean managed binaries and the packaged server work; it does not
yet prove the complete packaged browser demo or satisfy the public MVP install
criterion.

## Owner-gated production follow-up

Public release remains blocked until the owner deliberately:

1. selects and records the public license;
2. reserves the intended package and repository namespaces;
3. generates the production Minisign key and chooses its custody and recovery
   policy;
4. embeds and verifies the production public key;
5. approves an exact version, commit, target set, and digest inventory; and
6. authorizes creation of the immutable GitHub release and any external
   publication.

Production signing and publication belong in a separate protected workflow.
Ordinary CI success, pull-request auto-merge, a test-key signature, or a dry-run
artifact must never imply that authorization.
