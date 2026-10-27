# Contributing to aichurn

Thanks for taking the time to help. Bug reports, ideas, docs fixes and code are all welcome.

## Before you start

- **Bugs:** open an [issue](https://github.com/celaldrg/aichurn/issues/new/choose) with steps to reproduce.
- **Bigger changes:** open an issue first so we can agree on the approach before you spend time on it.
- **First contribution?** Look for issues labeled [`good first issue`](https://github.com/celaldrg/aichurn/labels/good%20first%20issue).

## Development setup

You need Rust 1.78 or newer and git. There are no third-party crates.

```sh
git clone https://github.com/celaldrg/aichurn
cd aichurn
cargo test
cargo run --release -- . --since all          # this repo
cargo run --release -- vercel/next.js          # any public GitHub repo
```

### Check the replay

`AICHURN_VERIFY=1` compares the line counts aichurn reconstructed for every file with the real files at the
analysed revision. Any change to `src/patch.rs` or `src/engine.rs` should keep this at "0 differ":

```sh
AICHURN_VERIFY=1 cargo run --release -- denoland/deno --json > /dev/null
# verify: 5,8xx files match, 0 differ, …
```

## Adding a new agent

Agent signatures live in [`src/agents.rs`](src/agents.rs).

1. Find a **public, self-declared** signal: a co-author trailer, a bot account or a footer the tool writes by itself. Link to an example commit in your PR.
2. Make the pattern strict enough that it can never match a human (people are called Claude, Devin and Jules), or a release bot that happens to share the tool's name.
3. Add a positive *and* a negative case to the tests at the bottom of `src/agents.rs`.

## Pull requests

1. Fork the repository and create a branch from `main`.
2. Keep the change focused: one fix or feature per pull request.
3. Add or update tests for any behavior change. CI (`cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`) must pass.
4. Update the README if you change flags, output or behavior users can see.

Commit messages: short imperative summary (`Add --quiet flag`, `Fix rename handling`).

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). By taking part you agree to follow it.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
