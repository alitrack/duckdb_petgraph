.PHONY: clean clean_all

PROJ_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))

EXTENSION_NAME=petgraph_ext

# Set to 1 to enable Unstable API
USE_UNSTABLE_C_API=0

# Target DuckDB C API version (not DuckDB release version)
TARGET_DUCKDB_VERSION=v1.2.0

# Skip tests for now — add SQLLogicTest files later
SKIP_TESTS=1

all: configure debug

# Include makefiles from DuckDB
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
