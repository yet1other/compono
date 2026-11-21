# Fixes from the review of 1.0.0-rc

Branch `fix/stable-review`, not committed. Line numbers of causes are those of
`1.0.0-rc`.

## 1. Summary

Every task of parts A, B, C and D is done except B9, whose repair is additive and is
deferred with its cost documented. The owner's answers are applied: the Maven group
is `io.github.yet1other`, the adapters stay `0.1.0-rc` with a yank step, a tree made
with no identity left is poisoned from the start, unclaimed saved state survives one
whole run (with a new saved-state format that still reads `1.0.0-rc`), GPUI is in
the Windows job, `compono-gpui` has its own `rust-version`, and D4 ran with the
tools installed. Every local check passes; GitHub Actions, the Windows job included,
did not run. No defect known to me stops the stable release.

## 2. Tasks

### A1 (4-1): a handle of one tree addresses a component of another tree

- **Cause.** `NodeId` (`crates/compono/src/component/handle.rs:10`) held only an
  index and a generation, and `Tree::slot` (`crates/compono/src/tree/mod.rs:296`)
  compared only those. The first node of every tree is index 0, generation 0.
- **Change.** `NodeId` is one `NonZeroU64` packing the tree's identity (12 bits),
  the generation (24 bits) and the index (28 bits). `Tree` claims an identity in
  `Tree::new` and gives it back in `Drop` (`crates/compono/src/tree/identity.rs`,
  new). `slot` and `slot_mut` compare the identity first, so every function that
  takes a handle or a `NodeId` (`get`, `send`, `phase`, `node_phase`, `contains`,
  `parent`, `handle_of`, `Cx::send`, …) treats a foreign one as stale. A slot now
  retires at its 16 777 215th generation instead of its 4 294 967 295th.
- **Identity design (wrap-around).** Identities live in a static bitmap of 4 096
  bits (`[AtomicU32; 128]`, 512 bytes). A new tree takes the next free bit after a
  shared counter (`fetch_add`, then `fetch_or` on the word); a dropped tree clears
  its bit with `fetch_and`. Ownership of a bit is decided only by read-modify-writes
  of one word, which are totally ordered whatever the `Ordering`, so every operation
  is `Relaxed` (stated in the module). Two live trees can never hold the same bit,
  whatever the counter has done: the counter only picks where to start looking, and
  `1 << 12` divides 2³², so it stays round when it wraps. An identity is reused only
  after its tree is dropped and the counter has passed every other identity. 0 is
  never issued (it keeps the niche) and 4 095 means "no tree": with 4 094 trees
  alive, a further tree takes 4 095 and is poisoned from the start, so
  `Tree::is_poisoned` returns `true`, every operation refuses, and `NativeTree`
  answers `Poisoned` (owner's answer 3; documented on `Tree::new` and
  `is_poisoned`). Test: `tree_identity_exhausted.rs` now asserts it; before the
  change it failed with "a tree with no identity looks healthy".
  It needs only 32-bit atomics, so it builds for `thumbv7em-none-eabihf` and without
  `std`. Not modelled with loom: the static cannot be a loom atomic; a threaded
  test covers it.
- **Why 64 bits.** Every native address crosses as one `i64` (commits, `ChildRef`,
  `back_start`, `contains`, and as dictionary keys in Swift and Kotlin). Keeping
  the size kept the FFI types, the Swift package and the Compose library unchanged;
  the bindings were regenerated with `boltffi pack` and needed no change for A1. A
  16-byte `NodeId` would have added 80 bytes to every `NodeSlot` and split every
  address into two numbers on the native side.
- **Size promises.** `ARCHITECTURE.md` "8 bytes" stays true; the asserts in
  `handle.rs` are kept and `Option<Handle<()>> == 8` and "the three parts fill 64
  bits" are added. `NodeSlot` stays 200/192/152/144 bytes.
- **Tests.** `crates/compono/tests/tree_identity.rs` (two trees: `get`, `phase`,
  `contains`, `parent`, `handle_of`, `node_phase`, `send`; bits from `to_bits`;
  four threads making and dropping 200 trees each). Before: all three failed
  (`bob.get(alices)` was `Some`, `send` was `Ok`). `tree_identity_exhausted.rs`
  holds 4 094 trees. `crates/compono-ffi/tests/native.rs`
  `an_address_from_one_native_tree_is_gone_in_another`, which failed on a checkout
  of `1.0.0-rc` with `left: Ok(()) right: Err(Gone)`. Unit tests in `handle.rs`.
- **Tests changed.** `design.rs` `a_reused_slot_does_not_revive_old_handles` masked
  the low 32 bits of the address as the slot; it now masks the low 28. The two tree
  unit tests on retired generations use the new limit.
- **Public API.** `NodeId::to_bits` has a new layout; bits from `1.0.0-rc` name
  nothing. A tree holds at most 268 435 454 components at once (an address past it
  is `NodeId::NONE`).

### A2 (4-2): a handle with the wrong type loses the message, and a debug build panics

- **Cause.** `Tree::send_now` (`tree/mod.rs:260`) returned `Ok(())` when
  `downcast_mut::<C>()` failed; `Node::accept` (`component/node.rs:26`) had a
  `debug_assert!` that panicked in the drain, which poisoned the tree.
- **Change.** `Tree::send` checks the type with `handle_of` first and returns
  `StaleHandle`. `Cx::send` refuses a target that is not a `K` (`TypeId` for itself,
  `handle_of` for a child) with `NotOwnChild`, its only error, which already covers a
  stale child. `Node::accept` returns `Result<(), StaleHandle>`, no `debug_assert!`.
