"""Entry point that exercises the production-referenced symbols."""

from __future__ import annotations

from sample_pkg.models import Animal, Shelter
from sample_pkg.service import ShelterService, describe


def build_demo_shelter() -> Shelter:
    """A small shelter with one senior and two younger animals."""
    shelter = Shelter(name="Gerenuk House")
    shelter.add(Animal(name="Nala", species="cat", age_years=3))
    shelter.add(Animal(name="Bruno", species="dog", age_years=11))
    shelter.add(Animal(name="Pip", species="cat", age_years=1))
    return shelter


def main() -> None:
    """Print the shelter summary and one line per animal."""
    shelter = build_demo_shelter()
    service = ShelterService(shelter)
    print(service.summary())
    for animal in shelter.animals:
        print(describe(animal))


if __name__ == "__main__":
    main()
