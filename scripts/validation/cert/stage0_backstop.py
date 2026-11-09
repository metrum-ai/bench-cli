#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Name- and ID-scoped provider teardown logic for an independent watchdog.

This module does not create resources or start a watchdog. An external
independent process must call it against the real provider, persist its arm
receipt and remain alive across controller/box failure. A local successful unit
test is not proof that a real provider backstop is armed.
"""
import math
import re
from dataclasses import dataclass

SCOPE = re.compile(r'bench-cli-v190-stage0-[a-f0-9]{24}\Z')


@dataclass(frozen=True)
class BackstopScope:
    name: str
    deadline_epoch: float

    def __post_init__(self):
        if (not isinstance(self.name, str) or not SCOPE.fullmatch(self.name)
                or type(self.deadline_epoch) not in (int, float) or not math.isfinite(self.deadline_epoch)
                or self.deadline_epoch <= 0):
            raise ValueError('invalid provider backstop scope/deadline')


def matching(resources, name):
    """Reject malformed/ambiguous inventories before any deletion."""
    if not isinstance(resources, list):
        raise ValueError('invalid provider inventory')
    ids = []
    for item in resources:
        if (not isinstance(item, dict) or not isinstance(item.get('id'), str)
                or not re.fullmatch(r'[A-Za-z0-9_-]+', item['id']) or not isinstance(item.get('name'), str)):
            raise ValueError('malformed provider inventory entry')
        if item['name'] == name:
            ids.append(item['id'])
    if len(ids) != len(set(ids)) or len(ids) > 1:
        raise ValueError('ambiguous scoped provider resources')
    return ids


def check_available(scope, inventory, now):
    """Provider enumeration and uniqueness check required before creation."""
    if type(now) not in (int, float) or not math.isfinite(now):
        raise ValueError('invalid current time')
    if now >= scope.deadline_epoch:
        raise ValueError('backstop deadline has elapsed')
    if matching(inventory(), scope.name):
        raise ValueError('scoped provider name already exists')


def enforce(scope, inventory, delete, *, resource_id, creation_closed, stop_requested, now):
    """Request scoped deletion or verify absence; one point-in-time step.

    Return False until a separate subsequent inventory also confirms absence;
    callers must perform two independent absent observations after creation is
    closed and persist them. Never delete by ID without an exact name match.
    """
    if (type(creation_closed) is not bool or type(stop_requested) is not bool
            or type(now) not in (int, float) or not math.isfinite(now)
            or (resource_id is not None and (not isinstance(resource_id, str)
                                             or not re.fullmatch(r'[A-Za-z0-9_-]+', resource_id)))):
        raise ValueError('invalid backstop state')
    if now < scope.deadline_epoch and not stop_requested:
        raise ValueError('early enforcement requires an explicit stop request')
    found = matching(inventory(), scope.name)
    if found:
        if resource_id is not None and found[0] != resource_id:
            raise ValueError('provider name/ID mismatch; refusing deletion')
        delete(found[0])
        return False
    if not creation_closed:
        # An in-flight create may still succeed after a timeout/deadline.
        return False
    return True
