.PHONY: build test lint fmt install clean

build:
	cargo build --release

test:
	cargo test

lint:
	cargo clippy --workspace -- -D warnings

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

install:
	cargo install --path crates/reckoner-cli

clean:
	cargo clean