- **Tests.** `tree::tests::a_handle_of_the_wrong_type_is_stale_for_tree_send` and
  `…_is_refused_by_cx_send` (crate-internal, since only `Handle::new` can forge one
  after A1). Before: debug build panicked at `node.rs:28`; release returned `Ok`.
  Both pass in debug and release, and `is_poisoned()` is `false`.
- **Decision.** `Cx::send` returns `NotOwnChild`, not `StaleHandle`: changing its
  error type breaks more and gives no stronger guarantee.

### A3 (5-1): concurrent saves corrupt the state file

- **Cause.** `crates/compono-desktop/src/state_file.rs:79` used one temporary name,
  `state.bin.tmp`, and `File::create` truncated another writer's file.
- **Change.** Each save creates `.state.bin.<pid>.<counter>.tmp` with
  `OpenOptions::create_new(true)` (retrying on `AlreadyExists`), syncs it, renames
  it, then syncs the directory. On Windows the directory is not synced: the standard
  library cannot open a directory there (it needs `FILE_FLAG_BACKUP_SEMANTICS`), and
  NTFS journals the rename; this is documented on `sync_dir`. A failed save removes
  its file. No lock between processes was added.
- **Tests.** `concurrent_stores_never_leave_a_torn_file`: two threads, 200 rounds,
  256 KiB and 192 KiB values, then no file left but `state.bin`. Before: "round 1: a
  torn file of 262144 bytes".
- **Tests changed.** `store_replaces_and_creates_the_directory` and
  `a_failed_store_leaves_no_temporary_file` looked for `state.bin.tmp`; they now
  check that the directory holds nothing else.

### A4 (2-1): a poisoned tree still saves its state

- **Cause.** `Tree::save` (`persist.rs:234`) did not check `is_poisoned`;
  `NativeTree::read` (`native_tree.rs:206`) did not either, so `save` worked too.
- **Change.** `SaveError` is a `#[non_exhaustive]` enum, `Encode` and `Poisoned`;
  `Tree::save` returns `Poisoned` before any `Persist::save`. `NativeTree::read` and
  so `save` return `NativeError::Poisoned`. Dropping a poisoned tree no longer tells
  components they are hidden (it stops their tasks and drops them). The Swift
  package's `ComponoRoot.persist` ignores empty data, and the Compose library keeps
  the last good bytes (`lastGoodState` in `Retained.kt`); the showcase returns empty
  bytes or throws when the save fails. The desktop showcases already wrote nothing
  on `None`.
- **Public functions that run user code, and their check.** `mount`,
  `mount_persistent`, `send`, `set_phase`, `back`, `back_start`, `back_cancel`,
  `back_commit`, `pump` (all through `Tree::operate`, already); `save` (new check);
  `Drop for Tree` (new: no phase change). `set_spawner` drops the previous spawner
  and runs no component code; it is left as it is. `NativeTree::with`, `read`,
  `save` check it. `Sender::send` runs no user code.
- **Tests.** `poison.rs` `a_poisoned_tree_saves_nothing_and_says_so` (before: `Ok`
  with one leaf saved) and `dropping_a_poisoned_tree_tells_no_component_it_is_hidden`
  (before: `["Hidden"]`). `native.rs` poisoned test (before: `read` was `Ok(())`).
  `compono-desktop` `a_poisoned_tree_leaves_the_last_state_file_as_it_was` (passes
  with the core fix; on `1.0.0-rc` the save would have been written). Kotlin
  `aFailedSaveKeepsTheLastGoodState` and Swift
  `testAnEmptySaveKeepsTheLastGoodStateFile` were written with the change and not
  run against the old code.
- **Tests changed.** `a_tree_whose_call_panicked_answers_poisoned` asserted that
  `read` still works on a poisoned `NativeTree`; it now asserts `Poisoned`.
- **Public API.** `SaveError` is an enum; `NativeTree::read` refuses a poisoned
  tree; `TreeBridge.save` may return empty data, meaning "keep the last state".

### A5 (7-1): the published URLs use the old GitHub account name

- `gh api repos/yet1other/compono` gave `full_name: yet1other/compono`.
- Changed: `Cargo.toml` `repository`, `README.md` Swift URL, POM `url`, `scm` and
  `developer` (`id`, `name`, `url`) in `native/android/compono/build.gradle.kts`. No
  `homepage`, badge or CI file named the account. The git remote is unchanged.
