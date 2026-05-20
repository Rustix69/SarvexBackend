SHELL := /usr/bin/env bash

ENV_FILE ?= .env.example
COMPOSE := docker compose --env-file $(ENV_FILE)

.PHONY: help build test run stop reset logs proto migrate migrate-down seed

help:
	@echo "Targets:"
	@echo "  make build         - Build all Milestone 3 Go service skeletons"
	@echo "  make test          - Run Milestone 3 Go tests"
	@echo "  make run           - Start full Milestone 3 compose stack"
	@echo "  make stop          - Stop compose stack"
	@echo "  make reset         - Recreate compose stack volumes"
	@echo "  make logs          - Tail compose logs"
	@echo "  make proto         - Run proto generation script"
	@echo "  make migrate       - Placeholder migration entrypoint"
	@echo "  make migrate-down  - Placeholder migration rollback entrypoint"
	@echo "  make seed          - Placeholder seed entrypoint"

build:
	@go build ./...
	@echo "[build] Milestone 3 skeleton binaries compile."

test:
	@go test ./...
	@echo "[test] Milestone 3 skeleton tests passed."

run:
	@$(COMPOSE) up -d --build
	@echo "[run] Milestone 3 stack is up."

stop:
	@$(COMPOSE) down
	@echo "[stop] Milestone 0 infrastructure is down."

reset:
	@$(COMPOSE) down -v
	@$(COMPOSE) up -d postgres nats redis
	@echo "[reset] Milestone 0 infrastructure recreated."

logs:
	@$(COMPOSE) logs -f --tail=200 postgres nats redis

proto:
	@./scripts/proto-gen.sh

migrate:
	@./scripts/migrate.sh up

migrate-down:
	@./scripts/migrate.sh down 1

seed:
	@./scripts/seed-demo-data.sh
