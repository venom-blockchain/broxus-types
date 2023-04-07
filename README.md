<p align="center">
  <a href="https://github.com/venom-blockchain/developer-program">
    <img src="https://raw.githubusercontent.com/venom-blockchain/developer-program/main/vf-dev-program.png" alt="Logo" width="366.8" height="146.4">
  </a>
</p>

# Everscale types &emsp; [![crates-io-batch]][crates-io-link] [![docs-badge]][docs-url] [![rust-version-badge]][rust-version-link] [![workflow-badge]][workflow-link]

[crates-io-batch]: https://img.shields.io/crates/v/everscale-types.svg

[crates-io-link]: https://crates.io/crates/everscale-types

[docs-badge]: https://docs.rs/everscale-types/badge.svg

[docs-url]: https://docs.rs/everscale-types

[rust-version-badge]: https://img.shields.io/badge/rustc-1.65+-lightgray.svg

[rust-version-link]: https://blog.rust-lang.org/2022/11/03/Rust-1.65.0.html

[workflow-badge]: https://img.shields.io/github/actions/workflow/status/broxus/everscale-types/master.yml?branch=master

[workflow-link]: https://github.com/broxus/everscale-types/actions?query=workflow%3Amaster

> Status: WIP

## About

A set of primitive types and utilities for the Everscale blockchain.

Heavily inspired by [`ton-labs-types`](https://github.com/tonlabs/ton-labs-types),
but with much more emphasis on speed.

## Development

### How to bench

```bash
cargo bench dict
```

### How to miri check

```bash
# Add Miri component
rustup +nightly component add miri

# Run all tests with Miri
cargo +nightly miri test
```

### How to fuzz

```bash
# Install fuzzer
cargo install cargo-fuzz

# Run any of the fuzzer targets
cargo +nightly fuzz run boc_arc_decode_pair -j 12
cargo +nightly fuzz run boc_arc_decode -j 12
cargo +nightly fuzz run boc_rc_decode_pair -j 12
cargo +nightly fuzz run boc_rc_decode -j 12
```

## Contributing

We welcome contributions to the project! If you notice any issues or errors, feel free to open an issue or submit a pull request.

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
