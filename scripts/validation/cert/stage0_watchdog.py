#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Independent watchdog loop for an approved provider adapter.

The caller must run this loop in a separate failure domain from the rented box,
provide an independently approved provider adapter and ensure its process stays
alive. This module does not provision a box, authenticate an adapter, or itself
establish that a real backstop is armed. No credentials belong in receipts.
"""
import time

import stage0_backstop


def watch(scope, provider, state, *, clock=time.time, pause=time.sleep, interval=3):
    """Arm before create; delete only the exact scoped resource when triggered.

    Keep monitoring if create is still in flight or a provider operation fails.
    Two distinct absent inventories after create closure are required for clean.
    State and provider are supplied by an independent host-specific adapter.
    """
    if type(interval) not in (int, float) or not 0 < interval <= 30:
        raise ValueError('invalid watchdog interval')
    stage0_backstop.check_available(scope, provider.inventory, clock())
    state.armed(scope)
    absent = 0
    while True:
        now = clock()
        stop = state.stop_requested()
        if now < scope.deadline_epoch and not stop:
            pause(min(interval, max(0.0, scope.deadline_epoch - now)))
            continue
        try:
            closed = state.creation_closed()
            absent_once = stage0_backstop.enforce(scope, provider.inventory, provider.delete,
                resource_id=state.resource_id(), creation_closed=closed, stop_requested=stop, now=now)
            absent = absent + 1 if absent_once else 0
            if absent >= 2:
                state.cleaned(scope)
                return
        except (OSError, ValueError, RuntimeError) as exc:
            absent = 0
            # Report only a bounded type, never provider response bodies/tokens.
            state.error(type(exc).__name__)
        pause(interval)
