# npm installation and release

The npm distribution contains the Rust executable; it does not compile Rust on
the user's machine or download programs from an installation script. The main
`justmux` package exposes both `jm` and `justmux`. npm selects an exact-version
optional package for the operating system and CPU.

Users install or update with:

```sh
npm install --global justmux@latest
jm --version
justmux --version
```

Node.js 22.15 or newer and tmux are required. The launcher uses `process.execve`
to replace itself with the native program, keeping the terminal and process
handoff behavior of the Rust build. There is no persistent Node supervisor.
Windows is not supported by the native tmux application; use the Linux build
inside WSL instead. Installing with `--omit=optional` omits the required native
platform package and is not supported.

## Update prompts

Global npm installations check the public npm registry on interactive startup
without a command, including `--fullscreen`, at most once per 24 hours. Checks
have a three-second total timeout, and errors allow the native app to start.
The prompt is an inline TUI selector shown before the main Rust interface:

1. **Update** (initial selection): install the displayed exact version into
   the existing global npm prefix, then exit. Start `jm` again afterward.
2. **Skip**: continue this launch. The same version may prompt on the next
   interactive launch, using the daily cache without another request.
3. **Skip this version**: remember the dismissal across restarts and daily
   metadata refreshes. A different release can prompt again.

Up/Down (or j/k) moves between choices, Enter confirms, and 1/2/3 selects
directly. Esc and Ctrl+D skip; Ctrl+C cancels startup. The selector briefly uses
raw input and inverse highlighting in a small terminal region. It restores
input and the cursor before npm or the Rust interface starts, including on
SIGINT, SIGTERM, SIGHUP, and SIGQUIT. It does not use an alternate screen or
capture the mouse. A failed installation prints a manual retry command and
continues with the available installed program. The updater never invokes
`sudo` and never installs without an explicit selection.

`JUSTMUX_NO_UPDATE_CHECK=1` or `NO_UPDATE_CHECK=1` disables checks. Noninteractive
runs, CI, help, version, direct commands, local/npm-link/npx installs, and Cargo
builds do not perform them. The cache stores only version and check time (plus
the version explicitly skipped) under `$XDG_CACHE_HOME/justmux/update.json`, or the normal
macOS/Linux user cache directory. No tmux or terminal content is sent to npm.

Version `0.1.1` includes this feature. Existing official `0.1.0` installations
need one manual npm update to receive the updater; they cannot notify users
through code they do not yet contain.

## Published packages

Current releases target macOS ARM64 and Linux x64 only, using three packages
with the same version:

| Package | Rust target | Executable |
| --- | --- | --- |
| `justmux` | JavaScript launcher | `jm` and `justmux` |
| `justmux-darwin-arm64` | `aarch64-apple-darwin` | `bin/jm` |
| `justmux-linux-x64` | `x86_64-unknown-linux-musl` | `bin/jm` |

The Linux target uses a static musl executable so the package does not depend
on the user's glibc version. Native tmux still needs to be installed separately.
The source launcher manifest is private; the preparation command creates the
publishable manifest with the exact native-package dependencies.

## Local preparation and publication

The recommended end-to-end entry point is `./scripts/release VERSION`. It
coordinates versioning, checks, builds, GitHub source/tag push, npm publication,
and verification. See [the local release workflow](local-release.md) for preview,
explicit inclusion of uncommitted changes, and resuming failures. The commands
below remain available for building or inspecting packages individually.

Local builds are the primary release path. Keep the Cargo and npm source
versions equal, then prepare all supported platforms:

```sh
node scripts/npm-release.mjs build --all
node scripts/npm-packages.mjs validate --dir dist/npm
node scripts/npm-packages.mjs pack --dir dist/npm
node scripts/npm-packages.mjs publish --dir dist/npm --dry-run
node scripts/test-npm-install.mjs --pty
```

On an Apple Silicon Mac, building both targets requires the Apple SDK for the
Darwin target, Zig and `cargo-zigbuild` for the Linux cross-build, and both Rust
targets. The script checks these prerequisites before building; it never
installs tools implicitly. Linux x64 hosts can build the Linux target, but a
Mac is required for the Darwin build.

The build command reports missing toolchains or cross-build prerequisites; do
not substitute one architecture's executable for another. Prepared package
directories are under `dist/npm`, and packed tarballs and `SHA256SUMS` are
under `dist/npm/tarballs`. Inspect the files and run the installation smoke
test before publication. The installation test uses an isolated temporary npm
prefix and loopback registry, then checks tmux attach and popup switching through
the installed launcher. On Linux, pass `--binary` with the static musl build path.
A dry run performs no npm publication.

