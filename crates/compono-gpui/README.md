# compono-gpui

Connects a Compono tree to GPUI: a host, `update` that redraws only what changed, a view per component, router views, a list view and a task spawner.

This crate is part of [Compono](https://crates.io/crates/compono). Its documentation
has a compiled example on the first page.

The minimum Rust version is 1.96. `VERSIONING.md` in the repository says what a
version number promises.

## Security advisories

GPUI (`gpui-pre =0.3.12`) depends on `heapless 0.9`. `heapless` 0.9.3 and earlier
have the RustSec advisories RUSTSEC-2026-0574 to RUSTSEC-2026-0582, among them a
use-after-free in `FindMutView::pop`, a race in its Treiber stack and wrong
orderings in `Spsc`. All are fixed in 0.9.4, which this repository's `Cargo.lock`
uses. Compono's own code does not use `heapless`; whether GPUI reaches the faulty
parts is up to GPUI. An application has its own lock file: run
`cargo update -p heapless` so that it resolves 0.9.4 or later.

## Licence

Apache-2.0. See `LICENSE`.
