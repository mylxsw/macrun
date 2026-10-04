.DEFAULT_GOAL := help
SHELL := /bin/sh

CARGO := ./scripts/cargo-local.sh
PYTHON ?= python3
DOCKER ?= docker
PROFILE ?= release
TARGET_DIR ?= target
export CARGO_TARGET_DIR := $(abspath $(TARGET_DIR))
ifeq ($(PROFILE),release)
CARGO_FLAGS := --release
else ifeq ($(PROFILE),debug)
CARGO_FLAGS :=
else
$(error PROFILE must be release or debug)
endif
CARGO_BINARY := $(TARGET_DIR)/$(PROFILE)/macrun
DIST_DIR ?= dist
NATIVE_PLATFORM := $(shell uname -s | tr A-Z a-z)-$(shell uname -m | sed -e s/x86_64/amd64/ -e s/aarch64/arm64/)
CLIENT_DIST := $(DIST_DIR)/$(if $(filter debug,$(PROFILE)),debug/,)$(NATIVE_PLATFORM)
BINARY := $(CLIENT_DIST)/macrun
PLATFORM ?= linux/amd64
SERVER_DIST ?= $(DIST_DIR)/$(subst /,-,$(PLATFORM))
IMAGE ?= macrun:generic-demo
PREFIX ?= $(HOME)/.local
SOCKET ?= /tmp/macrun.sock
DATA ?= .local/server
WORKER_DATA ?= .local/worker
LISTEN ?= 0.0.0.0:7443
SERVER ?=
CERT ?= $(DATA)/cert.der
TOKEN_FILE ?= $(DATA)/token
WORKER_CONFIG ?= worker.toml
WORKSPACE ?= .
INTERVAL_MS ?= 1000

.PHONY: help deps doctor build build-client build-worker build-server server-image \
        check fmt fmt-check lint test smoke cross-smoke install init serve worker \
        status mcp sync sync-watch docker-check desktop-deps desktop-build desktop-dev

help: ## Show common commands and defaults
	@awk 'BEGIN {FS = ":.*## "; print "Usage: make <target> [NAME=value]\n"} /^[a-zA-Z_-]+:.*## / {printf "  %-16s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@printf '\nDefaults: PROFILE=%s PLATFORM=%s PREFIX=%s\n' '$(PROFILE)' '$(PLATFORM)' '$(PREFIX)'
	@printf 'One binary provides CLI, server, worker and MCP modes. build-client builds for the current OS.\n'

deps: ## Install missing Rust, rustfmt/clippy, and fetch locked dependencies
	@./scripts/deps.sh

doctor: ## Check native build tools; report optional Python and Docker availability
	@$(CARGO) --version
	@command -v cc >/dev/null || { echo 'Missing C compiler: install Xcode CLI tools or build-essential'; exit 1; }
	@$(CARGO) fmt --version
	@$(CARGO) clippy --version
	@$(PYTHON) --version 2>/dev/null || echo 'Python 3 missing (needed for smoke tests)'
	@$(DOCKER) info >/dev/null 2>&1 && echo 'Docker: ready' || echo 'Docker unavailable (needed for build-server/server-image/cross-smoke)'

build: ## Build the current OS binary (release by default; PROFILE=debug supported)
	$(CARGO) build --locked $(CARGO_FLAGS)
	@mkdir -p '$(CLIENT_DIST)'
	cp '$(CARGO_BINARY)' '$(BINARY)'
	@printf 'Built: %s\n' '$(BINARY)'

build-client: build ## Build native CLI / Mac client (run on Mac for a Mac binary)

build-worker: build ## Build native worker (same binary as client and server)

desktop-deps: ## Prepare macOS desktop dependencies (cached npm install, Rust if missing)
	@./scripts/desktop-deps.sh

desktop-build: desktop-deps ## Build Macrun Desktop.app (PROFILE=debug for faster builds)
	@cd desktop && unset CARGO_TARGET_DIR && npm run desktop:build $(if $(filter debug,$(PROFILE)),-- --debug,)
	@printf '\nApplication: %s/desktop/src-tauri/target/$(PROFILE)/bundle/macos/Macrun Desktop.app\n' '$(CURDIR)'

