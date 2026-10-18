# Local security build

This fork was reviewed against upstream version 0.24.6 at
`5892992dead8f9596c5291d87b0259d0b494f650`. The local build has version
`0.24.6-local.5` and bundle identifier `com.seichris.claude-desktop-switcher`.

The source review found no explicit runtime credential exfiltration, Keychain
access, telemetry or automatic updater. This is limited source evidence, not a
guarantee about every dependency or an operating-system security boundary.
Claude accounts still run as the same macOS user.

## Changes before installation

- Refuse duplication, deletion, purge and link repair while any Claude Desktop
  instance runs, including a secondary environment. Process-inspection errors
  propagate rather than being interpreted as no matching processes.
- Port process-output validation and Unicode locale handling from upstream
  [PR #225](https://github.com/matsumotory/claude-desktop-switcher/pull/225),
  commit `3e19abd7dbcf643afc8d02a87e268d8537dad80b`. Reject empty, invalid or
  partially unreadable output and use UTF-8 for process inspection. This fork
  applies those checks to PID/argument pairs as well, keeping its quit-all guard.
- Keep CSW storage and copied directories private with mode 0700.
- Reserve new profile directories exclusively. An unfinished directory may hold
  another account's sign-in data and is refused rather than adopted. Validate
  the sharing source first and publish metadata only after setup succeeds.
- Skip symbolic links and special files throughout recursive copying. Refuse
  symlinked profile roots and redirected stored profile identities/data paths.
- Use absolute paths for native launch/process-inspection tools; restrict URL
  opening to the project's GitHub paths and remove inline script permission.
- Direct the update button to this fork. Upstream downloads do not contain these
  local changes; inspect and rebuild future updates deliberately.
- Show saved Claude usage for discovered environments, without a fixed account
  list. Read only Desktop usage history and exact matching local usage-cache
  responses. Show supported usage percentages, saved reset times and the source
  observation time; mark stale and missing readings explicitly. Do not request
  credentials, sign in, make network requests or modify environment data.
  See the [usage guide](USER_GUIDE_EN.md#checking-saved-claude-usage) and
  [read scope](PRIVACY_EN.md#local-files-read-for-claude-usage).
- Add opt-in complete local Desktop Code sharing with `csw share-code`, using existing Desktop sign-ins and no Claude Code CLI login. It copies complete Code sidebar, worktree and preset records only while Claude is stopped, backs up every participant, and refuses independent conflicting changes. `csw unshare-code` disables future synchronization without removing current data or sign-ins. Participating environments cannot be deleted until sharing is disabled. See the [sharing guide](USER_GUIDE_EN.md#complete-local-code-sharing-in-this-fork).

## Build and use

With Xcode command-line tools and the pinned Rust toolchain installed:

```sh
sh scripts/build-local-app.sh
```

This uses the committed lockfile, embeds the local frontend and creates an
ad-hoc signed app in `target/release/bundle/macos`. It does not use an Apple
Development/Developer ID private key, notarize, install or launch the app.
The script also builds the matching CLI at `target/release/csw`. Install both
outputs before enabling Code sharing; an older Switcher app does not perform
synchronization. For a CLI already on your `PATH`, replace it with this local
binary, or invoke `./target/release/csw` from the checkout.

An ad-hoc signature verifies the local bundle's consistency; it is not publisher
identity or Apple notarization. Do not disable Gatekeeper globally.

Create a **New environment** using **Separate everything** for each account and
sign in through the official Claude app. Existing Claude remains the default.
Do not use Duplicate to add a different account: it intentionally copies Desktop
sign-in state. Quit every Claude Desktop instance before maintenance. External
Claude Code sessions can also write profile files; close those before maintenance.
These checks are not a transactional snapshot or protection against malicious
same-user filesystem changes racing an operation.

## Dependency check on 2026-10-15

This is the dependency snapshot from the initial hardening review, before the
local-usage addition. Its package counts do not describe later lockfile changes.

OSV's batch API was queried for all 454 registry package versions in Cargo.lock.
The macOS normal/build graph contained 263 distinct package entries. The glib
unsoundness advisory and proc-macro-error maintenance notice are outside that
graph. Five included Unicode crates have unmaintained notices:
unic-char-property, unic-char-range, unic-common, unic-ucd-ident and
unic-ucd-version. No vulnerability match beyond those maintenance notices was
returned for the macOS graph. This does not detect undisclosed vulnerabilities
or prove dependency code is benign. API: https://google.github.io/osv.dev/post-v1-querybatch/

Regression coverage includes orphaned sign-in trees, failed creation, top-level
and managed-path sockets, a running secondary environment, external and
dangling nested symlinks, both copy walkers, private directories, redirected
TOML paths and a symlinked profile root. Use:

```sh
cargo test --locked --workspace --exclude csw-desktop
cargo clippy --locked -p csw-core -p csw-cli --all-targets -- -D warnings
```
