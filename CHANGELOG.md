# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The semantic versioning guarantees cover the interface to the substrate runtime which
includes this pallet as a dependency. This module will also add storage migrations whenever
changes require it. Stability with regard to offchain tooling is explicitly excluded from
this guarantee: For example adding a new field to an in-storage data structure will require
changes to frontends to properly display it. However, those changes will still be regarded
as a minor version bump.

The interface provided to smart contracts will adhere to semver with one exception: Even
major version bumps will be backwards compatible with regard to already deployed contracts.
In other words: Upgrading this pallet will not break pre-existing contracts.

## [Unreleased]

### New

- Add new gas metering method: mutable global + local gas function
[#34](https://github.com/paritytech/wasm-instrument/pull/34)

## [v0.3.0]

### Changed

- Use 64bit arithmetic for per-block gas counter
[#30](https://github.com/paritytech/wasm-instrument/pull/30)

## [v0.2.0] 2022-06-06

### Changed

- Adjust debug information (if already parsed) when injecting gas metering
[#16](https://github.com/paritytech/wasm-instrument/pull/16)

## [v0.1.1] 2022-01-18

### Fixed

- Stack metering disregarded the activiation frame.
[#2](https://github.com/paritytech/wasm-instrument/pull/2)

## [v0.1.0] 2022-01-11

### Changed

- Created from [pwasm-utils](https://github.com/paritytech/wasm-utils) by removing unused code and cleaning up docs.
