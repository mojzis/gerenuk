"""Fixtures for the sample package's tests.

Three shapes gerenuk's selection has to see through, because pytest injects
fixtures by *name* and a type checker cannot follow a name:

* ``shelter`` is consumed by name from a test's parameter list;
* ``described`` requests ``shelter`` — a chained fixture;
* ``clock`` is ``autouse``, so every test in this directory consumes it without
  ever naming it.

``described`` calls :func:`sample_pkg.service.describe`. That is the edge that
matters: a change to ``describe`` reaches ``tests/test_fixtures.py`` only
through this file, and `tyf` cannot see it from the consuming side.

The shelter is built here rather than with
:func:`sample_pkg.cli.build_demo_shelter` on purpose. Importing ``cli`` would
put this file among the test importers of a module that already has a
module-level edge, and the whole subtree would be selected wholesale — burying
the fixture edge these fixtures exist to demonstrate.
"""

from __future__ import annotations

import pytest

from sample_pkg.models import Animal, Shelter
from sample_pkg.service import describe


@pytest.fixture
def shelter() -> Shelter:
    """A shelter with one senior and two younger animals, rebuilt per test."""
    shelter = Shelter(name="Gerenuk House")
    shelter.add(Animal(name="Nala", species="cat", age_years=3))
    shelter.add(Animal(name="Bruno", species="dog", age_years=11))
    shelter.add(Animal(name="Pip", species="cat", age_years=1))
    return shelter


@pytest.fixture
def described(shelter: Shelter) -> list[str]:
    """One description per animal — chained off ``shelter``."""
    return [describe(animal) for animal in shelter.animals]


@pytest.fixture(autouse=True)
def clock() -> int:
    """Autouse: in scope for every test here, named or not."""
    return 0
