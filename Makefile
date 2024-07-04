include .make/env.mk
export

SOURCE_PATH ?= $(PWD)
RUST_IMAGE_NAME     ?= ghcr.io/arrow-air/tools/arrow-rust
RUST_IMAGE_TAG      ?= 1.2
DOCKER_IMAGE_NAME   ?= sim-carrier
CARGO_MANIFEST_PATH ?= Cargo.toml
CARGO_INCREMENTAL   ?= 1
RUSTC_BOOTSTRAP     ?= 0
RELEASE_TARGET      ?= x86_64-unknown-linux-musl
PUBLISH_DRY_RUN     ?= 1
OUTPUTS_PATH        ?= $(SOURCE_PATH)/out
ADDITIONAL_OPT      ?=

ifeq ("$(CARGO_MANIFEST_PATH)", "")
cargo_run = echo "$(BOLD)$(YELLOW)No Cargo.toml found in any of the subdirectories, skipping cargo check...$(SGR0)"
else
cargo_run = docker run \
	--name=$(DOCKER_NAME) \
	--rm \
	--user `id -u`:`id -g` \
	--workdir=/usr/src/app \
	$(ADDITIONAL_OPT) \
	-v "$(SOURCE_PATH)/:/usr/src/app" \
	-v "$(SOURCE_PATH)/.cargo/registry:/usr/local/cargo/registry" \
	-e CARGO_INCREMENTAL=$(CARGO_INCREMENTAL) \
	-e RUSTC_BOOTSTRAP=$(RUSTC_BOOTSTRAP) \
	-t $(RUST_IMAGE_NAME):$(RUST_IMAGE_TAG) \
	cargo $(1) --manifest-path "$(CARGO_MANIFEST_PATH)" $(2)
endif

check-cargo-registry:
	if [ ! -d "$(SOURCE_PATH)/.cargo/registry" ]; then mkdir -p "$(SOURCE_PATH)/.cargo/registry" ; fi
check-logs-dir:
	if [ ! -d "$(SOURCE_PATH)/logs" ]; then mkdir -p "$(SOURCE_PATH)/logs" ; fi

setup:
	mkdir -p .px4 .qgc .gazebo .runtime .ccache
	chmod 0700 .runtime
	docker build -t px4-local -f Dockerfile-px4 .

build: check-cargo-registry
	@$(call cargo_run,build)

release: check-cargo-registry
	@$(call cargo_run,build --release --target $(RELEASE_TARGET))

test: check-cargo-registry
	@$(call cargo_run,test)

clean: check-cargo-registry
	rm -rf .ccache .simulator-gazebo
	@$(call cargo_run,clean)
