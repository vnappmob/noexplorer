# Contributing

Thanks for helping! NoExplorer is deliberately small. Simplicity, speed and safety (never
lose user data) come before features.

1. **Read [AGENTS.md](AGENTS.md).** It's short and covers the map, commands, invariants and recipes.
2. Set up: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).
3. For anything bigger than a bug fix, open an issue first to agree on the approach.
4. Keep PRs focused. Run the checks (`npm run check`, `cargo fmt --check`,
   `cargo clippy --all-targets -- -D warnings`, `cargo test`).
5. Update the docs listed in AGENTS.md → *Keep docs in sync*, and add a `CHANGELOG.md` entry
   under **Unreleased** for user-visible changes.
6. Commit messages: Conventional Commits (`feat:`, `fix:`, `docs:`…).

By contributing you agree that your contributions are licensed under the MIT License.
