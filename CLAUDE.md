<!-- Claude Code reads this file automatically. The single source of truth for agents is AGENTS.md. -->
@AGENTS.md

## Storage policy (maintainer's rules for agents, 2026-10-16)

`src-tauri/target` once grew to 22 GB, and several more GB every day. These rules keep it bounded.

**Hard limits:** `src-tauri/target` ≤ **12 GB**, and the disk keeps **> 60 GB free**.
`scripts/clean-build.sh` (`npm run clean:build`) measures both and cleans in tiers. Tier 0
always removes leftovers of the old crate name `noback`. Tier 1 removes incremental caches.
Tier 2 removes this crate's own artifacts (the `noexplorer` binary, `libnoexplorer_lib*`, test
binaries, fingerprints, build-script output). Dependencies stay, so the next build only
recompiles the app crate. It exits 1 when it can't get under the limits.

1. **Run `scripts/clean-build.sh` at the start of every session and after every release build**,
   without being asked. If it exits 1 (still over the limits), **stop and tell the maintainer
   before building anything else.**
2. **Never run a full `cargo clean`** (or delete `target/` or `deps/`) unless the maintainer asks.
3. **Prefix every cargo command you run with `CARGO_INCREMENTAL=0`.** Agents build once and move
   on, so incremental caches are pure waste. Don't change the maintainer's own setup: their
   `npm run tauri dev` keeps incremental builds.
4. **Verify with `cargo check` and `cargo clippy`.** Use `cargo build` / `cargo test` only when
   something has to actually run. **Never build a release "to try it"**; release builds happen
   only when the maintainer approves a release.
5. **No test files or fixtures over 100 MB** in the scratchpad or the repo. Generate them inside
   the test and delete them right after.
6. **Remove worktrees when their work is done**: the worktree and its branch, once the branch is
   merged or the maintainer says it's no longer needed.
7. Changes to build profiles in `src-tauri/Cargo.toml` are **proposed with before/after
   measurements** (target size, build time) and applied only after the maintainer approves.
