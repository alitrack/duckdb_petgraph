.PHONY: clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME=petgraph_ext

# Required: duckdb-rs relies on unstable C API functionality
USE_UNSTABLE_C_API=1

# Target DuckDB version (must match duckdb = "=1.10505.0" pin in Cargo.toml)
TARGET_DUCKDB_VERSION=v1.5.5

# Pin the test runner — use py3.6-compatible version (CI Docker lacks Python 3.12)
DUCKDB_TEST_VERSION=1.2.0

all: configure debug

# Include makefiles from DuckDB extension-ci-tools
include extension-ci-tools/makefiles/c_api_extensions/base.Makefile
include extension-ci-tools/makefiles/c_api_extensions/rust.Makefile

configure: venv platform extension_version

debug: build_extension_library_debug build_extension_with_metadata_debug
release: build_extension_library_release build_extension_with_metadata_release

test: test_debug
test_debug: test_extension_debug
test_release: test_extension_release

clean: clean_build clean_rust
clean_all: clean_configure clean
