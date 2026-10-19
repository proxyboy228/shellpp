#!/usr/bin/env python3
# Copyright Shell++ contributors. SPDX-License-Identifier: Apache-2.0
"""Runs the reference script of docs/storage-format.md, section 14.3, unchanged.

    run_reference_script.py <storage-format.md> <copy of shellpp.db> password|recovery

The script is extracted from the Markdown (the first ```python block after the
"### 14.3 Reference script" heading), compiled, and run with getpass.getpass
replaced by a function that returns the environment variable SHELLPP_KAT_SECRET,
so no secret is passed on the command line. It prints one payload per line.

Exit codes: 0 success, 2 the script could not be extracted or compiled,
3 the optional packages argon2-cffi or PyNaCl are not installed (skip).
Used by crates/core/tests/kat_manual_decrypt.rs.
"""

import getpass
import os
import re
import sys


def extract(markdown: str) -> str:
    start = markdown.find("### 14.3 Reference script")
    if start < 0:
        raise ValueError("section 14.3 not found")
    match = re.search(r"```python\n(.*?)\n```", markdown[start:], re.S)
    if not match:
        raise ValueError("no python block in section 14.3")
    return match.group(1) + "\n"


def main() -> int:
    md_path, db_path, mode = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(md_path, encoding="utf-8") as f:
        source = extract(f.read())
    try:
        code = compile(source, "shellpp_read.py", "exec")
    except SyntaxError as e:
        print(f"reference script does not compile: {e}", file=sys.stderr)
        return 2
    try:
        import argon2.low_level  # noqa: F401
        import nacl.bindings  # noqa: F401
    except ImportError:
        print("argon2-cffi or PyNaCl not installed", file=sys.stderr)
        return 3
    secret = os.environ["SHELLPP_KAT_SECRET"]
    getpass.getpass = lambda prompt="": secret
    sys.stdout.reconfigure(encoding="utf-8")
    namespace = {"__name__": "shellpp_read"}
    exec(code, namespace)
    sys.argv = ["shellpp_read.py", db_path] + (["--recovery"] if mode == "recovery" else [])
    namespace["main"]()
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except ValueError as e:
        print(str(e), file=sys.stderr)
        sys.exit(2)
