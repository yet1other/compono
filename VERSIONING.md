# Versions

## Two version lines

The core crates share one version, set as `version` in the workspace manifest, and
are released together: `compono`, `compono-motion`, `compono-test`, `compono-ffi`,
`compono-tokio`, `compono-desktop` and `compono-web`. None of them names a type of
a UI toolkit in its public interface, so a toolkit release never changes them.

The adapters `compono-egui`, `compono-iced` and `compono-gpui` each have a version
of their own, set in their own manifest and kept below `1.0.0`. Each names types
of its toolkit (egui 0.37, iced 0.14, gpui 0.3) in its public interface, and those
toolkits are below `1.0.0` themselves. Under Semantic Versioning, `0.x` gives a
breaking release a new minor number, so a toolkit release that breaks an adapter
raises that adapter's minor version and leaves every other crate alone. An
adapter depends on the core crates at their release line, as an application does.

The Swift package in `native/apple/Compono` and the Compose library in
`native/android/compono` carry the version of the core crates they were tested
with.

## Semantic versions

Compono follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

| Change | Version |
|---|---|
| Removing or renaming a public item, changing a signature, adding a required trait method, or changing documented behaviour | major |
| A new public item, a new provided trait method, a new variant of a `#[non_exhaustive]` enum, a new field of a `#[non_exhaustive]` struct | minor |
| A fix that makes behaviour match the documentation, a faster or smaller implementation, documentation | patch |

These are also part of the public interface, and changing them is a major change:

- **The format of saved state.** Bytes from `Tree::save` of version 1.x are read by
  every later 1.y, for the same schema number and the same component and route types.
  How a router numbers its entries is part of that format.
- **The order of lifecycle and Back.** Parents rise before children and fall after
  them, an old entry is lowered before a new one is raised, and Back goes to the
  deepest visible component, the most recently shown first.
- **The allocation budget** enforced by `crates/compono/tests/allocations.rs`.
- **The numbers that cross a language boundary:** `NodeId::to_bits` and `phase_code`.
- **Cargo features.** A feature is never removed and never starts to remove items.

These are not part of the public interface:

- Anything under `examples/`, `benchmarks/` and `native/showcase/`.
- The text of `Debug` and `Display` output.
- Exact timings.
- The state that `History` stores in a browser history entry. An entry written by
  another version is treated as an entry the application does not know.

A release candidate, such as `1.0.0-rc`, can still change in any way before `1.0.0`.

## UI toolkits

An adapter follows its toolkit: supporting a toolkit release that breaks the
toolkit's types is a new minor version of the adapter, `0.x` to `0.(x+1)`, and a
patch of the adapter otherwise. The rules of the table above apply to an adapter
with "minor" in place of "major" and "patch" in place of "minor", as Semantic
Versioning has it for `0.x`. An adapter reaches `1.0.0` only after its toolkit does.

## Releasing

1. Set the core version once, in `[workspace.package]` of the root `Cargo.toml`,
   and the same number in every `compono-*` dependency of the core crates.
2. Set each adapter's own `version` in its manifest, and its dependencies on the
   core crates to the new core version.
3. Give the Swift package tag and `componoVersion` of the Compose library the core
   version.
4. Publish the core crates in dependency order (`compono-motion`, `compono`,
   `compono-test`, `compono-desktop`, `compono-tokio`, `compono-web`,
   `compono-ffi`), then the adapters. `cargo publish --dry-run -p <crate>` shows
   the version each will take.
5. Once the adapters' `0.1.0` is published, yank their `1.0.0-rc` on crates.io,
   which was published before they had versions of their own:
   `cargo yank compono-egui@1.0.0-rc`, then the same for `compono-iced` and
   `compono-gpui`. Projects that already lock it keep building.

## Minimum Rust version

The minimum supported Rust version is **1.96**, set as `rust-version` in the
workspace manifest and checked by CI on that toolchain. `compono-gpui` sets its own
`rust-version`, today also 1.96, the lowest that builds it; it can rise with GPUI
without changing the others, and CI checks it on its own toolchain.

- Raising it is a minor change, never a patch.
- It is raised only to a version that has been stable for at least six months.
- The changelog names the new minimum in the entry of the release that raises it.

## Deprecation

An item is marked `#[deprecated]` for at least one minor release before a major
release removes it. The note names the replacement.
