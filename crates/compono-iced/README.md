# compono-iced

Connects a Compono tree to iced: a host, `Action` messages, a subscription for work from other threads, router views, a list view and Back shortcuts.

This crate is part of [Compono](https://crates.io/crates/compono). Its documentation
has a compiled example on the first page.

The minimum Rust version is 1.96. `VERSIONING.md` in the repository says what a
version number promises.

## Security advisories

iced 0.14 renders text through `cryoglyph`, which depends on `lru 0.16`. RustSec
RUSTSEC-2026-0253 marks `lru` before 0.18.2 unsound: `LruCache::pop` can free a
value twice if a `Drop` panics. The fix is outside iced's version requirement, so
it arrives with an iced release. Compono's own code does not use `lru`.

## Licence

Apache-2.0. See `LICENSE`.
