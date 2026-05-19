SHELL := /usr/bin/env bash

ENV_FILE ?= .env.example
COMPOSE := docker compose --env-file $(ENV_FILE)

.PHONY: help build test run stop reset logs proto migrate migrate-down seed

help:
	@echo "Targets:"
	@echo "  make build         - Milestone 0 scaffold build placeholder"
	@echo "  make test          - Milestone 0 scaffold test placeholder"
	@echo "  make run           - Start Milestone 0 infrastructure (postgres, nats, redis)"
	@echo "  make stop          - Stop Milestone 0 infrastructure"
	@echo "  make reset         - Recreate Milestone 0 infrastructure volumes"
	@echo "  make logs          - Tail infrastructure logs"
	@echo "  make proto         - Run proto generation script"
	@echo "  make migrate       - Placeholder migration entrypoint"
	@echo "  make migrate-down  - Placeholder migration rollback entrypoint"
	@echo "  make seed          - Placeholder seed entrypoint"

build:
	@echo "[build] Milestone 0 scaffold ready; service builds start in next milestone."

test:
	@echo "[test] Milestone 0 scaffold ready; service tests start in next milestone."

run:
	@$(COMPOSE) up -d postgres nats redis
	@echo "[run] Milestone 0 infrastructure is up."

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
