.PHONY: fmt check test build phase01-up phase01-down

fmt:
	cargo fmt --all

check:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --workspace

phase01-up:
	docker compose -f docker-compose.yml up --build

phase01-down:
	docker compose -f docker-compose.yml down -v
