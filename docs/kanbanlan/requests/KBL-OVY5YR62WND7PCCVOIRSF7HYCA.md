# Require CI and complete safe auto-merge governance

- Kanbanlan: `KBL-OVY5YR62WND7PCCVOIRSF7HYCA`
- Canonical home: `github`
- Canonical request: [#8](https://github.com/jmitchel3/crawlson/issues/8)

## Request

## Outcome

Make repository auto-merge meaningful and safe now that the stable CI check has passed on main.

## Acceptance criteria

- [x] Keep auto-merge and merged-branch deletion enabled.
- [x] Allow pull-request branch updates and use squash as the only merge method.
- [x] Require full-SHA GitHub Actions references while keeping workflow tokens read-only and unable to approve pull requests.
- [x] Add a versioned main ruleset requiring pull requests, the strict up-to-date CI check, resolved conversations, and linear history while preventing deletion and force-push.
- [x] Use zero approvals during solo maintenance to avoid deadlock; document when to raise it.
- [ ] Verify the ruleset and repository settings through GitHub APIs and record the result.

## Scope boundaries

Repository governance only. No source behavior, release publication, or broad auto-arming of untrusted pull requests.

## Decisions

- `CI` is required only after it passed on both pull-request and `main` events,
  avoiding an unseen-check deadlock.
- Main uses pull requests, strict up-to-date `CI`, resolved conversations,
  linear history, squash-only merging, and deletion/force-push prevention.
- Approval count remains zero for solo maintenance. Raise it to one when a
  second active maintainer can review without blocking urgent work.
- Auto-merge is armed per eligible pull request; untrusted public pull requests
  are never auto-armed merely because they exist.
- Workflow tokens remain read-only and cannot approve pull requests. Every
  referenced GitHub Action must use a full commit SHA.

## Verification

- The aggregate `CI` check passed on PR #7 and on merge commit
  `d2498c177b2ccd871400cadee6f8c58219e6d396` on `main` across Linux, macOS, and
  Windows before protection was enabled.
- Live repository settings and the active ruleset will be read back through the
  GitHub API after application.

## Delivered result

- Versioned the intended `main` ruleset and documented the safe auto-merge
  workflow.
- Applied repository merge, Actions, and ruleset settings after the required
  check had proven stable on `main`.
