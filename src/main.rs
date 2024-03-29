use crate::ReleaseType::{Major, Minor, Patch};
use anyhow::{anyhow, bail, Context as _, Error, Result as ARes};
use clap::{crate_name, crate_version, App, Arg};
use fehler::throws;
use regex::{Captures, Regex};
use semver::{Identifier, Version, VersionReq};
use std::env::set_current_dir;
use std::fs::File;
use std::io::{Read, Write};
use std::process::{Command, Output};

#[throws]
fn main() {
    let matches = App::new(crate_name!())
        .version(crate_version!())
        .about("Opinionated automated release actions for Rust projects.")
        .args(&[
            Arg::with_name("patch")
                .short("p")
                .long("patch")
                .help("Release is a patch (x.y.Z). Default: new minor version."),
            Arg::with_name("major")
                .short("M")
                .long("major")
                .help("Release is a new major version (X.y.z). Default: new minor version.")
                .conflicts_with("patch"),
            Arg::with_name("path")
                .short("r")
                .long("repo")
                .takes_value(true)
                .help("Path to the git repository to use. Default: current directory."),
            Arg::with_name("commit")
                .short("b")
                .long("branch")
                .takes_value(true)
                .help("Start from this branch or commit. Default: no checkout."),
            Arg::with_name("base")
                .short("f")
                .long("for")
                .takes_value(true)
                .help("Use this version as the base (X or X.Y). Default: latest.")
                .conflicts_with("major"),
            Arg::with_name("install")
                .short("i")
                .long("install")
                .help("Install the new version locally."),
            Arg::with_name("no-push")
                .short("n")
                .long("no-push")
                .help("Do not perform a final push to the remote."),
        ])
        .after_help(
            "\
        This program performs the following actions:\n\
        + In --repo, by default the current directory.\n\
        + If --branch is specified, checkout the commit.\n\
        + Check if repo is clean and up to date: `git status`, `git rev-list`.\n\
        + Retrieve the latest semver tag from git, possibly coerced by --for.\n\
        + Increase the semver. Defaults to minor, use --patch or --major as needed.\n\
        + Edit Cargo.toml, replacing `version`.\n\
        + Run the cargo commands: `update`, `clippy -D warnings`, `fmt`.\n\
        + Commit and create a new semver tag for the version.\n\
        + If --install, run `cargo install`.\n\
        + If a semver tag for the next minor does not already exist:\n\
        ++ Edit Cargo.toml, replacing `version` with the next minor with '-dev' prerelease.\n\
        ++ Run `cargo update` again.\n\
        ++ Commit.\n\
        + Unless --no-push, push the new HEAD, then push the new tag.\n\
        \n\
        WARNING: Cargo.toml is naively edited using regexps. Most importantly, the first\n\
        occurrence of `^version = ..$` must belong to [package]. See the v1 for safe parsing,\n\
        which sadly came with too many caveats.\n\
        ",
        )
        .get_matches();
    let release = if matches.is_present("patch") {
        Patch
    } else if matches.is_present("major") {
        Major
    } else {
        Minor
    };
    if let Some(path) = matches.value_of("path") {
        set_current_dir(path)?;
    }
    let branch = matches.value_of("commit");
    let constraint = {
        if let Some(base) = matches.value_of("base") {
            if !Regex::new(r"\d+(\.\d+)?")?.is_match(base) {
                bail!("--for: invalid format, should be `X` or `X.Y`.")
            }
            if !matches.is_present("patch") && Regex::new(r"\d+\.\d+")?.is_match(base) {
                bail!("--for: when specifying a minor version (x.Y), `patch` is mandatory.")
            }
            VersionReq::parse(&format!("~{}.0", base))?
        } else {
            VersionReq::any()
        }
    };
    let no_push = matches.is_present("no-push");

    if let Some(branch) = branch {
        Command::new("git")
            .args(&["checkout", branch])
            .output_success()
            .context(format!("Failed to checkout branch {}", branch))?;
    }
    let install = matches.is_present("install");

    Command::new("git")
        .args(&["status", "--porcelain=v2"])
        .empty_stdout()
        .context("`git status` not empty; repo not clean")?;

    if !no_push {
        Command::new("git")
            .arg("fetch")
            .output_success()
            .context("Failed to fetch upstream")?;

        Command::new("git")
            .args(&["rev-list", "HEAD..HEAD@{upstream}"])
            .empty_stdout()
            .context("`git rev-list` not empty; repo behind upstream")?;
    }

    let out = Command::new("git")
        .args(&["tag", "--list"])
        .output_success()?;
    let stdout = String::from_utf8(out.stdout)?.trim().to_owned();
    let mut semver_tags = vec![];
    let semver_tag_re = Regex::new(r"^v\d+.\d+.\d+$")?;
    for line in stdout.lines() {
        if !semver_tag_re.is_match(line) {
            continue;
        }
        semver_tags.push(Version::parse(&line[1..])?);
    }
    let semver_tags = semver_tags;
    let latest = {
        if let Some(v) = semver_tags.iter().filter(|v| constraint.matches(v)).max() {
            v.clone()
        } else {
            bail!(
                "No matching semver tag found for constraint {}.",
                constraint
            )
        }
    };

    let mut new_version = latest;
    match release {
        Major => new_version.increment_major(),
        Minor => new_version.increment_minor(),
        Patch => new_version.increment_patch(),
    };
    let new_version = new_version;

    if semver_tags.contains(&new_version) {
        bail!(
            "Attempting to release a version that already exists: {}",
            new_version
        );
    }

    let next_exists = {
        let mut next = new_version.clone();
        next.increment_minor();
        semver_tags.contains(&next)
    };

    update_cargo_toml_version(&new_version)?;

    Command::new("cargo").arg("update").output_success()?;

    Command::new("cargo")
        .args(&["clippy", "--", "-D", "warnings"])
        .output_success()?;

    Command::new("cargo").arg("fmt").output_success()?;

    Command::new("git")
        .args(&[
            "commit",
            "-am",
            &format!("Release version {}.", new_version),
        ])
        .output_success()?;

    Command::new("git")
        .args(&["tag", &format!("v{}", new_version)])
        .output_success()?;

    if install {
        Command::new("cargo")
            .args(&["install", "--path", "."])
            .output_success()?;
    }

    if !next_exists {
        let mut post_version = new_version.clone();
        post_version.increment_minor();
        post_version.pre = vec![Identifier::AlphaNumeric("dev".to_owned())];
        let post_version = post_version;

        update_cargo_toml_version(&post_version)?;

        Command::new("cargo").arg("update").output_success()?;

        Command::new("git")
            .args(&["commit", "-am", "Post-release."])
            .output_success()?;
    }

    if !no_push {
        Command::new("git").arg("push").output_success()?;

        Command::new("git")
            .args(&["push", "origin", &format!("v{}", new_version)])
            .output_success()?;
    }
}

