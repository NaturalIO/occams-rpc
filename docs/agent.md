# General
- All comment must be in English. Omit unnecessary comment.
- If you don't know 3rd-party API, should lookup on `https://docs.rs/<crate>`.

# For procedure macros

- All type reference in the macro should be full qualified path.
- In order to avoid the need to use multiple crates,  all type reference in ./stream/macros/ must be in (or re-exported in) `razor-rpc-stream` crate, and the reference in ./macros/ must be in `razor-rpc` crate.
- Add compile_fail doc test for unexpected syntax, be aware that don't add doc test in test files, doc tests only run with lib files.

# For async trait

- macro should support with and without `#[async_trait]`
- All trait definition by us should avoid using `#[async_trait]` as much as possible, prefer to use `impl Future + Send`.


