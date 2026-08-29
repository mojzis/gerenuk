"""A minimal registry decorator.

Functions marked with it are invoked by a runner rather than called, which is
exactly the case `[tool.gerenuk] ignore-decorators` exists for: a change to one
has no callers worth chasing.
"""

from __future__ import annotations

from typing import Callable, TypeVar

F = TypeVar("F", bound=Callable[..., object])

REGISTERED: list[str] = []


def transformation(func: F) -> F:
    """Record ``func`` in the registry and return it unchanged."""
    REGISTERED.append(func.__name__)
    return func