Publication needs an npm account with permission to create or update all three
package names. The account must satisfy npm's current authentication and 2FA
requirements. Authenticate interactively on the publishing machine:

```sh
npm login --registry=https://registry.npmjs.org
npm whoami --registry=https://registry.npmjs.org
node scripts/npm-packages.mjs publish --dir dist/npm
```

The publication command validates the complete distribution and its checksums,
publishes the platform packages first, and publishes the main package last.
The main package must never reference a platform version that is not yet
available. npm versions are immutable: a changed build needs a new version.
On a retry, only an already-published package whose bytes match the prepared
tarball may be skipped; a different tarball at that version is an error.

For the next release, update all source version fields together before
rebuilding:

```sh
node scripts/npm-release.mjs version 0.1.2
node scripts/npm-release.mjs build --all
```

The version command updates `Cargo.toml`, `Cargo.lock`, and the npm manifest;
it requires a higher version and does not create or push a Git tag. A prerelease
version uses npm's `next` distribution tag; stable versions use `latest`.
Publication refuses to move either tag to an older version. Partial or
interrupted local builds are marked incomplete and cannot be packed or published;
rerun the full build successfully to prepare a release.

npm may accept a publication before its processing has finished. The script waits
for every native package's metadata and tarball to become publicly downloadable
with the expected integrity before publishing the main package, then verifies the
main package too. If this wait times out, continue checking without republishing:

```sh
node scripts/npm-packages.mjs verify-registry --dir dist/npm
```

The historical `0.1.0` release was built locally and published to npm on
2026-11-17 with five packages covering four platforms. A fresh anonymous
installation on macOS ARM64 verified both
command names and actual tmux attach, detach, and two-client popup switching.
The Intel macOS and Linux artifacts were cross-built and inspected; their runtime
behavior still needs testing on those platforms. No GitHub Actions run was used
for this publication. Release `0.1.1` is restricted to macOS ARM64 and Linux x64
and adds the three-choice startup update picker. It was built and published
locally on 2026-11-18; all three public tarballs matched the prepared integrity
values. A real public-registry upgrade from a local preview of `0.1.0` to
`0.1.1` passed on macOS ARM64. No GitHub build or Git push was used. The
historical packages for other architectures are unchanged.

Neither preparing nor packing logs in, creates an npm account, changes a shell
configuration, pushes Git commits, or publishes a package. Package names must
remain available until the first publication; a successful availability check
does not reserve them.

## GitHub Actions build and automated OIDC release

`.github/workflows/npm-release.yml` runs automatically when a `v*` version tag
is pushed, and can also be triggered manually via `workflow_dispatch`. Manual
runs default to `publish=false`, which builds and tests on two native runners
and uploads a complete distribution artifact without npm credentials.

The runner matrix uses macOS 15 ARM64 and Ubuntu 24.04 x64. Each runner executes
the Rust tests and actual PTY smoke
check using an isolated tmux socket before preparing its package. Linux builds
also reject a dynamic ELF interpreter. The build logs record the OS, Rust,
Node.js, npm, and tmux versions. Runner availability and quotas are governed by
the GitHub repository/account; the local release path does not require them.

For example, after pushing the workflow to the repository, a manual build-only
run is:

```sh
gh workflow run npm-release.yml --repo Quaternijkon/justmux -f publish=false
```

This command is an explicit external action, not part of local preparation.
The checked-in terminal demo is not overwritten.

## Trusted npm publication from GitHub

Configure an npm trusted publisher on each of the three current packages. Use
these exact values:

| npm trusted publisher setting | Value |
| --- | --- |
| Provider | GitHub Actions |
| Organization or user | `Quaternijkon` |
| Repository | `justmux` |
| Workflow filename | `npm-release.yml` |
| Environment | `npm` |

Create the repository's `npm` environment and configure required reviewers if
manual approval is desired. Trusted publishing uses GitHub OIDC and requires
no long-lived npm token in the repository. A new package cannot rely on its
per-package trusted-publisher configuration before that package exists, so
bootstrap the first version with a permitted local npm account.

Pushing an annotated `v<Cargo version>` tag automatically verifies the matching
version, builds both platforms, and publishes with npm provenance through OIDC.
You can also manually trigger a publish on an existing tag:

```sh
gh workflow run npm-release.yml --repo Quaternijkon/justmux --ref v0.1.0 -f publish=true
```

It builds and verifies the distribution first and uses npm provenance for the
publication. Local publication remains available as an alternative, but use only
one publication channel per release version.
