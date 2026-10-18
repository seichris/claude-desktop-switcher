# Privacy and Transparency

This document lists everything Claude Desktop Switcher (CSW) reads, writes, and never touches on your Mac, together with the steps to verify each promise yourself. A bare "it is safe" claim gives you nothing to check, so we publish the facts and the verification steps side by side.

This document is kept in sync with the implementation. Whenever the implementation changes, the same change updates this document, and a pre-release consistency check looks for any gap between the two.

## The four promises

1. **No internet communication.** CSW sends no usage data and performs no auto-update requests.
2. **No reading of passwords or sign-in data.** CSW never touches the macOS Keychain that stores your passwords, and never opens the contents of sign-in tokens or browser cookies. When you duplicate an environment, sign-in files that the duplicate copies, such as the Claude Desktop App's cookies, are copied as they are, like any other file, without being opened. See "About duplicating an environment" below for details.
3. **Limited, documented local reads.** Sharing and copying touch only the items you choose. The usage view additionally reads the Claude Desktop App's saved usage history and exact matching usage-cache responses. It does not read conversations or sign-in data to obtain usage, and never changes the existing Claude data. The unshipped automatic-quit candidate can additionally record the installed Claude app identity and process launch time. Its quit and Accessibility-dialog operations are disabled by the readiness gate described below.
4. **Zero impact by default.** CSW rewrites no shell configuration or OS-level environment variables. Existing Claude is modified only when you explicitly enable complete local Code sharing, which replaces its Code stores while excluding sign-in storage. Launching without CSW uses the stores already present and does not synchronize them.

## About network access

CSW does not connect to the internet and has no built-in facility to do so.

The single exception is the links at the bottom of the window (User guide, Report an issue, Check for updates), the "See how to verify" button in the About screen, and the "Open releases page" button that shows where to get the csw command. Pressing one hands a predetermined GitHub URL to macOS, which opens it in your default browser. The browser communicates; CSW itself does not. "User guide" and "See how to verify" open the Japanese or English version of their document, matching the display language. "Open releases page" appears in the first-run guide, the terminal section of the detail screen and the isolation check results, and opens the same releases page as "Check for updates". The URLs are fixed to the following six, and the implementation additionally rejects any URL that is not `https`.

- `https://github.com/seichris/claude-desktop-switcher/blob/main/docs/USER_GUIDE.md`
- `https://github.com/seichris/claude-desktop-switcher/blob/main/docs/USER_GUIDE_EN.md`
- `https://github.com/seichris/claude-desktop-switcher/issues`
- `https://github.com/seichris/claude-desktop-switcher/releases`
- `https://github.com/seichris/claude-desktop-switcher/blob/main/docs/PRIVACY.md`
- `https://github.com/seichris/claude-desktop-switcher/blob/main/docs/PRIVACY_EN.md`

Update checks work the same way: CSW never queries for new versions; it simply opens the GitHub releases page in your browser.

## Where CSW writes

By default, CSW writes inside its own folder, `~/.context-switcher-claude/`. Deleting an environment moves its folder to the macOS Trash. Explicitly enabling complete local Code sharing additionally permits replacing the three Code stores described below in participating Desktop environments, including Existing Claude. Sign-in storage is outside this operation.

| Path | Contents |
|---|---|
| `~/.context-switcher-claude/config.toml` | The name of the currently selected environment |
| `~/.context-switcher-claude/session.toml` | The environments that were open together when you last quit them all |
| `~/.context-switcher-claude/automatic-switch/safe-launch.json` and `.proof-*` | The unshipped candidate's private launch record and temporary files: process ID, launch time, managed data directory and reviewed app identity; insufficient to authorize quitting |
| `~/.context-switcher-claude/native-code.toml` and `native-code.lock` and `.native-code.toml-*` | An explicitly enabled local Code sharing group, change fingerprints and an operation lock |
| `~/.context-switcher-claude/native-code-journal.json` and `.csw-write-*` | Transaction recovery metadata and private temporary configuration files |
| `~/.context-switcher-claude/native-code-launch-pending` | A launch reservation that blocks synchronization until Claude startup is observed or explicitly cleared |
| `~/.context-switcher-claude/native-code-backups/` | Private snapshots before changed synchronizations, their locations manifest and the preceding sharing configuration |
| `~/.context-switcher-claude/profiles/<name>/profile.toml` | That environment's settings (name, note, creation record, and sharing mode) |
| `~/.context-switcher-claude/profiles/<name>/state.toml` | When CSW first and last launched that environment |
| `~/.context-switcher-claude/profiles/<name>/desktop-data/` | That environment's data for the Claude Desktop App |
| `~/.context-switcher-claude/profiles/<name>/cli-data/` | That environment's data for Claude Code |

