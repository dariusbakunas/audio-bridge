# Build targets for audio-hub workspace

CARGO ?= cargo
CROSS ?= cross
#
# NOTE: cross's Docker images are amd64-only. On Apple Silicon, we need to
# force emulation.
CROSS_CONTAINER_OPTS ?= --platform=linux/amd64
export CROSS_CONTAINER_OPTS
DOCKER_DEFAULT_PLATFORM ?= linux/amd64
export DOCKER_DEFAULT_PLATFORM
PROFILE ?= --release

CRATES := bridge audio-hub-server hub-cli
LINUX_X64_TARGET := x86_64-unknown-linux-gnu
LINUX_ARM64_TARGET := aarch64-unknown-linux-gnu

.PHONY: help build build-local build-linux-x64 build-linux-arm64 build-all clean

help:
	@echo "Targets:"
	@echo "  build / build-local   Build for host (${PROFILE})"
	@echo "  build-linux-x64       Cross-compile for ${LINUX_X64_TARGET}"
	@echo "  build-linux-arm64     Cross-compile for ${LINUX_ARM64_TARGET} (RPi 4/5 64-bit)"
	@echo "  build-all             Build host + both Linux targets"
	@echo "  clean                Remove build artifacts"

build: build-local

build-local:
	${CARGO} build ${PROFILE} $(foreach c,${CRATES},-p ${c})

build-linux-x64:
	${CROSS} build ${PROFILE} --target ${LINUX_X64_TARGET} $(foreach c,${CRATES},-p ${c})

build-linux-arm64:
	${CROSS} build ${PROFILE} --target ${LINUX_ARM64_TARGET} $(foreach c,${CRATES},-p ${c})

build-all: build-local build-linux-x64 build-linux-arm64

clean:
	${CARGO} clean
