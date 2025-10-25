# General
- If you don't know 3rd-party API, should lookup on `https://docs.rs/<crate>`.

# For procedure macros

- All type reference in the macro should be full qulified path.
- All type reference must be in (or re-exported in) occams_rpc_stream crate (so that user don't need use multiple crates)
- Add compile_fail doc test for unexpected syntax, be aware that don't add doc test in test files, doc tests only run with lib files.

# For async trait

- macro should support with and without `#[async_trait]`
- All trait definition by us should avoid using `#[async_trait]` as much as possible, prefer to use `impl Future + Send`.