In addition, the settings window stores three records for its display in the app's own screen data location managed by macOS: the accent color you chose, whether the first-run guide was shown, and whether you closed the sign-in card that the detail screen shows after an environment's first launch. The sign-in card record is kept per environment, so it includes the environment's name and creation time to tell environments apart. This location holds only these records, never the contents of an environment's folder or Claude's data.

## Where CSW reads

- CSW reads and writes its own folder listed above. When the detail screen shows the data breakdown, it reads only file names and their sizes and dates inside this folder, never file contents. The targets of shared links are never included in the totals.
- CSW touches the existing Claude folders, `~/Library/Application Support/Claude` and `~/.claude`, in the following cases:
  1. **Existence and emptiness checks**: it checks whether a folder or file exists. When you open the create screen, it also checks whether each of the two existing Claude folders is empty; that check stops at the first entry it finds and does not use the entry's name. File contents are never read.
  2. **Creating and re-pointing shared links**: for items you set to Shared when creating or duplicating an environment, it creates symbolic links inside the new environment that point to the existing Claude's originals. The isolation check's repair command, `csw doctor --fix`, likewise only re-points share links that no longer point at their declared source, or creates a share link that is missing while its source exists. Only the link is swapped or created; real files are never touched. The only exception is that an empty folder is removed. If the existing Claude had no source folder yet when you created the environment, CSW creates an empty folder on the environment's side. Once the source exists, `csw doctor --fix` removes this empty folder and then creates the share link. Removing a folder fails if it contains anything, so CSW can remove only empty folders without reading what is inside. The links and the empty folders it removes are all in the environment's folder; nothing is written on the existing Claude's side.
  3. **Copying**: for items you set to Copy when creating or duplicating an environment, it reads those files and copies them into the new environment. Only the items you chose are read, and the existing Claude's side is never modified.
  4. **Saved usage**: the usage view reads the Desktop files listed below. It does not read Claude Code's folder for usage.
  5. **Explicit complete local Code sharing**: the Code stores described below are read, backed up and replaced in every participant, including Existing Claude. Sign-in storage is excluded.
- To tell which environment a running Claude is using, CSW reads the list of running processes and their launch arguments. The unshipped candidate can also check a newly launched target process's launch time and app identity for its private launch record, as described below. It does not read process memory, environment variables or communication contents.
- The identifier of the frontmost application is read only when you press "Check the current environment" in the menu bar icon's menu, to answer which environment the Claude in front is using. Only the process identifier and its launch arguments are read; screen contents and window titles are never read. CSW never polls for it.
- To check whether a CSW installer disk image is still mounted, CSW reads the list of mounted disk images with `hdiutil` when it starts and when you press "Eject". Only each disk image's file path and mount points are read. CSW picks out its own disk images by name and ignores all others, and it never reads the contents of any disk image.

### Local files read for Claude usage

Usage reads are confined to each discovered environment's Desktop data root: `~/Library/Application Support/Claude` for Existing Claude, or `~/.context-switcher-claude/profiles/<name>/desktop-data` for a CSW environment. No account names, email addresses, or fixed account list are built into this feature.

| Path below the Desktop data root | What the usage reader uses |
|---|---|
| `plan-usage-history.json` | Saved usage percentages, observation times, and the latest observation's organization identifier, when the history version includes it |
| `Cache/Cache_Data` | Chromium Simple cache entry names, bounded key and format metadata, and the body of an exact matching saved usage response |

The only response URLs accepted are `https://claude.ai/api/organizations/<org>/usage` and that exact URL with `?skip_spend=1`. The organization identifier comes from that environment's latest history, not from sign-in files. These URLs are used only to match local cache keys; CSW never requests them. Other cache bodies and authentication headers are not parsed. A history version without an organization identifier can supply percentages, but is not used to guess a cached organization.

The reader keeps each environment separate even when two accounts belong to the same organization. It validates supported file formats and integrity fields, limits file enumeration, reads and decompression, and refuses symbolic links, hard links, and non-regular input files. Unsupported or unsafe input gives a history-only reading or unavailable data; it never triggers a credential or network fallback. Nothing is repaired, copied, or written by the usage reader.

Only the environment name, usage percentages, times, source, and display state reach the screen. Raw cache contents, response URLs, and organization identifiers are not returned to the screen, logged, or saved by CSW. Usage reports are held in memory rather than written as another cache.

The last-checked time is the saved observation's time. A reading over 30 minutes old or past a displayed reset is marked stale. Missing values remain unavailable, and a passed reset never changes a saved percentage to zero. A cache can survive a change of sign-in, so a reading does not prove which account is signed in now. The official Claude app remains the place to verify current usage and account identity.

## Complete local Code sharing

