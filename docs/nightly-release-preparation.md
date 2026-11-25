# Windows, Linux and macOS release-ref nightly: supervised manual go-live

**Enabled for supervised manual runs, not yet qualified.** The producer (`nightly.yml`, main) and
the signer (`nightly-sign.yml`, on the dedicated `nightly-signer` branch) keep only their exact
context guards and run when one of the reviewed manual actors dispatches them.
`nightly-dispatch.yml` stays literally off: it only forwards scheduled producers. There is no
schedule. The installed policy
[`.github/nightly-signing-policy.json`](../.github/nightly-signing-policy.json) is the v3 policy.
Its `producer_workflow_id` is the registration of `nightly.yml` on main, read with:

```bash
gh api repos/storytold/vectorcraft/actions/workflows/nightly.yml --jq .id
```

The callers pass no platform secrets. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to `fc7124a1defa96813aee060031dc8b4d2afa3652`, the craft-common-actions
main commit that merged [#370](https://github.com/storytold/craft-common-actions/pull/370) (the
Windows and macOS nightly registry rows for the 14 apps) after
[#369](https://github.com/storytold/craft-common-actions/pull/369) (shared helper checkouts kept out
of the app's clean-source check, and macOS on `macos-15`), following
[#364](https://github.com/storytold/craft-common-actions/pull/364) (the signer may run on the
policy's signer branch) and [#363](https://github.com/storytold/craft-common-actions/pull/363) (the
multi-app pipeline, which makes the signer's `app-name` input required and admits any reviewed
registry app), with #342 (`a4f6fd51`, policy v3 and the `platforms` input) and #339 (`605d5876`)
earlier. Both callers name it in `uses` and `tooling-revision`. A pin is reviewed source
availability, not provider, queue, native signing or crypto qualification. Existing stable workflows are unchanged.

**First runs.** The first runs are manual dispatches in this order: `nightly.yml` on main (only a
listed manual actor passes the producer's plan step), then `nightly-sign.yml` on `nightly-signer`
with that producer run ID and attempt, by a listed manual actor. `nightly-dispatch.yml` only matters
once a schedule is approved.

**Signer branch.** The signer runs on `refs/heads/nightly-signer`, not on the stable `release`
branch, so nightly signer files never need a stable release sync. The policy's `release_branch`
names it, the signer's guard requires it and the dispatcher targets it. The branch only
fast-forwards to reviewed main commits, through the settings script's `advance` (no queued signer
run, a trust-closure scan, then the full diff of `.github/` and `packaging/`), and the existing
`release` environment trusts it alongside `release`. Its trust closure is all of `.github/` and
`packaging/`, including `packaging/env.sh`, which `packaging/macos/import-cert.sh` sources with
the Apple keys. On that branch only the pinned `nightly-sign.yml` caller may reach an environment:
no app workflow binds one, nothing else calls `nightly-release.yml`, and the only caller of the
shared stable `release.yml` is `release.yml`, whose callee must carry the stable-only environment
guard. The contract tests check the in-repo part on every main PR; the settings scan also resolves
the callee. Stable releases still come only from `refs/heads/release`. History does not carry over
a switch between `release` and `nightly-signer`, so the switch happens before the first
publication.

**Registry admission.** The signer requires the protected policy's `platforms` map to equal the
run app's admitted targets in craft-common-actions' `nightly-apps.json`, and it refuses rather
than sign a subset. VectorCraft's row admits `linux-x86_64`; the matching `windows-x86_64` and
`macos-aarch64` targets and entrypoints are a craft-common-actions follow-up, and the first
signer run stops at admission until they merge.

**Manual actors.** `bflatastic`, `dylanmtaylor`, `echelon`, `FredApps` and `skylartaylor`, in the
producer's `manual-actors` and the policy's `manual_actors`. The list mirrors the release-managers
team (id 19837241), leaving out `echelon-robot`; update it by PR in both places when the team
changes.

**Platform rows.** The producer caller lists
`platforms: '["windows-x86_64","linux-x86_64","macos-aarch64"]'` on its single job: one producer
run builds every listed platform for one all-or-nothing release (one failed or
skipped platform stops that night's release, Windows included), and the signer admits
platforms only from the protected policy, so there is no separate Linux or macOS caller job and
the signer caller has no platform input.

## Installed policy values and their sources

Read on 2026-11-24; re-read anything that may have moved before `nightly-signer` is advanced to it.

| Field | Value | Source |
|---|---|---|
| `repository_id` | `1398097510` | `gh api repos/storytold/vectorcraft --jq .id` |
| `producer_workflow_id` | `430275079` | `gh api repos/storytold/vectorcraft/actions/workflows/nightly.yml --jq .id` |
| `producer_tooling_revision`, `signer_tooling_revision`, `producer_workflow_ref` | `fc7124a1…` (craft-common-actions #370 after #369 and #364) | The callers' pin |
| `manual_actors` / `automatic_signer_actors` | the five release managers / `[]` | release-managers team (id 19837241) members who can push; equals the producer caller's `manual-actors` |
| `max_age_seconds`, `max_artifact_bytes` | `86400`, `2147483648` | Go-live checklist |
| `platforms` | three rows | Learning Machines stable identities (Windows Authenticode publisher subject, Linux OpenPGP fingerprint, Apple Developer ID team `DJ6XS33FX8`) |
| `candidate_policy.required_workflows` | CI (`ci.yml`, workflow `380135806`), 8 jobs | Workflow list API; jobs that succeeded on recent main push runs |
| `candidate_policy.sonar` | `storytold` / `storytold_vectorcraft`, app `12526` | `sonar-project.properties`; the SonarCloud check on main commits |

## Ownership and interface

- `.github/workflows/nightly.yml`: default-main registration of the unsigned producer, named
  `VectorCraft unsigned nightly`, enabled for manual dispatch by `manual-actors`; no signing secrets
  may be passed or inherited.
- `.github/workflows/nightly-sign.yml`: top-level registration on `nightly-signer`, enabled for manual dispatch,
  and only ever fast-forwards to reviewed main commits. It rejects wrong repository/event/actual branch,
  including same-name tags. Its constant whole-run concurrency (`vectorcraft-nightly-release`) is distinct
  from stable, cancellation is off and `queue: max` must be hosted-qualified. The caller itself passes no
  credentials; only the shared callee's sign jobs bind the `release` environment, which trusts `release` and
  `nightly-signer`.
- `.github/workflows/nightly-dispatch.yml`: transport only, never authorization. It is literally
  off (`false &&`) and forwards only a first-attempt scheduled producer to `nightly-sign.yml` on
  `nightly-signer`. No schedule is registered until activation is reviewed.
- Shared reusable entrypoints are `nightly-unsigned.yml` and `nightly-release.yml` in
  `storytold/craft-common-actions`. Both must resolve to a **real reviewed public immutable
  commit**; the defining workflow SHA and tooling input must match.

## Local regression checks

`python3 packaging/nightly-release-tests.py` verifies the nightly workflow and policy source contracts:
- Exact context guards on `nightly.yml` and `nightly-sign.yml` (including multi-line continuation checks).
- Manual actors match the reviewed release-managers list in both `nightly.yml` and `.github/nightly-signing-policy.json`, with `automatic_signer_actors` empty.
- Both callers pin the same immutable `craft-common-actions` commit in `uses` and `tooling-revision`.
- No nightly schedule is registered, no cache actions or secrets appear in the app callers, and `nightly-sign.yml` retains its literal root `concurrency` block (`group: vectorcraft-nightly-release`, `cancel-in-progress: false`, `queue: max`).
- Policy v3 schema, platform rows, bounds, required main-push CI workflow, and SonarCloud project identity match `sonar-project.properties`, and the Sonar gate shape keeps every live condition once with accepted failures inside the reviewed issue scope.
- The signer guards `refs/heads/nightly-signer`, passes `app-name: vectorcraft`, and the policy's
  `release_branch` names it.
- A trust-closure scan reads every workflow: nothing binds an `environment`, only `nightly-sign.yml`
  calls `nightly-release.yml` (at a full SHA), and the only caller of the shared stable `release.yml`
  is `release.yml`.
- `nightly-dispatch.yml` keeps a literal-off guard, a `workflow_run`-only trigger and a
  `refs/heads/nightly-signer` request.

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
(`nightly-archive-tests.py`, `linux/nightly-launch-tests.py`, `macos/nightly_plist_tests.py`,
`macos/nightly_sign_tests.py`, `nightly-release-tests.py`); `.github/workflows/nightly-check.yml`
runs them with the Windows PowerShell suites, and `.github/workflows/nightly-macos.yml` builds and
verifies the unsigned bundle behind the `full-ci` label.

Every packaged Linux Nightly format starts the app the same way, through one launcher
(`packaging/linux/nightly-launch.sh`): the deb/rpm's `/usr/bin/vectorcraft-nightly`, the AppImage's
`AppRun`, the Arch handoff's `bin/vectorcraft` and the Flatpak's `/app/bin/vectorcraft`. It runs the
GUI beside it (`<app>-bin`, the standard Nightly layout;
`/usr/lib/vectorcraft-nightly/vectorcraft-bin` for deb/rpm, `/app/libexec/vectorcraft-bin` inside the
Flatpak) with `--launch-profile nightly-v1 --profile-root <dir> -- "$@"`, and passes a version query,
help or the Launcher's own `--launch-profile` invocation straight through. The plain tar's
`vectorcraft` is craft-common-actions' shared template instead
(`packaging/linux/nightly-tar-launch.sh`, #363 rendered for this app): it adds its own tokens unless the first argument is already a `--launch-profile` pair,
execs `vectorcraft-bin` beside it with
`--profile-root <${XDG_DATA_HOME:-~/.local/share}/ai.storyteller.vectorcraft.nightly> --`, and asks a
bare `--version`/`-V` in the Launcher's form. The
Nightly tar therefore holds `vectorcraft` (launcher), `vectorcraft-bin` (GUI), `vectorcraft.png`
(the Nightly art) and the notices.

The build switch is `VECTORCRAFT_BUILD_CHANNEL=nightly-v1`, which `apps/vectorcraft/build.rs` turns
into `cfg(vectorcraft_nightly_v1)`; the identity is `VECTORCRAFT_VERSION`
(`X.Y.Z-nightly.YYYYMMDD.<run>.<attempt>.g<sha12>`), also accepted from
`VECTORCRAFT_OVERRIDE_VERSION`, which is the name this repository's packaging scripts already pass
to cargo. A nightly build without that identity refuses to build.

The executable answers `--version`, `-V` and the Launcher's `--launch-profile nightly-v1
--profile-root <abs> --version` before root validation, logging, IPC or any window, with exactly
`VectorCraft <version> (nightly-v1)`. A nightly without a profile is refused (exit 2) with the
message to open VectorCraft Nightly from ArtCraft Launcher; a stable build refuses the profile. That
refusal goes to standard error, which a Launcher launch records: a release build has no console, so
a Nightly started outside the Launcher exits 2 without a dialog. A native message box would need a
new dependency (the workspace denies `unsafe_code`, and a cross-platform dialog crate would change
`Cargo.lock`); the Launcher is the supported way in.

A nightly does not update itself: Help › Check for Updates says that ArtCraft Launcher manages
the Nightly's updates, and the startup check, the pending install and the updater itself are not
created (`updates_ui::Runtime` keeps no updater unless `os_window::stable_services_allowed()`). The
craft-update records, downloads and logs otherwise live in the stable app's folders, which the
profile fence refuses.

All user state of a nightly launch lives in the profile root the Launcher passes: `config.toml`,
`logs`, `Data Recovery`, `Plug-ins`, `Fonts`, the user's libraries and script data, the
single-instance record, the session record and the framework's `window.json` (the window's size
and position). eframe's own persistence is off in this build (its `persistence` feature is not
enabled, so no `app.ron` is written anywhere); if it is ever turned on, its state goes to
`<profile>/eframe`, a folder of its own, never to the `window.json` file.
`apps/vectorcraft/src/profile_root.rs` fences it off stable's folders, a portable copy and the
installation on every platform, and `apps/vectorcraft/src/profile.rs` (`install_data_root`) is the
one hook every data path follows.

## The Sonar issue scope

The pinned candidate validator requires every entry in `candidate_policy.sonar.issue_types` to map
from a reviewed `accepted_conditions` row, and it requires the gate condition set to equal
`accepted_conditions` plus `required_conditions`. The installed policy lists `VULNERABILITY` with
`new_security_rating` accepted (max 5) and the other five gate conditions required - the same shape
PhotoCraft and WordCraft ship - so it satisfies that contract before SonarCloud is read. `debt` stays
empty, so any unresolved vulnerability on `main` is still refused, and the live gate is still compared
condition by condition against the policy. `producer_workflow_id` is now populated and the signer runs
on `nightly-signer`; the registry's Windows and macOS targets are what still stand between this tree
and a first signed nightly.
