rslease 1.1.0
Opinionated automated release actions for Rust projects.

USAGE:
    rslease.exe [FLAGS] [OPTIONS]

FLAGS:
    -h, --help       Prints help information
    -i, --install    Install the new version locally.
    -M, --major      Release is a new major version (X.y.z). Default: new minor version.
    -p, --patch      Release is a patch (x.y.Z). Default: new minor version.
    -V, --version    Prints version information

OPTIONS:
    -f, --for <base>         Use this version as the base (X or X.Y). Default: latest.
    -b, --branch <commit>    Start from this branch or commit. Default: no checkout.
    -r, --repo <path>        Path to the git repository to use. Default: current directory.

This program performs the following actions:
+ In --repo, by default the current directory.
+ If --branch is specified, checkout the commit.
+ Check if repo is fully clean (`git status` returns nothing).
+ Retrieve the latest semver tag from git, possibly coerced by --for.
+ Increase the semver. Defaults to minor, use --patch or --major as needed.
+ Edit Cargo.toml, replacing `version`.
+ Run the cargo commands: `update`, `build`, `clean`, `clippy`, `fmt`.
+ Commit and create a new semver tag for the version.
+ If --install, run `cargo install`.
+ Unless --patch was specified, perform the 3 following steps:
++ Edit Cargo.toml, replacing `version` with the next minor with '-dev' prerelease.
++ Run `cargo update` again.
++ Commit.
+ Push the new HEAD, then push the new tag.