This fork provides the opt-in operation `csw share-code --source default --replace <environment>...` for environments that already share local transcripts, project memory and input history. The permanent Code stores it reads and replaces below their Desktop data roots are `claude-code-sessions/<account>/<workspace>/`, `git-worktrees.json`, and `ccd-session-presets.json`. During replacement it also creates sibling `.csw-stage-*` and `.csw-previous-*` directories, plus `.git-worktrees.json-*` and `.ccd-session-presets.json-*` temporary files, all containing only these Code stores. After an interruption, a private journal and the selected source backup allow the intended copy to finish. Every overwritten original remains in its backup. It copies complete records, including saved Code permissions, connector references, scheduled-task definitions, deletion markers and worktree bookkeeping. It maps the Code presets' account key to each destination. It never opens account configuration, cookies, browser storage or the Keychain for this operation. There are no network requests or Claude Code CLI sign-ins.

All Claude Desktop instances must be closed. Each participant must have one unambiguous account/workspace directory. Every store is backed up before any replacement. Symbolic links, hard links, special files, invalid JSON, oversized inputs and independent changes in more than one environment are refused. The normal Switcher launch flow carries forward the complete store changed since the preceding synchronization. It does not inject data into an already running sidebar. Launching outside CSW does not run this synchronization.

This shares local Code content and saved Code behavior, including worktree cleanup state and scheduled tasks. It does not share cloud Projects, cloud Chat history, subscriptions, connector sign-ins or Web-side custom groups. Use one participating environment at a time. The initial source and overwritten destinations remain in local backups; backups are not silently pruned.

Quit all Claude Desktop instances and run `csw unshare-code` to disable subsequent synchronization. Existing local data, backups and sign-ins remain; disabling does not restore older stores or unlink the separately shared transcript, memory and input-history files. Delete or permanently remove a participant only after disabling the group.

The launch reservation remains if startup is not observed. After quitting every Claude Desktop instance, `csw unshare-code` clears that reservation and disables synchronization. Reconfigure with an explicitly chosen source to resume. If a copy was interrupted, its journal completes the recorded intended copy before disabling; original stores remain in backups.

## Disabled automatic-quit prototype

The automatic-quit prototype is blocked by security review, has not been merged or installed, and is not an available feature. This candidate rejects automatic quitting with `automatic_guard_unverified` before requesting a native quit or inspecting or pressing native quit-dialog controls. No Accessibility grant is needed for current use. Let current work and input waits finish and quit Claude manually before changing environments. The installed local build remains `0.24.6-local.5`.

Claude 2.42304.0 installs its working-session quit guard asynchronously. A launch record, process age, the finished-launching flag and a verified vendor build do not establish that the guard is ready for that exact process. Allowing Accessibility does not solve this missing proof. The unconditional refusal stays in place until a reliable readiness check is verified. See the [development proposal](proposals/automatic-account-switch.md).

The candidate's launch-record code can check the installed bundle at `/Applications/Claude.app`: the version in `Contents/Info.plist`, its vendor code signature and the SHA-256 hash of `Contents/Resources/app.asar`. It also records a newly launched process's ID, launch time and managed data directory after explicitly disabling Claude's special background-launch mode. This record is stored at `automatic-switch/safe-launch.json` in CSW's own folder, using mode 0600 in a mode-0700 directory, with `.proof-*` temporary files. It contains no credentials and is insufficient to authorize quitting.

The dormant native adapter uses `NSRunningApplication` for a single normal quit request and limits its Accessibility traversal to the target Claude's native modal controls, with only the exact known **"Wait for Claude"** action allowed. It does not scan the main conversation WebView, other apps, authentication settings, browser storage or process environment variables. It contains no force-quit or termination-signal path. These boundaries describe implemented code that is blocked from quit execution, not proof that automatic quitting is safe to enable.

macOS grants Accessibility permission to the app as a whole; it does not enforce a per-dialog scope. The narrower traversal above is a code restriction. The explicit permission control can request the OS prompt, but the permission cannot bypass the readiness gate. No new credential reads, network requests or CLI sign-ins are added.

## About duplicating an environment

Duplicate, in the environment's detail screen, copies the contents of the selected environment's folder, as they are, into a new environment folder on your Mac. The only things it leaves out are the always-isolated items (the account sign-in info in config.json, the connector and app settings, the device ID and the session state), the plugins, whose install records point at the original environment's location, and state.toml, where CSW records launch times. Symbolic links are not copied as they are, and shared items are linked again to the existing Claude. Even then, all CSW does is copy the files verbatim; it never interprets their contents and never sends them anywhere. The existing Claude cannot be duplicated.

