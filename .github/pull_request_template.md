## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## How it was tested

<!-- Tests added, and how you checked the behaviour (CLI, MCP, screenshots). -->

## Checklist

- [ ] `cargo xtask ci` passes (fmt, clippy, tests, assets, brands, layers, wasm).
- [ ] No panics in shipped code: no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!` or
      `unimplemented!` outside tests, and no unchecked indexing on data from files, parameters or
      messages. Errors are returned as `Result` and shown to the user
      ([why and how](https://github.com/storytold/vectorcraft/blob/main/docs/development.md#robustness-vector-w3k2-never-crashes)).
- [ ] Clean-room: nothing copied from Adobe products or GPL/AGPL code ([AGENTS.md](https://github.com/storytold/vectorcraft/blob/main/AGENTS.md)).
- [ ] Every new asset has a row in `ASSETS.md`.
