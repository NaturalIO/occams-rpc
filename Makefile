# filter out target and keep the rest as args
PRIMARY_TARGET := $(firstword $(MAKECMDGOALS))
ARGS := $(filter-out $(PRIMARY_TARGET), $(MAKECMDGOALS))

.PHONY: git-hooks
git-hooks:
	git config core.hooksPath ./git-hooks;

.PHONY: init
init: git-hooks

.PHONY: fmt
fmt: init
	cargo fmt

.PHONY: test
all-test: test-core test-codec test-stream-macros test-api-macros test
	echo run all tests

.PHONY: test-core
test-core: init
	cargo check -p occams-rpc-core
	cargo test -p occams-rpc-core

.PHONY: test-codec
	cargo check -p occams-rpc-codec
	cargo test -p occams-rpc-codec

.PHONY: test-stream-macros
test-stream-macros: init
	cargo test -p occams-rpc-stream-macros -- --nocapture

.PHONY: test-api-macros
test-api-macros: init
	RUST_BACKTRACE=1 cargo test -p occams-rpc-api-macros -- --nocapture

# usage:
# make test-stream "test_normal --F tokio"
# make test-stream "test_normal --F smol"
# make test-stream - "--features smol"
.PHONY: test-basic
test: init
	@echo "Run integration tests"
	cargo test -p occams-rpc-test ${ARGS} -- --nocapture --test-threads=1
	@echo "Done"

pressure: init
	cargo test -p occams-rpc-test ${ARGS} --release bench -- --nocapture --test-threads=1

.PHONY: build
build: init
	cargo build -p occams-rpc-core
	cargo build -p occams-rpc-tokio
	cargo build -p occams-rpc-smol
	cargo build -p occams-rpc-stream-macros
	cargo build -p occams-rpc-stream
	cargo build -p occams-rpc-tcp
#	cargo build -p occams-rpc-test
	cargo build

.DEFAULT_GOAL = build

# Target name % means that it is a rule that matches anything, @: is a recipe;
# the : means do nothing
%:
	@:
