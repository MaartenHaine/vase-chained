# GitHub settings for vase-chained

The fork's hardening (wayfinder Q1.4, M0a) is half in the repository and half in GitHub's settings. The repository
half is committed: the inherited release workflows are deleted, the workflow rules are in
[CONTRIBUTING.md](../../CONTRIBUTING.md#github-actions-workflows), and `.github/dependabot.yml` asks for weekly
updates of the action pins and the Cargo workspace. This page is the other half: the settings the owner clicks on
`github.com/MaartenHaine/vase-chained`. Nothing here can be set from a commit.

The repository half takes effect on GitHub only once it is on `main`, the default branch. Dependabot reads
`.github/dependabot.yml` only from there, and `main`'s `release*.yaml` stay live until they are deleted there. So
first land the repository half on `main` (a pull request from `linux-tiler`, or a cherry-pick), and check:

```sh
git fetch origin && git ls-tree --name-only origin/main .github/ .github/workflows/
#   .github/dependabot.yml, and no .github/workflows/release*.yaml
```

Change no setting below until that check passes. Then do them in this order. Each section ends with a read-only
`gh api` call that shows the setting took.

## 1. Dependabot (Settings > Advanced Security)

In **Settings**, open **Advanced Security** in the sidebar ("Security and quality"). Under "Dependabot":

| Setting                     | Set to     | Why                                                                                                                                 |
| --------------------------- | ---------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Dependency graph            | **Enable** | Alerts are computed from it. It may already be on.                                                                                  |
| Dependabot alerts           | **Enable** | An advisory against a crate in `Cargo.lock` or a pinned action shows up as an alert.                                                |
| Dependabot security updates | **Enable** | Dependabot opens a pull request that fixes an alert. It is reviewed and merged like any other pull request.                         |
| Grouped security updates    | Optional   | One pull request per ecosystem instead of one per alert.                                                                            |
| Dependabot version updates  | **Enable** | On a fork, `.github/dependabot.yml` does nothing until this is enabled: GitHub doesn't turn version updates on for forks by itself. |

Check:

```sh
gh api repos/MaartenHaine/vase-chained/vulnerability-alerts --include | head -1     # HTTP/2.0 204: alerts on
gh api repos/MaartenHaine/vase-chained/automated-security-fixes                     # "enabled": true
gh api repos/MaartenHaine/vase-chained/contents/.github/dependabot.yml --jq .path   # on main: .github/dependabot.yml
```

Dependabot's own pull requests then show under **Insights > Dependency graph > Dependabot**. Until the first workflow
lands in `main`'s `.github/workflows/`, the `github-actions` entry there has nothing to update and may report that it
found no files. That is expected and goes away with the Linux CI job.

## 2. Actions (Settings > Actions > General)

| Setting                                                            | Set to                                                                                             |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| Actions permissions                                                | **Allow MaartenHaine, and select non-MaartenHaine, actions and reusable workflows**                |
| ↳ Allow actions created by GitHub                                  | **Off**. It would allow every `actions/*` and `github/*` action, `actions/github-script` included. |
| ↳ Allow Marketplace actions by verified creators                   | **Off**                                                                                            |
| ↳ Allow or block specified actions and reusable workflows          | Exactly the actions the workflows use, one `owner/repo@*` line each (see below).                   |
| Require actions to be pinned to a full-length commit SHA           | **On**. A workflow that names an action by tag or branch fails before it runs.                     |
| Approval for running fork pull request workflows from contributors | **Require approval for all external contributors**                                                 |
| Workflow permissions                                               | **Read repository contents and packages permissions** (the read-only default for `GITHUB_TOKEN`)   |
| Allow GitHub Actions to create and approve pull requests           | **Off**                                                                                            |

The allow-list and the SHA rule work together: the list says which actions may run, the SHA rule says they run only
at a pinned commit, and Dependabot moves the pins. `@*` in the list allows any commit of that action, so a Dependabot
bump doesn't need a settings change.

Today no workflow exists, so the list starts empty. The pull request that adds the first workflow (the M0a Linux CI
job) adds its actions to the list in the same sitting, for example:

```text
actions/checkout@*
```

Any later action is added to the list only after its source at the pinned SHA has been read, and the pull request
that starts using it says so.

GitHub doesn't run a fork's workflows until its Actions tab is opened once and the workflows are enabled there. Do
that now, after the settings above are in place and before the Linux CI job's pull request is opened, so that the job
runs on its own pull request. It is safe because no branch on GitHub has a workflow file yet: `main` passed the check
at the top of this page, and every other pushed branch must pass this one:

```sh
git fetch origin && for b in $(git for-each-ref --format='%(refname:short)' refs/remotes/origin); do
  git ls-tree -r --name-only "$b" .github/workflows/; done
#   prints nothing
```

Check:

```sh
gh api repos/MaartenHaine/vase-chained/actions/permissions
#   "enabled": true, "allowed_actions": "selected", "sha_pinning_required": true
gh api repos/MaartenHaine/vase-chained/actions/permissions/selected-actions
#   "github_owned_allowed": false, "verified_allowed": false, "patterns_allowed": [...]
gh api repos/MaartenHaine/vase-chained/actions/permissions/workflow
#   "default_workflow_permissions": "read", "can_approve_pull_request_reviews": false
gh api repos/MaartenHaine/vase-chained/actions/permissions/fork-pr-contributor-approval
#   "approval_policy": "all_external_contributors"
```

## 3. Protect `main` (Settings > Rules > Rulesets)

Chained builds the fork only at a reviewed commit that descends from `main`, so `main` must never be rewritten or
deleted. This is a one-person repository: nobody else can approve a pull request, so the ruleset requires a pull
request with **zero** approvals. It enforces the route (a branch, a pull request, a diff to read), not a second
reviewer.

In **Settings**, open **Rules > Rulesets**, then **New ruleset > New branch ruleset**:

| Field                                          | Set to                                                                                                            |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| Ruleset name                                   | `main`                                                                                                            |
| Enforcement status                             | **Active**                                                                                                        |
| Bypass list                                    | **Empty**. No bypass, not even for repository admins.                                                             |
| Target branches                                | **Add a target > Include default branch** (the default branch is `main`)                                          |
| Restrict deletions                             | **On**                                                                                                            |
| Block force pushes                             | **On**                                                                                                            |
| Require a pull request before merging          | **On**, with **Required approvals: 0** and **Allowed merge methods: Merge** only. The other sub-options stay off. |
| Require status checks to pass                  | **Off for now.** See below.                                                                                       |
| Require signed commits                         | **Off.** Signing was dropped on 2026-09-24; Chained checks the checkout's integrity instead.                      |
| Require linear history                         | **Off.** The monthly upstream merges from CasperTeirlinck/vase land as merge commits.                             |
| Restrict creations, Restrict updates           | Off                                                                                                               |
| Require deployments, code scanning, and others | Off                                                                                                               |

**Merge commits only.** "Allowed merge methods" is a list, not an on/off switch, and it allows merge, squash and
rebase by default. A squash or rebase of an upstream-sync pull request would put new commits on `main` instead of
upstream's own, so upstream's commits would stop being ancestors of `main` and the next monthly merge would re-conflict
on everything since the fork point. Merge only keeps them.

**Status checks.** A required check that never reports blocks every merge, so add it only after the Linux CI job has
run once on a pull request: its own, since the workflows are enabled before it is opened (section 2). Then edit the
ruleset: turn on **Require status checks to pass**, add that job's checks by name, and leave "Require branches to be
up to date before merging" off. Each later CI job is added the same way.

**If the ruleset is ever in the way** (a broken required check, say), set its enforcement status to **Disabled**,
do the one thing, and set it back to **Active**. Don't add a bypass entry: a bypass stays forever, a disabled
ruleset is visible on the Rulesets page.

Optional: a second branch ruleset for `linux-tiler`, with only **Restrict deletions** and **Block force pushes**, so
the Linux Tiler's long-lived branch can't be rewritten either while agent branches (`lt/*`) still merge into it
freely.

Check:

```sh
gh api repos/MaartenHaine/vase-chained/rulesets
gh api repos/MaartenHaine/vase-chained/rules/branches/main
#   "deletion", "non_fast_forward" and "pull_request" (required_approving_review_count 0,
#   allowed_merge_methods ["merge"]); no "required_signatures"
```

Classic branch protection (**Settings > Branches > Add classic branch protection rule** for `main`) gives the same
result if rulesets aren't wanted: "Require a pull request before merging" with 0 approvals, "Allow force pushes" and
"Allow deletions" both unticked, "Require signed commits" unticked, and "Do not allow bypassing the above settings"
ticked. Classic protection has no merge-method option, so also untick "Allow squash merging" and "Allow rebase
merging" under **Settings > General > Pull Requests**. Use the ruleset or classic protection, not both.

## What these settings don't cover

- **Releases.** The fork publishes none: the upstream release workflows are deleted, and so are the Homebrew
  templates, `install.sh` and the `release`, `bump-version-*` and `changelog-latest` recipes that only served upstream's
  releases. Chained builds macOS from source (`just app`). The Windows build later comes
  from a new, attested CI job, which gets its own entry in Chained's security audit before it ships, and its own
  `id-token: write` and `attestations: write` on that one job only.
- **Secrets.** No workflow needs one. Don't add repository secrets; a future job that needs one must not run
  pull-request code.
- **Self-hosted runners.** Never: one on the desktop would run pull-request code in the owner's session
  (`devtest-security.md` §3.5).