desktop-dev: desktop-deps ## Run desktop development app with frontend hot reload; Ctrl+C to stop
	@cd desktop && unset CARGO_TARGET_DIR && npm run desktop:dev

docker-check:
	@$(DOCKER) info >/dev/null 2>&1 || { echo 'Start Docker Desktop / OrbStack / Docker Engine first'; exit 1; }

build-server: docker-check ## Export Linux release binary via Docker (default linux/amd64)
	$(DOCKER) build --platform '$(PLATFORM)' --target binary --output 'type=local,dest=$(SERVER_DIST)' .
	@printf 'Linux server/CLI binary: %s/macrun\n' '$(SERVER_DIST)'

server-image: docker-check ## Build a runnable Linux server image; override IMAGE / PLATFORM
	$(DOCKER) build --platform '$(PLATFORM)' -t '$(IMAGE)' .

fmt: ## Format Rust sources
	$(CARGO) fmt

fmt-check: ## Check formatting without changing files
	$(CARGO) fmt --check

lint: ## Run clippy and reject warnings
	$(CARGO) clippy --locked --all-targets -- -D warnings

test: ## Run Rust tests
	$(CARGO) test --locked

check: fmt-check lint test ## Run format, lint and unit/integration tests

smoke: build ## Exercise real local QUIC, tasks, files, sync and fixture MCP
	$(PYTHON) scripts/smoke.py '$(BINARY)'

cross-smoke: build server-image ## On Mac: verify Linux amd64 server -> Mac worker (fixture GUI)
	@test "$$(uname -s)" = Darwin || { echo 'cross-smoke requires a Mac host'; exit 1; }
	@test '$(PLATFORM)' = linux/amd64 || { echo 'cross-smoke currently requires PLATFORM=linux/amd64'; exit 1; }
	@mkdir -p .local
	$(PYTHON) scripts/cross-smoke.py '$(BINARY)' '$(IMAGE)'

install: build ## Install native binary into PREFIX/bin (default ~/.local/bin)
	@mkdir -p '$(PREFIX)/bin'
	install -m 755 '$(BINARY)' '$(PREFIX)/bin/macrun'
	@printf 'Installed: %s/bin/macrun\n' '$(PREFIX)'

init: build ## Create server identity under DATA; refuses to overwrite an existing identity
	'$(BINARY)' init --data '$(DATA)'

serve: build ## Run server in foreground; configure DATA / LISTEN / SOCKET
	'$(BINARY)' --socket '$(SOCKET)' serve --listen '$(LISTEN)' --data '$(DATA)'

worker: build ## Run worker in foreground; requires SERVER and valid CERT / TOKEN_FILE
	@test -n '$(SERVER)' || { echo 'Usage: make worker SERVER=IP:7443 CERT=... TOKEN_FILE=... [WORKER_CONFIG=...]'; exit 1; }
	'$(BINARY)' worker --server '$(SERVER)' --cert '$(CERT)' --token-file '$(TOKEN_FILE)' --data '$(WORKER_DATA)' $(if $(WORKER_CONFIG),--config '$(WORKER_CONFIG)',)

status: build ## Inspect server/worker connection
	'$(BINARY)' --socket '$(SOCKET)' status

mcp: ## Start stdio MCP using an already built binary (no build output on stdout)
	@'$(BINARY)' --socket '$(SOCKET)' --workspace '$(WORKSPACE)' mcp

sync: build ## Synchronize WORKSPACE once and wait for completion
	'$(BINARY)' --socket '$(SOCKET)' --workspace '$(WORKSPACE)' sync

sync-watch: build ## Continuously synchronize WORKSPACE; foreground process
	'$(BINARY)' --socket '$(SOCKET)' --workspace '$(WORKSPACE)' sync --watch --interval-ms '$(INTERVAL_MS)'