- **Maven group (owner's answer 1).** `io.github.yet1other` in `coordinates` of
  `build.gradle.kts` and in `README.md`, the only two places; `skills/` and the
  other docs had none. The Kotlin packages are `compono.android` and
  `compono.showcase.ffi`, with no `yet300`, so they are unchanged. `README.md` and
  `CHANGELOG.md` say that `io.github.yet300:compono:1.0.0-rc` stays on Maven Central
  and is replaced. Nothing was published.
- Check: `grep -rn "yet300"` outside `target`/build folders finds only the notes
  about the old artifact in `README.md` and `CHANGELOG.md`, and this report. The packaged `compono` manifest has
  `repository = "https://github.com/yet1other/compono"`, and
  `:compono:generatePomFileForMavenPublication` wrote the new `url`, `scm` and
  `developer`.

### B1 (4-3): an `Output` is lost when its child is removed before delivery

- **Cause.** `deliver_output` (`tree/execution.rs:140`) dropped an output whose
  child slot was gone.
- **Change.** Routes are `Rc` instead of `Box`. A slot counts its queued outputs
  (`queued_outputs: u16`, sticky at `u16::MAX`, fits in padding; `NodeSlot` size
  unchanged). When a child with queued outputs is removed, `readdress_outputs`
  rewrites them in place to `Work::Orphan { parent, route, payload }`, so they arrive
  in order; one whose parent is gone too is dropped at delivery. Documented on
  `Cx::emit` and in `ARCHITECTURE.md` "Execution".
- **Tests.** `outputs_before_removal.rs`: an editor emits `Close` then `Saved(7)`,
  the parent removes it on `Close`, and it emits `Saved(8)` when hidden. Before:
  `[Close]`. After: `[Close, Saved(7), Saved(8)]`. Second test: the parent goes too;
  nothing panics and the tree is not poisoned. (The brief's "sends `Close` to
  itself" cannot be written: a component has no handle of its own.)

### B2 (4-4): the default `KIND = ""` lets children restore the state of another type

- **Change.** `Persist::KIND` has no default. `assert_kind::<K>()`, a
  `const { assert!(!K::KIND.is_empty()) }`, runs in `mount_persistent`,
  `persistent_child*`, `save_node` and `Cx::restored`, so an empty kind fails to build
  (E0080, a `compile_fail` doctest on `KIND`). It is a post-monomorphisation error:
  `cargo build` and `cargo test` report it, `cargo check` does not. `restored` now
  always compares the kind. 55 implementations in the workspace got a `KIND`
  (scripted, `<file>.<type>`); the examples in `skills/` already had one.
- **Test.** `kind_required.rs`: a parent saves `Volume(4242)` and `Pin(11)` and is
  restored with them in the other order. Before (with no `KIND`):
  `volume=11 pin=4242`. After: each starts fresh.
- **Tests changed.** `saved_kinds.rs` `a_component_without_a_kind_restores_as_before`
  became `a_component_restores_state_its_own_kind_saved` (kind 0 is now `"plain"`).
  `named_routers.rs` `PLAIN_BYTES`, golden bytes saved with the empty kind, now hold
  the kind hashes; the keys are unchanged.

### B3 (4-7): one version for the workspace couples the core to the UI toolkits

- `compono-egui`, `compono-iced` and `compono-gpui` have `version = "0.1.0-rc"`;
  the other crates keep the workspace version. `VERSIONING.md` ("Two version
  lines", "UI toolkits", new "Releasing"), `README.md` (install) and `CHANGELOG.md`
  describe it. "Releasing" step 5 yanks the adapters' `1.0.0-rc` once their
  `0.1.0` is published (owner's answer 2). `compono-web` names no `web-sys` type publicly and `compono-tokio`'s
  Tokio is 1.x, so both stay on the core line.
- Check: `cargo package --list` worked for each crate, and `cargo pkgid` gives
  1.0.0-rc for `compono`, `compono-motion`, `compono-ffi`, `compono-test` and 0.1.0-rc
  for the three adapters. The workspace builds. Owner's decision (section 6).

### B4 (7-2): CI does not run the Swift and Kotlin tests

- New jobs `swift` (macOS: `boltffi pack apple`, `swift test`) and `android`
  (Ubuntu: Java 21, NDK 28.2.13676358, `cargo-ndk` 4.1.2, `boltffi pack android`,
  `gradle :compono:testDebugUnitTest` with Gradle 9.8.1) in `.github/workflows/ci.yml`.
- Run here: `boltffi pack apple` and `pack android` succeeded; `swift test` passed 31
  tests; `gradle :compono:testDebugUnitTest :app:assembleDebug
  :compono:generatePomFileForMavenPublication` succeeded. GitHub Actions was not run.

### B5 (6-1): Android crashes when a pump runs after `onCleared`

- **Cause.** `Retained.kt:52` closed the bindings but left them attached.
- **Change.** `ComponoTree.detach()`; `onCleared` calls it before `close()`. A pump
  posted before runs with no bridge and does nothing. Both run on the main thread, so
  no ordering is involved.
- **Test.** `RetainedTest.aPumpScheduledBeforeOnClearedDoesNothing`, through a real
  `ViewModelStore.clear()`. Before: `IllegalStateException: ShowcaseApp is closed`.

### B6 (6-2): on iPad, each window sends its `scenePhase` to the one tree

- **Step 1.** Confirmed by reading: `Lifecycle.swift:60` called `tree.setPhase` with
  each scene's own `scenePhase`, and `ComponoRoot` is one per process.
- **Change.** `ComponoTree.setScenePhase(_:for:)` and `removeScene(_:)` keep the
  phase of each connected scene and give the tree the highest. Each
  `componoLifecycle` names its scene with a `UUID` and forgets it on disappear. A
  scene going to the background still saves.
- **Tests.** `testTheTreeTakesTheHighestPhaseOfAllScenes` (three orders) and
  `testADisconnectedSceneNoLongerHoldsTheTreeUp`. Against stub methods with the old
  behaviour, both failed (four assertion failures, then a stale 2).

### B7 (5-2): a failure of `pushState` breaks the web history

- **Change.** `Browser::push` and `replace` return `Result<(), Refused>` (new error).
  `History::write` advances the model only past accepted entries and stops at the
  first refusal; the next `sync` writes the rest. `BrowserHistory` maps a thrown
  `pushState`/`replaceState` to `Refused`.
- **Tests.** Three in `crates/compono-web/tests/history.rs` with a fake browser that
  refuses: one push, one replace, three pushes; each then checks the browser's
  entries and that Back stays in the app. Before: `["/0"]` where `["/0", "/1"]` was
  expected, and the same for the other two.

### B8 (3-1): `cx.child_keyed` has a quadratic cost

- **Cause.** `has_child_keyed` (`tree/mod.rs:405`) walked every sibling.
- **Change.** `Tree::named_children: HashSet<(NodeId, u64)>` of live named children,
  inserted when the name is free and removed in `remove_leaf`. It grows by doubling,
  so there is no allocation per child. `has_child_keyed` is gone.
- **Measure.** `set_scaling` creates 1 000, 10 000 and 50 000 named children. Before:
  1 095, 11 587 and 60 975 ns per child. After: see section 4.
- **Test.** `named_children.rs`: a repeated name is refused while its first child
  lives, a refused sibling does not free the name, the name is free once its child
  is gone, and two parents may use one name. (Guards; they passed before as well.)

### B9 (3-2): `ChildSet::reconcile` costs O(n) for each edit

- Deferred (additive). `reconcile` documents that it costs the whole list, with
  numbers measured here, and `ARCHITECTURE.md` says so. `set_scaling` now prints the
  cost of one added row: 22 µs at 500 rows, 235 µs at 5 000, 1 418 µs at 50 000 and
  15 581 µs at 500 000. See section 3.

### C1 (2-2): a task that the executor drops never reports

- **Cause.** The future built in `Cx::perform` (`concurrency/task.rs:121`) reported
  only when it finished.
- **Change.** A `Report` guard inside the future reports the end once: the message,
  a failure after a panic, or, from its `Drop`, a failure when the executor drops it
  unfinished. Each report is one push to the inbox, which wakes the host once; the
  pump releases the guard and `Cx::cancel` then returns `false`. A task the
  component cancels may wake the host once more, for nothing (documented). `perform`
  now checks for a spawner before it builds the task, so `NoSpawner` wakes nothing.
- **Test.** `dropped_tasks.rs`: four tasks, the executor drops them. Before: 0 wakes
  (and `[true; 4]` outstanding). After: 4 wakes, `[false; 4]`. Loom: section 4.

### C2 (2-3): a `Scope` that is dropped between operations does not wake the host

- **Step 1.** Reproduced with a scope kept outside the tree: 0 wakes.
- **Change.** The graveyard (`router/scope.rs`, `Grave`) holds the host's waker and
  knows whether an operation runs (`Tree::operate` sets it, and `Drop for Tree`).
  Outside an operation the first scope buried wakes the host, once.
- **Test.** `scope_between_operations.rs`. Before: `left: 0 right: 1`.

### C3 (6-3): `NativeError` does not reach Swift and Kotlin

- **Change.** `native/showcase/ffi/src/component.rs` exports `#[boltffi::error]
  TreeError` (`WrongThread`, `Busy`, `Save`, `QueueClosed`, `Poisoned`,
  `UnknownPhase`, `Other`). The root's standard methods return
  `Result<_, TreeError>`: Swift's generated methods `throws`, Kotlin's raise
  `TreeError` exceptions. `Gone` stays normal use and gives nothing. `checked` no
  longer has a `debug_assert!` and `set_phase` no longer asserts. The Swift
  `TreeBridge` requirements are `throws`, and both packages log a failed call
  (`os.Logger`, `android.util.Log` through `reportFailure`) and answer as with no
  tree.
- **Decision.** Component objects' methods keep returning `Option`: a misuse there
  gives `None`, and the same misuse reaches the next root call, which every UI makes
  on each change and which throws. Making every component method throw would add
  `try` to every Swift view for no stronger guarantee.
- **Tests.** Rust: `a_call_from_another_thread_throws_its_kind`,
  `an_unknown_phase_code_throws` (they replace two tests that asserted the debug
  panic, which was the defect). Swift `testAThrownErrorIsLoggedAndAnsweredAsNoTree`
  and Kotlin `aThrownErrorIsReportedAndAnsweredAsNoTree`; their revert checks are in
  section 4. Accepted by the owner (answer 5).

### C4 (6-4): no binding calls `collect_retired`

- **Change.** `ShowcaseApp::collect_retired()` (static). Swift `TreeBridge` has a
  static `collectRetired()` requirement with an empty default; `ComponoTree.attach`
  keeps the type, and every pump calls it, even after the bindings are released.
  Kotlin `TreeBridge.collectRetired()` with an empty default, given to `treeBridge`
  by `collectRetired = { ShowcaseApp.collectRetired() }`; called at every pump,
  also after `detach`. The rule is documented on `compono_ffi::collect_retired`.
- **Tests.** Swift `testEveryPumpCollectsRetiredTreesEvenWithoutBindings`, Kotlin
  `everyPumpCollectsRetiredTreesEvenAfterDetach`.

### C5 (6-5): the generated JNI glue hides Kotlin exceptions in `commit`

- `ComponoTree.commit` catches every `Exception` and logs it. The generated
  `jni_glue.c` is not edited. Change BoltFFI would need: the generated callback glue
  should check `ExceptionCheck` after calling `commit`, log or rethrow, and clear it
  instead of returning silently. Test `aThrowingCommitIsReportedInsteadOfLost`: the
  bridge's `contains` throws during a commit's prune and is reported. Its revert
  check (section 4) shows that it fails only when both `catch` blocks are removed:
  the inner one in `call` already reports the failure, so the outer `catch` in
  `commit` is defence in depth that no test reaches.

### C6 (5-3): some spring parameters give NaN

- **Change.** `Spring` uses its damping ratio in `0..=Spring::MAX_DAMPING_RATIO` (4)
  and its stiffness in `0..=Spring::MAX_STIFFNESS` (100 000); NaN damping is 1 and
  NaN stiffness 0. `Spring::new` and `from_response` clamp; `advance` and
  `settle_time` clamp too, since fields stay public. In that range the 1 ms RK4 step
  is stable (|λ·dt| ≤ 2.49 < 2.78). `no_std`, no dependency (proptest is a
  dev-dependency only), no allocation. The presets are unchanged: the regenerated
  `golden/motion.csv` is byte-identical, so the Swift and Kotlin `MotionTests` read
  the same table (both suites pass).
- **Test.** `finite_springs.rs`, 2 000 proptest cases of NaN, ±∞, ±0, `f32::MAX`,
  1e12, ranges and any `f32`, checking position, velocity, settle time and
  `Timing::progress`; and 10 000 frames of a 1e12 stiffness. Before: minimal case
  `damping_ratio = NaN, stiffness = 3.4e38` and NaN frames. Its
  `.proptest-regressions` file is kept.

### C7 (5-5): a transition stops at very large values of `now_secs`

- **Change.** `TransitionState::frame` documents that motion is correct within 2⁴⁰ s
  of the clock's origin; past it, or for a non-finite reading or start, the
  transition ends at once.
- **Test.** `state.rs` `a_clock_out_of_range_ends_the_transition` (1e17, NaN, ∞).
  Before: "start 1e17: still running".

### C8 (5-4): saved entries that nobody claims stay forever

- **Why entries go unclaimed.** A component this version no longer builds, a child a
  run creates late or not at all, or a router entry not rebuilt yet (a lower stack
  entry, a page not shown). Routers that restore already drop what is not in their
  snapshot (`keep_router_entries`).
- **Change (after the owner's answer 4).** Each saved entry records `unclaimed`:
  whether it went through a whole run, from one restore to a save, without being
  claimed. At a save, a restored entry that nothing claimed yet is:
  - kept as it is when a live component or router can still claim it (the
    component at its path exists, or the deepest live component on its path still
    has the router whose entry the next key names);
  - kept with `unclaimed = true` when it was restored unmarked;
  - dropped when it was restored already marked, so it stayed unclaimed through
    the whole previous run and again in this one.
  So a lazily created child gets its state in the run after one that never created
  it, even if that run saved first, and state nothing claims is gone after two
  runs. Before anything is mounted, everything is carried forward unchanged.
- **Format.** The envelope is format `0x00c0_0003`. Bytes of `1.0.0-rc`
  (`0x00c0_0002`) are still read (`LegacyEnvelope`), every entry unmarked; the
  application's schema number is not touched. Documented on `Tree::save`, in
  `ARCHITECTURE.md` "Persistence" and in `CHANGELOG.md`.
- **Tests.** `unclaimed_entries.rs`:
  `a_child_created_only_after_a_save_still_restores_the_earlier_run` (three runs;
  the second saves before creating the child; the third gets 41). Before: `left: 0
  right: 41`. `state_nobody_claims_for_a_whole_run_is_dropped` (10 000 old notes are
  kept by the first run and dropped by the second). Before this rework: "the first
  run without notes dropped them". The named-stack test still passes.
- **Tests changed.** `named_routers.rs`: `PLAIN_BYTES` is now the new format, and
  the bytes of `1.0.0-rc` are kept as `LEGACY_BYTES`; the restore test reads both.

### C9 (4-5): `set_child_phase` above the phase of the parent returns `Ok`

- `Cx::set_child_phase` returns `Result<Phase, NotOwnChild>`, the phase the child has
  now. Smallest change: the same error, a value in place of `()`. Test
  `child_phase.rs` (stub returning the request failed with `Focused` vs `Visible`).
  Tests changed to the new type: `tree_laws.rs`, `cx_destroy.rs` (`.map(drop)`).

### C10 (4-6): `send` returns `Ok` at the work limit before all work is done

- Verified correct: `finish` wakes the host when work is left, and the next pump
  continues it. Documented on `Tree::send` and `Tree::set_work_limit`. Test
  `work_limit.rs` `send_at_the_limit_returns_ok_and_the_next_pumps_finish_the_work`
  (11 pieces, limit 4: `Ok`, 2 round trips done, one wake, the rest by pumps).

### C11 (4-8): public enums and structs can not grow without a major release

- **Made `#[non_exhaustive]`.** `compono-motion`: `Easing`, `Transition`, `Timing`,
  `BackAnimation`, `MotionPreference`, `StackChange`, `Motion`, `LayerStyle`,
  `BackEvent`, `StackFrame`, `Layer`, `Frame`, `Spring`. `compono`: `PanelsMode`,
  `router::link::Navigation`, and `SaveError` (A4). `compono-test`: `CommitRecord`,
  `Violation`. `compono-desktop`: `Modifiers`. `compono-web`: `history::EntryState`.
  Constructors added: `Navigation::new`, `BackEvent::new`, `Spring::new`,
  `Modifiers::new`, `EntryState::new`. `Motion` and `LayerStyle` start from a preset
  or `LayerStyle::REST` and have fields set (field assignment works on a
  non-exhaustive struct).
- **Already non-exhaustive.** `RestoreOutcome`, `Commit`, `Diagnostic`, `SendError`,
  `DiagnosticRecord`, `NativeError`, `Key`, `Convention`, `PageEvent`, `Address`.
- **Left exhaustive.** `Phase` (three phases, encoded across the FFI by
  `phase_code`), `SwipeEdge` (left, right, none), `Direction` (enter or exit, front
  or back), `Size` (width and height), `Panel`, `ShownPanels` and `PanelKeys` (one
  per panel of `PanelTypes`, whose three associated types fix the count; a fourth
  panel is a trait change anyway).
- Changed for it: the adapters' `Modifiers` literals, `compono-web`'s `Navigation`
  literals, `showcase-web`'s pattern (`..`), motion tests and the golden example,
  two doctests, and `router_laws.rs` (a wildcard arm).

### C12 (3-3): the allocation budgets cover only `Stack`

- `allocations.rs` `every_router_and_message_stays_within_its_budget`, with payloads
  that are not zero-sized: output 1, message to a child and its answer 2, select a
  page 3, scroll `Items` by one row 4, details panel open and close 1, dialog show
  and close 1 (debug and release alike). None was easy to remove: each payload is a
  `Box<dyn Any>` in the type-erased queue, and a page or row built again needs its
  node, route and scope.

### C13 (3-4, 3-5, 3-6): small performance items

- `navigation_cost`: the "named" rows are removed; the comment says why (a name is
  resolved once, when the router is built).
- `persist.rs:264`: `leaving` can be as large as a dropped router; it is now a
  `HashSet`.
- `Stack::push_to_front` (`router/stack.rs:403`): searched again after each removal,
  quadratic in equal entries. Now one `extract_if` pass, removing newest first as
  before. Covered by the `tree_laws` and `regressions` models.

### C14 (7-5): a dependency of `compono-gpui` has RustSec advisories

- `cargo audit` is not installed. Read from `rustsec/advisory-db` with `gh api`:
  `heapless` 0.9.3 has RUSTSEC-2026-0574 to RUSTSEC-2026-0582, all patched in
  0.9.4, and RUSTSEC-2020-0145 (patched in 0.6.1, not affected). `cargo update -p
  heapless --precise 0.9.4` worked within `gpui-pre`'s requirement. Compono's code
  does not use `heapless`. The `compono-gpui` README lists them and tells
  applications to update their own lock file. CI gains an `audit` job.

### D1 (7-3, 7-4): CI does not check all crates and all targets

- MSRV job (macOS, Rust 1.96) tests all ten published crates. `core` checks
  `compono-motion` for `thumbv7em`. `workspace` builds `compono-ffi` for
  `aarch64-linux-android` and `aarch64-apple-ios`. New `windows` job tests the
  whole workspace, GPUI included (owner's answer 6).
- **GPUI on Windows.** Not built here: a cross `cargo check --target
  x86_64-pc-windows-msvc -p compono-gpui -p showcase-gpui` from macOS stops in
  `gpui-pre`'s build script with `NotAttempted("llvm-rc")`, because it needs a
  Windows resource compiler. The Windows runner has one with MSVC, so this is a
  limit of cross-building, not a known failure on Windows. Unverified until the
  job runs.
- **MSRV (owner's answer 7).** `compono-gpui` has its own `rust-version = "1.96"`:
  it builds (`--all-targets`) on 1.96, 1.97 and 1.98 and its 8 tests pass on 1.96,
  and it cannot go lower than `compono`, which it depends on. `showcase-gpui`
  already declares 1.99, and 1.98 refuses it. The MSRV job is a matrix with one
  entry per minimum: the core crates and the egui and iced adapters on 1.96, and
  `compono-gpui` on its own version. All of them were run on 1.96 here: 326
  tests passed, 0 failed.

### D2 (7-6, 7-7): the benchmark numbers cannot be reproduced

- `benchmarks/run.sh` runs both; `benchmarks/RESULTS.md` maps each table to its raw
  file, with the machine (Apple M4, 24 GB) and the files' date (2026-11-10). The raw
  outputs in `out/` and both `Cargo.lock` files are no longer ignored (binaries
  are). The FFI-boundary iOS run was never written to a file; `run-apple.sh` now
  writes `out/apple-*.log`. The benchmark builds use `--locked`. `README.md` has a
  "Benchmarks" section: which numbers include the FFI boundary and how each side
  measures memory. Every cargo command in CI uses `--locked`, every action is pinned
  to a SHA with its version.

### D3: the 100 000-row load example has no assertions

- `examples/showcase/tests/load_memory.rs`: 7 bytes per key and 435 per live row
  measured; asserts at most 16 and 650. CI runs it with `--release`.

### D4: checks that the review did not complete

Tools installed with the owner's agreement: `cargo-audit`, `cargo-semver-checks`,
`cargo-public-api`, and Miri on nightly.

- **Miri** (`cargo +nightly miri test`, `PROPTEST_CASES=4`,
  `-Zmiri-disable-isolation` so proptest can read its regression files, 15 minutes
  per file): `audit` 7 passed (1 skipped: `a_deep_chain_changes_phase_saves_and_is_destroyed_without_overflowing`,
  which builds a very deep tree on purpose and ran for over 20 minutes under
  Miri), `regressions` 18, `routers` 9 (all passed, then the process hit the limit
  while exiting), `router_ids` 3, `rebuilt_router` 4, `properties` 2, `tree_laws` 4,
  `router_laws` 4, `compono-web` `history` 14, `compono-motion` `springs` 4. No
  undefined behaviour reported, 0 failed.
- **`cargo semver-checks --baseline-version 1.0.0-rc --release-type minor`**, so
  that every major break is reported although the version did not change (without
  `--release-type` it skipped all 260 lints). Breaks found: `compono`: `SaveError`
  became an enum, `Persist::KIND` lost its default, `PanelsMode` and `Navigation`
  non-exhaustive; `compono-motion`: 13 types non-exhaustive; `compono-test`:
  `Violation`, `CommitRecord`; `compono-desktop`: `Modifiers`; `compono-web`:
  `EntryState`, and `Browser::push` and `replace` return a value. `compono-ffi` and
  `compono-tokio`: none. **Every one is in `CHANGELOG.md`; none is unlisted.**
  Breaks it cannot see, also in `CHANGELOG.md`: the layout of `NodeId::to_bits`,
  `NativeTree::read` refusing a poisoned tree, the saved-state format.
- **`cargo public-api diff 1.0.0-rc`**: the same items, plus
  `Cx::set_child_phase` returning `Result<Phase, NotOwnChild>` (4-5, listed) and
  `NodeId` now showing as a tuple struct with a private field (no public field
  either way). Nothing unlisted.
- **`cargo audit`**: 0 vulnerabilities, 10 warnings, all in toolkit dependencies:
  unmaintained `chrono`, `instant`, `paste`, `rustls-pemfile`, `rustybuzz`,
  `ttf-parser`, `zbus_names`, `zvariant`, `zvariant_derive` (through GPUI, iced or
  their tooling), and unsound `lru 0.16.4` (RUSTSEC-2026-0253) through iced's
  `cryoglyph`. The fix, `lru` 0.18.2, is outside iced's requirement; the
  `compono-iced` README now says so. Compono's own code uses none of them.
- **`swift test` and the Gradle unit tests**: section 4.

## 3. Deferred tasks

- **B9 (3-2).** Additive repair for a minor release: point operations on `ChildSet`
  that cost what they change, `insert(index, key, phase, cx)`,
  `remove(&key, cx)` and `move_to(&key, index, cx)`, each `O(k)` in the rows touched
  plus a `Vec` shift, with an index from key to position kept beside `entries`
  (`HashMap<K, usize>`, updated on the shifted range, or a position-free order list).
  `reconcile` stays as it is. `router_scaling` would gain one measurement editing one
  row of 1 000, 10 000 and 100 000.

## 4. Checks

Run on an Apple M4, macOS 27.2.1, Rust 1.99.1, after the last change, with every
command of "Before finishing a change" in `AGENTS.md` and the extra ones the brief
lists.

| Command | Result |
|---|---|
| `cargo fmt --check` | ok (one formatting diff in a new test was fixed, then rerun) |
| `cargo clippy --workspace --all-targets -- -D warnings` | ok |
| `cargo clippy -p compono --no-default-features -- -D warnings` | ok |
| `cargo clippy -p compono --no-default-features --features std -- -D warnings` | ok |
| `cargo clippy -p compono --no-default-features --features serde -- -D warnings` | ok |
| `cargo test --workspace` | 355 passed, 0 failed |
| `cargo test -p compono --no-default-features` | 75 passed, 0 failed |
| `cargo check -p compono --no-default-features --target thumbv7em-none-eabihf` | ok |
| same with `--features serde` | ok |
| `cargo check -p compono-motion --target thumbv7em-none-eabihf` | ok |
| `cargo clippy -p compono-web -p showcase-web --target wasm32-unknown-unknown -- -D warnings` | ok |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` | ok |
| same for `compono-web` on `wasm32-unknown-unknown` | ok |
| `cargo test -p compono --release --test allocations` | 5 passed |
| `cargo test -p showcase --release --test load_memory` | 1 passed |
| loom (`--cfg loom`, `--release`, `target/loom`) | 6 passed |
| `scripts/sync-skills.sh --check` | ok |
| `native/showcase/ffi`: clippy `-D warnings`, `cargo test` | ok, 8 passed |
| `cargo audit` | 0 vulnerabilities, 10 warnings (D4) |
| Rust 1.96: tests of every published crate but GPUI | 326 passed, 0 failed |
| Rust 1.96: `compono-gpui` build `--all-targets` and tests | ok, 8 passed; also builds on 1.97 and 1.98 |
| Miri, `cargo semver-checks`, `cargo public-api` | D4 |
| GitHub Actions, including the Windows job | not run |

Native side, after the last change:

| Command | Result |
|---|---|
| `boltffi pack apple --release --deny-skipped` | ok |
| `boltffi pack android --release --deny-skipped` (NDK 28.2.13676358) | ok |
| `swift test` in `native/apple/Compono` | 31 passed, 0 failed |
| `gradle :compono:testDebugUnitTest :app:assembleDebug :compono:generatePomFileForMavenPublication` | 28 passed, 0 failed; build successful; POM `groupId` `io.github.yet1other` |
| `xcodebuild` of `native/showcase/ios`, Debug, iOS Simulator | build succeeded |

### Swift and Kotlin tests written with their fix: revert checks

Each fix was reverted on its own, its test run, and the fix put back (the test then
passed again). The harness was a temporary script, since deleted.

| Test | Fix reverted | Result with the fix reverted |
|---|---|---|
| Kotlin `aFailedSaveKeepsTheLastGoodState` | `lastGoodState` returns `fresh` | failed |
| Kotlin `aThrownErrorIsReportedAndAnsweredAsNoTree` | `call` without its `catch` | failed |
| Kotlin `aThrowingCommitIsReportedInsteadOfLost` | `commit`'s `catch` only | **passed**: the inner `catch` in `call` reports it |
| same | both `catch` blocks | failed |
| Kotlin `everyPumpCollectsRetiredTreesEvenAfterDetach` | no `collectRetired()` in the pump | failed |
| Swift `testAnEmptySaveKeepsTheLastGoodStateFile` | `persist` without `!state.isEmpty` | failed (1 failure) |
| Swift `testAThrownErrorIsLoggedAndAnsweredAsNoTree` | `call` with `try!` | failed (fatal error: `try!` raised `Refused`) |
| Swift `testEveryPumpCollectsRetiredTreesEvenWithoutBindings` | no `collectRetired()` in `runPump` | failed (2 failures) |

### Scaling examples, three runs each (release, `panic = "abort"`)

Two sessions. The first was on mains power; for the second, after the last change,
the Mac was on battery in Low Power Mode, which roughly halves its speed, so
`1.0.0-rc` was measured again under the same conditions. Compare within a session.

**Session 1, mains power** (`1.0.0-rc`, then this branch before the owner's
answers):

| | Before | After |
|---|---|---|
| `navigation_cost` push+pop, depth 1 / 10 / 100 (ns) | 165–203 / 150 / 150 | 190–206 / 156–180 / 156–170 |
| hide+show of the owner, depth 1 / 100 / 10 000 | 39–43 | 39–43 |
| `router_scaling` select page, 100 / 10 000 / 1 000 000 | 247–324 / 186–188 / 189–194 | 253–294 / 194–218 / 190–197 |
| scroll one row | 546–582 / 518–548 / 519–568 | 593–607 / 557–572 / 543–566 |
| details open and close | 101–104 | 119–137 |
| dialog open and close | 75–77 | 90–109 |
| `set_scaling` create per child, 1 000 / 10 000 / 100 000 / 1 000 000 | 123–205 / 108–191 / 98–142 / 61–69 | 138–250 / 125–224 / 93–159 / 65–73 |
| named children per child, 1 000 / 10 000 / 50 000 | 1 095 / 11 587 / 60 975 (one run) | 56–64 / 57–61 / 47–52 |

**Session 2, Low Power Mode** (`1.0.0-rc` once, this branch after every change three
times):

| | Before | After |
|---|---|---|
| `navigation_cost` push+pop, depth 1 / 10 / 100 (ns) | 304 / 288 / 288 | 323–328 / 309–311 / 308–311 |
| hide+show of the owner, depth 1 / 100 / 10 000 | 75–83 | 78–85 |
| `router_scaling` select page, 100 / 10 000 / 1 000 000 | 402 / 373 / 373 | 373–415 / 372–404 / 367–401 |
| scroll one row | 1 092 / 1 041 / 1 041 | 1 114–1 195 / 1 051–1 137 / 1 054–1 139 |
| details open and close | 197–198 | 234–239 |
| dialog open and close | 147–148 | 177–193 |
| `set_scaling` named children per child, 1 000 / 10 000 / 50 000 | not run | 111–113 / 101–105 / 201–225 |
| `set_scaling` one added row, 500 / 5 000 / 50 000 / 500 000 | not run | 17–21 µs / 178–237 µs / 1.5–2.0 ms / 31–39 ms |

Every row stays flat as the size grows; the named children's last column rises
from cache misses, not from the number of siblings. Each operation costs a
constant more than in `1.0.0-rc`, in both sessions: about 5–8 % per push and pop
and about 20 % per panel or dialog cycle. Likely sources are the identity compare in
every slot lookup (A1), the queued-output count on every emit and delivery (B1),
`Rc` routes (B1) and the graveyard's operation flag (C2); it was not profiled.

## 5. Not done

- **B9.** Deferred (section 3).
- **GitHub Actions.** None of the jobs ran on GitHub. GPUI could not be checked for
  Windows from macOS (D1). The Windows job is the first place it builds.
- **Miri.** One test skipped, `audit.rs`
  `a_deep_chain_changes_phase_saves_and_is_destroyed_without_overflowing`: it runs
  for more than 20 minutes under Miri, because it builds a very deep tree on purpose.
- **Not on a device or simulator.** The native showcase was built (`xcodebuild`,
  `:app:assembleDebug`) but not run.
- **Guards, not failing tests.** `named_children.rs` (B8) and the stack half of
  `unclaimed_entries.rs` (C8) pass on old code too; the `compono-desktop` poisoned
  file test (A4) fails only with the old core. The Kotlin outer `catch` in `commit`
  (C5) is reached by no test (section 4).
- `docs/prompts/fix-stable-review.md`, untracked when work began, is no longer in
  the folder. Nothing in this work touched `docs/prompts`.

## 6. Questions for the owner

The eight questions of the first report are answered and applied. Left open:

1. **Maven Central.** The new namespace `io.github.yet1other` must be registered
   before the Compose library can be published under it (the owner's step).
2. **C5 in BoltFFI.** Should an issue go to BoltFFI asking its JNI glue to check
   and report a Kotlin exception after a callback? The library now catches it
   itself.
3. **`lru` through iced.** Accept RUSTSEC-2026-0253 until iced moves to `lru` 0.18,
   as the `compono-iced` README says?
