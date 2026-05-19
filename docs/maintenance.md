# Maintenance

This chapter is for maintainers of `fuse-promise`.

The repository is a Linux system component. Keep the public boundary small:

```text
external applications
  -> fuse-promise/fuse-promise.h
  -> libfusepromise.so
  -> fuse-promised
  -> FUSE
```

Do not document or expose daemon IPC as a public interface.

## Documentation Maintenance

User documentation lives in `docs/` and is published with GitHub Pages.

Local documentation check:

```sh
uv tool run --with mkdocs==1.6.1 --with mkdocs-material==9.7.6 \
  mkdocs build --strict --site-dir site
```

The dependency pins live in:

```text
requirements-docs.txt
```

Navigation lives in:

```text
mkdocs.yml
```

When adding a user-facing page, add it to `nav`. When keeping an internal note
in the repository but out of the user guide, add it to `not_in_nav`.

## Public API Maintenance

The public API is defined by:

```text
include/fuse-promise/fuse-promise.h
```

When changing public behavior, update these files together:

```text
include/fuse-promise/fuse-promise.h
docs/public-api.md
examples/minimal_provider.c
tests/minimal-provider-smoke.sh
```

Compatibility rules:

- Keep public symbol names stable.
- Do not renumber existing status values or conflict policies.
- Keep public structs versioned with `struct_size`.
- Add new public fields only in a backward-compatible way.
- Keep Rust types, internal IPC messages, daemon storage, and FUSE adapter
  internals out of the public ABI.

## FUSE Backend Maintenance

The daemon supports two package variants:

| Package | Daemon feature |
|---|---|
| `fuse-promise` | `fuse-mount-fuse` |
| `fuse3-promise` | `fuse-mount-fuse3` |

The installed command names stay the same for both packages:

```text
fuse-promised
fpctl
```

When changing backend behavior, check both builds:

```sh
cargo check -p fuse-promise-daemon --features fuse-mount-fuse --locked
cargo check -p fuse-promise-daemon --features fuse-mount-fuse3 --locked
```

Mounted smoke tests:

```sh
FUSE_PROMISE_FUSE_BACKEND=fuse tests/minimal-provider-smoke.sh
FUSE_PROMISE_FUSE_BACKEND=fuse3 tests/minimal-provider-smoke.sh
```

## Package Maintenance

Packaging is maintained through:

```text
scripts/install-dev.sh
scripts/package-linux.sh
packaging/nfpm.yaml
.github/workflows/release.yml
docs/packaging.md
```

The release workflow builds DEB and RPM artifacts for both FUSE2 and FUSE3.
Those package builds run inside `ubuntu:18.04` through
`scripts/package-linux-bionic-container.sh` so released binaries keep a glibc
2.27 compatibility floor.

Before changing package names, dependencies, or installed paths, verify:

```sh
BUILD_PROFILE=release SONAME_MAJOR=1 tests/install-metadata.sh
```

## Release Maintenance

For a new release:

1. Update the workspace version in `Cargo.toml`.
2. Update `Cargo.lock`.
3. Add a `CHANGELOG.md` entry.
4. Add `docs/release-notes-<version>.md`.
5. Run the release gates.
6. Commit the release preparation.
7. Push `main`.
8. Create and push an annotated `v<version>` tag.
9. Confirm the GitHub Release contains DEB, RPM, source tarball, and
   `SHA256SUMS` assets.

Release gate:

```sh
BUILD_PROFILE=release SONAME_MAJOR=1 tests/stable-release-gates.sh
```

GitHub Release assets are published by `.github/workflows/release.yml`.
Publishing to a public package repository requires the Cloudsmith repository
secret and variables documented in [Packaging](packaging.md).
