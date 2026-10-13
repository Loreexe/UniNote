.PHONY: all build check test run fmt clippy clean

all: build

build:
	cargo build --workspace

check:
	cargo check --workspace

test:
	cargo test --workspace

run:
	cargo run -p uninote-app

fmt:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

clean:
	cargo clean
