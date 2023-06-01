# fqcv

<p align="center">
  <a href="https://github.com/fulcrumgenomics/fqcv/actions?query=workflow%3ACheck"><img src="https://github.com/fulcrumgenomics/fqcv/actions/workflows/build_and_test.yml/badge.svg" alt="Build Status"></a>
  <br>
</p>

Alignment for vector quality control

## Installing

### Installing with `cargo`
To install with cargo you must first [install rust](https://doc.rust-lang.org/cargo/getting-started/installation.html).
Which (On Mac OS and Linux) can be done with the command:

```console
curl https://sh.rustup.rs -sSf | sh
```

Then, to install `fqcv` run:

```console
cargo install fqcv
```

### Building From Source

First, clone the git repo:

```console
git clone https://github.com/fulcrumgenomics/fqcv.git
```

Secondly, if you do not already have rust development tools installed, install via [rustup](https://rustup.rs/):

```console
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then build the toolkit in release mode:

```console
cd fqcv
cargo build --release
./target/release/fqcv --help
```

## Developing

fqcv is developed in Rust and follows the conventions of using `rustfmt` and `clippy` to ensure both code quality and standardized formatting.
When working on fqcv, before pushing any commits, please first run `./ci/check.sh` and resolve any issues that are reported.

## Releasing a New Version

### Pre-requisites

Install [`cargo-release`][cargo-release-link]

```console
cargo install cargo-release
```

### Prior to Any Release

Create a release that will not try to push to `crates.io` and verify the command:

```console
cargo release [major,minor,patch,release,rc...] --no-publish
```

Note: "dry-run" is the default for cargo release.

See the [`cargo-release` reference documentation][cargo-release-docs-link] for more information

### Semantic Versioning

This tool follows [Semantic Versioning](https://semver.org/).  In brief:

* MAJOR version when you make incompatible API changes,
* MINOR version when you add functionality in a backwards compatible manner, and
* PATCH version when you make backwards compatible bug fixes.

### Major Release

To create a major release:

```console
cargo release major --execute
```

This will remove any pre-release extension, create a new tag and push it to github, and push the release to creates.io.

Upon success, move the version to the [next candidate release](#release-candidate).

Finally, make sure to [create a new release][new-release-link] on GitHub.

### Minor and Patch Release

To create a _minor_ (_patch_) release, follow the [Major Release](#major-release) instructions substituting `major` with `minor` (`patch`):

```console
cargo release minor --execute
```

### Release Candidate

To move to the next release candidate:

```console
cargo release rc --no-tag --no-publish --execute
```

This will create or bump the pre-release version and push the changes to the main branch on github.
This will not tag and publish the release candidate.
If you would like to tag the release candidate on github, remove `--no-tag` to create a new tag and push it to github.

[cargo-release-link]:      https://github.com/crate-ci/cargo-release
[cargo-release-docs-link]: https://github.com/crate-ci/cargo-release/blob/master/docs/reference.md
[new-release-link]:        https://github.com/fulcrumgenomics/fqcv/releases/new