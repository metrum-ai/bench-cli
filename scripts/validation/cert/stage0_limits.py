#!/usr/bin/env python3
# Copyright (c) 2026 Metrum AI, Inc.
# SPDX-License-Identifier: Apache-2.0
"""Deterministic box duration and projected-exposure controls."""
import re
from decimal import Decimal
from fractions import Fraction


def duration_cap(planned_seconds):
    """Return the exact box hard cap: planned duration times 1.25."""
    if type(planned_seconds) is not int or planned_seconds <= 0:
        raise ValueError('planned seconds must be a positive integer')
    return Fraction(planned_seconds * 5, 4)


def amount(value):
    """Money must be supplied as a decimal string, never a binary float."""
    if not isinstance(value, str) or not re.fullmatch(r'(?:0|[1-9][0-9]{0,11})(?:\.[0-9]{1,8})?', value):
        raise ValueError('amount must be a bounded nonnegative decimal string')
    return Fraction(Decimal(value))


def backstop_exposure(hourly_rate, planned_seconds):
    """rate × box hard cap × 1.1; round up before setting a provider cap."""
    cap = duration_cap(planned_seconds)
    return amount(hourly_rate) * cap * Fraction(11, 10 * 3600)


def ceil_cents(value):
    """Round an exact nonnegative fraction up to integer minor units."""
    if not isinstance(value, Fraction) or value < 0:
        raise ValueError('invalid exact amount')
    return -(-(value * 100).numerator // (value * 100).denominator)


def require_below_stop(*, incurred, committed, proposed, stop):
    """Refuse creation at or above the projected stop; unknown is not zero."""
    total = amount(incurred) + amount(committed) + amount(proposed)
    threshold = amount(stop)
    if threshold <= 0 or total >= threshold:
        raise ValueError('projected exposure reaches the rental stop')
    return total
