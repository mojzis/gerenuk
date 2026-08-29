"""Behaviour over :mod:`sample_pkg.models`.

Reference states, which the gerenuk fixtures assert on:

* ``describe`` — used from ``cli.py`` and ``pipelines.py``.
* ``ShelterService.summary`` — used from ``cli.py``.
* ``ShelterService.seniors`` — used only from ``tests/test_service.py``.
* ``legacy_export`` — used nowhere.
"""

from __future__ import annotations

from sample_pkg.models import Animal, Shelter


def describe(animal: Animal) -> str:
    """One-line description of an animal, marking seniors."""
    suffix = " — senior" if animal.is_senior() else ""
    return f"{animal.label()}, {animal.age_years}y{suffix}"


class ShelterService:
    """Read-only queries over a :class:`~sample_pkg.models.Shelter`."""

    def __init__(self, shelter: Shelter) -> None:
        self.shelter = shelter

    def summary(self) -> str:
        """Headline counts for the shelter."""
        count = len(self.shelter.animals)
        species = len(self.shelter.species())
        return f"{self.shelter.name}: {count} animal(s), {species} species"

    def seniors(self) -> list[Animal]:
        """Animals old enough to be flagged as senior."""
        return [a for a in self.shelter.animals if a.is_senior()]

    def _sorted_by_age(self) -> list[Animal]:
        """Private helper — gerenuk skips underscore-prefixed symbols."""
        return sorted(self.shelter.animals, key=lambda a: a.age_years)


def legacy_export(shelter: Shelter) -> str:
    """Old CSV export. Nothing calls this any more."""
    rows = [f"{a.name},{a.species},{a.age_years}" for a in shelter.animals]
    return "\n".join(rows)
