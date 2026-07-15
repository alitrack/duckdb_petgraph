#!/usr/bin/env python3
"""Append DuckDB extension metadata footer to a shared library."""

import argparse
import shutil
import os
import platform
import sys


def detect_platform():
    """Auto-detect DuckDB platform string."""
    system = platform.system()
    machine = platform.machine()
    if system == "Linux":
        return "linux_amd64" if machine == "x86_64" else "linux_arm64"
    elif system == "Darwin":
        return "osx_arm64" if machine == "arm64" else "osx_amd64"
    elif system == "Windows":
        return "windows_amd64"
    return "linux_amd64"  # fallback


def start_signature():
    encoded_string = "".encode("ascii")
    encoded_string += int(0).to_bytes(1, "big")
    encoded_string += int(147).to_bytes(1, "big")
    encoded_string += int(4).to_bytes(1, "big")
    encoded_string += int(16).to_bytes(1, "big")
    encoded_string += b"duckdb_signature"
    encoded_string += int(128).to_bytes(1, "big")
    encoded_string += int(4).to_bytes(1, "big")
    return encoded_string


def padded_byte_string(s):
    encoded = s.encode("ascii")
    return encoded + b"\x00" * (32 - len(encoded))


def main():
    parser = argparse.ArgumentParser(description="Append DuckDB extension metadata")
    parser.add_argument("input", help="Input shared library (.so/.dylib)")
    parser.add_argument("-o", "--output", required=True, help="Output .duckdb_extension file")
    parser.add_argument("--platform", default=None, help="DuckDB platform (auto-detected if omitted)")
    parser.add_argument("--duckdb-version", default="v1.2.0", help="DuckDB version")
    parser.add_argument("--extension-version", default="0.1.0", help="Extension version")
    args = parser.parse_args()

    platform_str = args.platform or detect_platform()

    shutil.copyfile(args.input, args.output)

    with open(args.output, "ab") as f:
        f.write(start_signature())
        f.write(padded_byte_string(""))  # FIELD8 unused
        f.write(padded_byte_string(""))  # FIELD7 unused
        f.write(padded_byte_string(""))  # FIELD6 unused
        f.write(padded_byte_string("C_STRUCT"))  # FIELD5 abi_type
        f.write(padded_byte_string(args.extension_version))  # FIELD4
        f.write(padded_byte_string(args.duckdb_version))  # FIELD3
        f.write(padded_byte_string(platform_str))  # FIELD2
        f.write(padded_byte_string("4"))  # FIELD1 header signature
        f.write(b"\x00" * 256)  # signature space

    size_mb = os.path.getsize(args.output) / (1024 * 1024)
    print(f"Extension written: {args.output} ({size_mb:.1f} MB)")


if __name__ == "__main__":
    main()
