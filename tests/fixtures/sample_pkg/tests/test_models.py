"""Tests for :mod:`sample_pkg.models`."""

from sample_pkg.models import Animal, Shelter


def test_is_senior_uses_an_inclusive_threshold() -> None:
    assert Animal("Bruno", "dog", 8).is_senior(), "8 years should already count as senior"
    assert not Animal("Pip", "cat", 7).is_senior(), "7 years should not count as senior"


def test_label_reads_as_name_then_species() -> None:
    assert Animal("Nala", "cat", 3).label() == "Nala (cat)"


def test_add_appends_in_order() -> None:
    shelter = Shelter(name="Test House")
    first = Animal("Nala", "cat", 3)
    second = Animal("Bruno", "dog", 11)
    shelter.add(first)
    shelter.add(second)
    assert shelter.animals == [first, second], "animals should keep insertion order"


def test_species_deduplicates() -> None:
    shelter = Shelter(name="Test House")
    shelter.add(Animal("Nala", "cat", 3))
    shelter.add(Animal("Pip", "cat", 1))
    shelter.add(Animal("Bruno", "dog", 11))
    assert shelter.species() == {"cat", "dog"}, "each species should appear once"


def test_a_new_shelter_starts_empty() -> None:
    assert Shelter(name="Empty").animals == [], "default_factory must give a fresh empty list"
    assert Shelter(name="Empty").species() == set(), "an empty shelter houses no species"
