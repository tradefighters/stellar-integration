.PHONY: default build test check fmt clean

default: build

## Build every contract to WASM (target/wasm32v1-none/release/*.wasm).
build:
	stellar contract build

## Host-side unit tests.
test:
	cargo test

## What CI runs.
check:
	cargo fmt --all --check
	cargo clippy --all-targets -- -D warnings
	cargo test
	stellar contract build

fmt:
	cargo fmt --all

clean:
	cargo clean
