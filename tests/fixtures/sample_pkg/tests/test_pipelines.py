"""Tests for :mod:`sample_pkg.pipelines`."""

from sample_pkg.models import Animal, Shelter
from sample_pkg.pipelines import Enricher, normalise_species


def test_run_describes_animals_in_order() -> None:
    shelter = Shelter(name="Test House")
    shelter.add(Animal("Nala", "cat", 3))
    shelter.add(Animal("Bruno", "dog", 11))
    assert Enricher(shelter).run() == [
        "Nala (cat), 3y",
        "Bruno (dog), 11y — senior",
    ]


def test_run_on_an_empty_shelter_returns_nothing() -> None:
    assert Enricher(Shelter(name="Empty")).run() == []


def test_normalise_species_lowercases() -> None:
    normalised = normalise_species(Animal("Nala", "Cat", 3))
    assert normalised.species == "cat", "the registry decorator returns the function unchanged"
