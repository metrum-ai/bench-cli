# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Promptfoo python assert: execute generated coding solutions in-process."""

from __future__ import annotations

import re
import textwrap
from typing import Any


def _strip_fences(src: str) -> str:
    src = src.strip()
    src = re.sub(r"^```(?:python)?\s*", "", src, flags=re.IGNORECASE)
    src = re.sub(r"\s*```$", "", src)
    return src.strip()


def get_assert(output: str, context: Any) -> dict[str, Any]:
    """Promptfoo python assertion entrypoint."""
    description = ""
    try:
        description = (context.get("test") or {}).get("description") or ""
    except Exception:
        description = ""

    code = _strip_fences(output or "")
    if not code:
        return {"pass": False, "score": 0, "reason": "empty code"}

    ns: dict[str, Any] = {}
    try:
        exec(textwrap.dedent(code), ns, ns)
    except Exception as exc:  # noqa: BLE001 - report to promptfoo
        return {"pass": False, "score": 0, "reason": f"exec failed: {exc}"}

    try:
        if description == "two_sum_indices":
            fn = ns.get("two_sum")
            if not callable(fn):
                return {"pass": False, "score": 0, "reason": "missing two_sum"}
            got = fn([2, 7, 11, 15], 9)
            ok = list(got) == [0, 1] or set(got) == {0, 1}
            return {
                "pass": bool(ok),
                "score": 1 if ok else 0,
                "reason": f"two_sum([2,7,11,15],9) -> {got}",
            }
        if description == "reverse_words":
            fn = ns.get("reverse_words")
            if not callable(fn):
                return {"pass": False, "score": 0, "reason": "missing reverse_words"}
            got = fn("the sky is blue")
            ok = got == "blue is sky the"
            return {
                "pass": bool(ok),
                "score": 1 if ok else 0,
                "reason": f"reverse_words(...) -> {got!r}",
            }
        return {"pass": False, "score": 0, "reason": f"unknown test {description}"}
    except Exception as exc:  # noqa: BLE001
        return {"pass": False, "score": 0, "reason": f"runtime failed: {exc}"}