The Claude Desktop App also keeps its sign-in state in cookies inside the environment's folder. Those cookies are copied as they are, like any other file, so the duplicate's Claude Desktop App opens signed in to the same account as the original. The original and the duplicate carry the same sign-in state, so logging out in either one also logs out the other. For an environment that uses a different account, use "New environment" instead of duplicating. Claude Code in the terminal, on the other hand, stores its sign-in in the macOS Keychain, outside the environment's folder, separately for each environment. CSW does not touch the Keychain, so this sign-in is not copied. Log in again the first time you use Claude Code in a terminal for the duplicate. The one exception: only when Claude Code cannot write to the Keychain does it place a `.credentials.json` file in the environment's folder. That file is copied like any other file, so in that case the duplicate's Claude Code uses the same credentials as the original.

## OS commands CSW runs

CSW runs only the following standard macOS commands. All of them are local operations and none of them communicate.

| Command | Purpose |
|---|---|
| `open` | Launches Claude with the selected environment's data folder, opens the fixed links in your default browser, and opens an environment's data folder in Finder |
| `pgrep` | Checks whether the Claude Desktop App is running |
| `ps` | Reads the launch arguments of running Claude processes to learn which environment's data folder they use |
| `hdiutil` | Detects a CSW installer disk image that is still mounted and ejects it |
| `codesign` | The unshipped candidate verifies the installed Claude app's vendor signature for its launch record; this does not authorize quitting |
| `PlistBuddy` | The unshipped candidate reads only the installed Claude app's version from its `Info.plist` for the launch-record check |

## What CSW never touches

- The macOS Keychain that stores your passwords. There is simply no code that reads or writes it.
- Sign-in tokens and cookies of your browser or Claude. The Claude Desktop App's sign-in state lives inside that environment's data folder, and CSW never opens and interprets those files' contents. When duplicating an environment they are copied verbatim, nothing more. Claude Code stores its sign-in in the macOS Keychain. CSW does not touch the Keychain, so it neither reads that sign-in nor copies it when duplicating. Only when Claude Code cannot write to the Keychain does it place a `.credentials.json` file in the environment's folder, and duplicating copies that file verbatim like any other file.
- Any other app's data beyond the cases listed under "Where CSW reads".
- The network. Nothing is sent and nothing is received.

## Verify it yourself

### Verify that CSW does not communicate

Start CSW, open the settings window, and run the following in a terminal.

```bash
lsof -i -a -p $(pgrep -f Claude-Desktop-Switcher | head -1)
```

This lists the internet connections held by the CSW process. An empty output means there are none.

One note for full accuracy: macOS renders the settings window through separate WebKit processes. CSW's screens load only local files bundled with the app, so those processes make no external connections either. The command above inspects the app's own process only, so if you want to audit the whole machine, watch for CSW-related connections with `nettop` or a dedicated network monitor.

### Verify the download is genuine

Notarization is Apple's review that confirms an app's identity. You can confirm that the CSW you downloaded passed this review with the following commands:

```bash
codesign -dv --verbose=2 "/Applications/Claude-Desktop-Switcher.app"
spctl --assess --type execute -v "/Applications/Claude-Desktop-Switcher.app"
```

If `spctl` reports `accepted` with `Notarized Developer ID`, the artifact passed Apple's notarization.

### Verify the isolation and links

You can inspect how an environment's data is actually wired, in Finder or a terminal:

```bash
ls -la ~/.context-switcher-claude/profiles/<name>/cli-data/
```

Items you set to Shared appear as symbolic links pointing at the existing Claude's originals. Isolated items appear as real files owned by that environment alone.

### Verify that no dependency can communicate

The no-network promise is not just written down; it is enforced mechanically at the dependency level. The repository's `deny.toml` registers HTTP and WebSocket client libraries as banned, and CI fails the build if any of them enters the dependency graph. This check runs automatically on every pull request.

One fact worth stating up front: the dependency record file `Cargo.lock` does contain entries for the HTTP libraries `reqwest` and `hyper`. They are declared by Tauri, the UI foundation of CSW, as dependencies for Windows and Linux, and they are not part of the macOS build. You can confirm this from the source with the commands below.

```bash
cargo tree --target aarch64-apple-darwin -i reqwest
cargo tree --target aarch64-apple-darwin -i hyper
```

If a command prints `warning: nothing to print.`, that library is not part of the macOS dependency graph.

### Verify the bill of materials (SBOM)

Every release from v0.19.0 onward ships with a CycloneDX-format list of all libraries contained in the artifacts. Download files such as `csw-desktop_aarch64-apple-darwin.cdx.json` from the GitHub releases page to see every component and version in the macOS build.

## About this document

CSW is open source and its entire source code is public. If anything in this document disagrees with the implementation, that is a bug: please tell us on [GitHub Issues](https://github.com/seichris/claude-desktop-switcher/issues).
