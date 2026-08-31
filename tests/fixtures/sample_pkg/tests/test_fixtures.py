"""Tests that reach their subject only through a fixture.

Nothing here mentions :func:`sample_pkg.service.describe`. The only route from
a change to ``describe`` to these tests runs through ``tests/conftest.py``'s
``described`` fixture — which is exactly the edge `gerenuk run` has to
reconstruct, since `tyf` dead-ends at the fixture's definition.
"""

from __future__ import annotations

from sample_pkg.models import Shelter


def test_described_marks_the_senior(described: list[str]) -> None:
    assert "Bruno (dog), 11y — senior" in described, "the senior marker crosses the fixture"


def test_shelter_holds_the_demo_animals(shelter: Shelter) -> None:
    assert len(shelter.animals) == 3, "the demo shelter has three residents"
