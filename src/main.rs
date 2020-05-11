use crate::ReleaseType::{Major, Minor, Patch};
use anyhow::{anyhow, bail, Context as _, Error, Result as ARes};
use clap::{crate_name, crate_version, App, Arg};
use fehler::throws;
use regex::{Regex, Captures};
use semver::{Identifier, Version, VersionReq};
use std::env::set_current_dir;
use std::process::{Command, Output};
use std::fs::File;
use std::io::{Read, Write};

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
        ])
        .after_help(
            "\
        This program performs the following actions:\n\
        + In --repo, by default the current directory.\n\
        + If --branch is specified, checkout the commit.\n\
        + Check if repo is fully clean (`git status` returns nothing).\n\
        + Retrieve the latest semver tag from git, possibly coerced by --for.\n\
        + Increase the semver. Defaults to minor, use --patch or --major as needed.\n\
        + Edit Cargo.toml, replacing `version`.\n\
        + Run the cargo commands: `update`, `clippy -D warnings`, `fmt`.\n\
        + Commit and create a new semver tag for the version.\n\
        + If --install, run `cargo install`.\n\
        + Unless --patch was specified, perform the 3 following steps:\n\
        ++ Edit Cargo.toml, replacing `version` with the next minor with '-dev' prerelease.\n\
        ++ Run `cargo update` again.\n\
        ++ Commit.\n\
        + Push the new HEAD, then push the new tag.\n\
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
            VersionReq::parse(base)?
        } else {
            VersionReq::any()
        }
    };

    if let Some(branch) = branch {
        Command::new("git")
            .args(&["checkout", branch])
            .output_success()
            .context(format!("Failed to checkout branch {}", branch))?;
    }
    let install = matches.is_present("install");

    let out = Command::new("git")
        .args(&["status", "--porcelain=v2"])
        .output_success()?;
    if !out.stdout.is_empty() {
        let stdout = String::from_utf8(out.stdout)?.trim().to_owned();
        bail!(anyhow!(stdout).context("`git status` not empty; repo not clean"));
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
        let sv = Version::parse(&line[1..])?;
        if constraint.matches(&sv) {
            semver_tags.push(sv);
        }
    }
    let latest = {
        if let Some(v) = semver_tags.into_iter().max() {
            v
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

    update_cargo_toml_version(&new_version)?;

    Command::new("cargo")
        .arg("update")
        .output_success()?;

    Command::new("cargo")
        .args(&["clippy", "--", "-D", "warnings"])
        .output_success()?;

    Command::new("cargo")
        .arg("fmt")
        .output_success()?;

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

    if release != Patch {
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

    Command::new("git").args(&["push"]).output_success()?;

    Command::new("git")
        .args(&["push", "origin", &format!("v{}", new_version)])
        .output_success()?;
}

trait CommandPropagate {
    fn output_success(&mut self) -> ARes<Output>;
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
    let re = Regex::new(r#"^(version\w*=\w*")[^"]*("\w*)$"#)?;
    if !re.is_match(&manifest) {
        bail!("Could extract version from Cargo.toml, see --help for more info.");
    }
    let manifest = re.replace(&manifest, |c: &Captures| format!("{}{}{}", &c[1], version, &c[2]));
    File::create("Cargo.toml")?.write_all(manifest.as_bytes())?;
}
