# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and the
signer (`nightly-sign.yml`, release) keep only their exact context guards and run when one of the reviewed
manual actors dispatches them. There is no schedule yet. The installed policy
[`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3
policy. Once `nightly.yml` reaches `main`, populate `producer_workflow_id` with its registration ID
before promoting the policy to `release`:

```bash
gh api repos/storytold/vectorcraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `605d58766e8f536564da8fc81d3676ac86816dea`, the craft-common-actions
main commit that merged [#339](https://github.com/storytold/craft-common-actions/pull/339) (bounded
lineage compares and an issue reread at the end of candidate admission) on top of
[#342](https://github.com/storytold/craft-common-actions/pull/342) (policy v3, the `platforms` input
and the Linux and macOS legs). Both callers name it in `uses` and `tooling-revision`. A pin is reviewed
source availability, not provider, queue, native signing or crypto qualification.
Existing stable workflows are unchanged.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on main (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `release` with that
producer run ID and attempt, by a listed manual actor.

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the release-managers
team (id 19837241), leaving out `echelon-robot`; update it by PR in both places when the team changes.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or
skipped platform stops that night's release, Windows included), and the release-ref signer admits
platforms only from the protected policy, so there is no separate Linux or macOS caller job and
the signer caller has no platform input.

## Installed policy values and their sources

Read on 2026-11-24; re-read anything that may have moved (and populate `producer_workflow_id` after `nightly.yml` lands on `main`) before the policy is promoted to `release`.

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1398097510` | `gh api repos/storytold/vectorcraft --jq .id` |
| `producer_workflow_id` | `null` (pending first registration on `main`) | Read via `gh api repos/storytold/vectorcraft/actions/workflows/nightly.yml --jq .id` once `nightly.yml` is on `main` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `605d5876…` (craft-common-actions #339) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members who can push; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows | Learning Machines stable identities (Windows Authenticode publisher subject, Linux OpenPGP fingerprint, Apple Developer ID team `DJ6XS33FX8`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `380135806`), 8 jobs | Workflow list API; jobs that succeeded on recent main push runs |
| `candidate_policy.sonar` | `storytold` / `storytold_vectorcraft`, app `12526` | `sonar-project.properties`; the SonarCloud check on main commits |

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `VectorCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level release registration, enabled for manual dispatch,
  and identical on main and release after reviewed promotion. It rejects wrong repository/event/actual branch,
  including same-name tags. Its constant whole-run concurrency (`vectorcraft-nightly-release`) is distinct
  from stable, cancellation is off and `queue: max` must be hosted-qualified. The caller itself passes no
  credentials; only the shared callee's sign jobs bind the `release` environment.
- Shared reusable entrypoints are `nightly-unsigned.yml` and `nightly-release.yml` in
  `storytold/craft-common-actions`. Both must resolve to a **real reviewed public immutable
  commit**; the defining workflow SHA and tooling input must match.

## Local regression checks

`python3 packaging/nightly-release-tests.py` verifies the nightly workflow and policy source contracts:
- Exact context guards on `nightly.yml` and `nightly-sign.yml` (including multi-line continuation checks).
- Manual actors match the reviewed release-managers list in both `nightly.yml` and `.github/nightly-signing-policy.json`, with `automatic_signer_actors` empty.
- Both callers pin the same immutable `craft-common-actions` commit in `uses` and `tooling-revision`.
- No nightly schedule is registered, no cache actions or secrets appear in the app callers, and `nightly-sign.yml` retains its literal root `concurrency` block (`group: vectorcraft-nightly-release`, `cancel-in-progress: false`, `queue: max`).
- Policy v3 schema, platform rows, bounds, required main-push CI workflow, and SonarCloud project identity match `sonar-project.properties`.

## The app side of a Nightly build

The producer and signer call into this repository:

| Platform | Compile step (no secrets) | Trusted signing step (release checkout) | Check |
|---|---|---|---|
| Windows x86_64 | `pwsh packaging/windows/package.ps1 -Arch x64 -Nightly -BuildOnly` | `packaging/windows/sign.ps1 -Required` | `nightly.ps1`'s PE, ProductName and version checks |
| macOS aarch64 | `bash packaging/macos/nightly.sh --arch aarch64` | `packaging/macos/nightly-sign.sh` (with `import-cert.sh`, `entitlements.plist`, `nightly.sh`) | `packaging/macos/nightly-verify.sh --expect signed --team` |
| Linux x86_64 | `bash packaging/linux/nightly.sh --arch x86_64` | the shared Linux signer | the shared `nightly_unix.py` isolation checks |

The three platform-specific scripts share `packaging/nightly-common.sh` (input checks, the owned
compile, the notice copier, the exclusive publish) and `packaging/nightly_paths.py` (the private
build root every helper stays inside). `packaging/nightly-tests.py` runs the synthetic suites
(`nightly-archive-tests.py`, `macos/nightly_plist_tests.py`, `macos/nightly_sign_tests.py`,
`nightly-release-tests.py`); `.github/workflows/nightly-check.yml` runs them with the Windows
PowerShell suites, and `.github/workflows/nightly-macos.yml` builds and verifies the unsigned
bundle behind the `full-ci` label.

The build switch is `VECTORCRAFT_BUILD_CHANNEL=nightly-v1`, which `apps/vectorcraft/build.rs` turns
into `cfg(vectorcraft_nightly_v1)`; the identity is `VECTORCRAFT_VERSION`
(`X.Y.Z-nightly.YYYYMMDD.<run>.<attempt>.g<sha12>`), also accepted from
`VECTORCRAFT_OVERRIDE_VERSION`, which is the name this repository's packaging scripts already pass
to cargo. A nightly build without that identity refuses to build.

The executable answers `--version`, `-V` and the Launcher's `--launch-profile nightly-v1
--profile-root <abs> --version` before root validation, logging, IPC or any window, with exactly
`VectorCraft <version> (nightly-v1)`. A nightly without a profile is refused (exit 2) with the
message to open VectorCraft Nightly from ArtCraft Launcher; a stable build refuses the profile.

All user state of a nightly launch lives in the profile root the Launcher passes: `config.toml`,
`logs`, `Data Recovery`, `Plug-ins`, `Fonts`, the user's libraries and script data, the
single-instance record, the eframe window state (`app.ron`) and the framework's `window.json`.
`apps/vectorcraft/src/profile_root.rs` fences it off stable's folders, a portable copy and the
installation on every platform, and `apps/vectorcraft/src/profile.rs` (`install_data_root`) is the
one hook every data path follows.
