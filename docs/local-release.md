# Release from your Mac

JustMux releases are built and published locally. npm distributes the prepared
Rust executables; it does not compile Rust in the cloud or on the user's
machine. GitHub Actions is an optional build and publication channel.

## Before releasing

Use an Apple Silicon Mac checkout on `main`, with the intended `origin` pointing to
`https://github.com/Quaternijkon/justmux.git`. Set up Rust, the Apple SDK, Zig,
`cargo-zigbuild`, Node.js 22.15 or newer, npm, Python 3, and tmux. Install the
two Rust targets listed in [the npm release guide](npm-release.md). Authenticate
Git access and run `npm login` with an account permitted to publish all three
JustMux packages.

The build produces a macOS ARM64 executable and a Linux x64 static musl
executable. Linux users still install tmux separately. Building both targets
requires a Mac because the Darwin build uses the Apple SDK.

## One command

For the next intended version, run:

```sh
./scripts/release 0.1.2
```

The wrapper is equivalent to `node scripts/release.mjs 0.1.2`.

`0.1.2` is the example next release after `0.1.1`. Replace it with the intended
new version; published npm versions cannot be overwritten.

The default requires a clean working tree. To explicitly include **all
nonignored modifications and untracked files** in the release commit, use:

```sh
./scripts/release 0.1.2 --commit
```

Review `git status` first: `--commit` is permission to include the complete
nonignored working tree, not just the release scripts.

To inspect the intended operations without changing files, commits, tags,
remote branches, or npm packages:

```sh
./scripts/release 0.1.2 --dry-run
```

The release sequence is:

1. Check the environment and release prerequisites.
2. Update the Cargo and npm versions together and create a release commit.
3. Run formatting, Clippy, Rust tests, the release build, and npm packaging
   tests. Build both supported platforms, validate and pack the three npm packages,
   and exercise actual installation and the installed command's terminal
   handoff.
4. Create an annotated version tag and atomically push `main` and that tag.
5. Publish the native packages first and the main `justmux` package last.
   Check registry availability and published package integrity.

The final push and npm publication are intentional effects of running the
release command. The dry run performs neither. npm's browser login or 2FA
challenge may still require your interaction; the script cannot approve it for
you. A successful dry run is a plan check, not evidence that a new release has
been built or published.

## Recovering an interrupted release

Continue an interrupted release with its original version:

```sh
./scripts/release 0.1.2 --resume
```

The command records state in `dist/releases/0.1.2/release-state.json` and
prepares packages under `dist/releases/0.1.2/npm`. After the release commit is
created, resume requires a clean working tree at that recorded commit and checks
the tag and prepared package hashes. Once the source has been pushed, it does not rebuild the
packages. Do not edit the source, rebuild different tarballs, or move the
version tag to make a failed publication fit an existing version.

If a commit hook or signing step rejected the initial release commit, correct
that hook or signing setup and run the same `--resume` command. The recorded
staged tree must remain unchanged. Resume also recognizes a matching commit
that completed immediately before the state file could be updated.

If publication stopped after some packages became public, resume checks the
published integrity and skips only packages whose contents match. A conflicting
package at the same version is an error; npm versions cannot be overwritten.
Keep the local release state and prepared packages until the release completes.

## GitHub and duplicate publication

Pushing a `v<version>` tag now triggers `.github/workflows/npm-release.yml` to
build and publish to npm automatically via GitHub Actions OIDC Trusted
Publishing. Choose one publisher for a release: either tag-triggered GitHub
publication or local publication (`./scripts/release` without pushing until
ready, or without tag-triggered CI). Running both can build different tarballs
for the same immutable npm version. npm's integrity checks will reject that
mismatch rather than replace the public package.

After publication, users install or update both command names with:

```sh
npm install --global justmux@latest
jm --version
justmux --version
```
