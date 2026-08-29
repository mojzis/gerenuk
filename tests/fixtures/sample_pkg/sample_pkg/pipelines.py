"""The middle of the chain `gerenuk impacted-tests` walks.

``Enricher.run`` calls :func:`sample_pkg.service.describe` and is itself called
from :mod:`sample_pkg.api`, whose tests are therefore two hops from a change to
``describe``. ``normalise_species`` is a registry function: a dead end.
"""

from __future__ import annotations

from sample_pkg import registry
from sample_pkg.models import Animal, Shelter
from sample_pkg.service import describe


class Enricher:
    """Turns a shelter into one description per animal."""

    def __init__(self, shelter: Shelter) -> None:
        self.shelter = shelter

    def run(self) -> list[str]:
        """Describe every animal in the shelter, in order."""
        return [describe(animal) for animal in self.shelter.animals]


@registry.transformation
def normalise_species(animal: Animal) -> Animal:
    """Lower-case an animal's species. Called by the registry runner only."""
    return Animal(
        name=animal.name, species=animal.species.lower(), age_years=animal.age_years
    )
