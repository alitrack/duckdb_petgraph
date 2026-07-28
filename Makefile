.PHONY: clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME=petgraph_ext
USE_UNSTABLE_C_API=1
TARGET_DUCKDB_VERSION=v1.5.5

# Force configure inside Docker
CONFIGURE_CI_STEP=minimal_configure

all: configure debug

include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

configure: minimal_configure

# Minimal configure: write files without importing duckdb (compat with CI Docker py3.6)
minimal_configure:
	@mkdir -p configure
	@echo "$${DUCKDB_PLATFORM:-$$(uname -s)_$$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/')}" | tr '[:upper:]' '[:lower:]' > configure/platform.txt
	@echo "$${EXTENSION_VERSION:-v0.5.0}" > configure/extension_version.txt
	@$(PYTHON_BIN) -m venv --without-pip configure/venv 2>/dev/null || $(PYTHON_BIN) -m venv configure/venv 2>/dev/null || \
	(echo "#!/bin/sh" > configure/venv/bin/python3 && chmod +x configure/venv/bin/python3 && mkdir -p configure/venv/bin && which python3 > /dev/null && ln -sf $$(which python3) configure/venv/bin/python3)
	@echo "Configure done: $$(cat configure/platform.txt) $$(cat configure/extension_version.txt)"

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_debug
test_debug: test_extension_debug
test_release: test_extension_release

clean: clean_build clean_rust
clean_all: clean_configure clean