type AVoid = ARes<()>;

trait CommandPropagate {
    fn output_success(&mut self) -> ARes<Output>;
    fn empty_stdout(&mut self) -> AVoid;
}

impl CommandPropagate for Command {
    fn output_success(&mut self) -> ARes<Output> {
        let output = self.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8(output.stderr)?.trim().to_owned();
            bail!(stderr);
        }
        Ok(output)
    }

    fn empty_stdout(&mut self) -> AVoid {
        let output = self.output_success()?;
        if !output.stdout.is_empty() {
            let stdout = String::from_utf8(output.stdout)?.trim().to_owned();
            bail!(anyhow!(stdout).context("Command stdout should be empty"));
        }
        Ok(())
    }
}

#[derive(Eq, PartialEq)]
enum ReleaseType {
    Major,
    Minor,
    Patch,
}

#[throws]
fn update_cargo_toml_version(version: &Version) {
    let mut manifest = String::new();
    File::open("Cargo.toml")?.read_to_string(&mut manifest)?;
    let re = Regex::new(r#"(?m)^(version\s*=\s*")[^"]*("\s*)$"#)?;
    if !re.is_match(&manifest) {
        bail!("Could extract version from Cargo.toml, see --help for more info.");
    }
    let manifest = re.replace(&manifest, |c: &Captures| {
        format!("{}{}{}", &c[1], version, &c[2])
    });
    File::create("Cargo.toml")?.write_all(manifest.as_bytes())?;
}
