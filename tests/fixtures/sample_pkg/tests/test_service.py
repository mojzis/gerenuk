"""Tests for :mod:`sample_pkg.service`.

``seniors`` is referenced only from here on purpose — gerenuk's integration
tests expect it to be reported as test-only.
"""

from sample_pkg.cli import build_demo_shelter
from sample_pkg.models import Animal, Shelter
from sample_pkg.service import ShelterService, describe


def test_describe_marks_seniors() -> None:
    assert describe(Animal("Bruno", "dog", 11)) == "Bruno (dog), 11y — senior"


def test_describe_leaves_young_animals_unmarked() -> None:
    assert describe(Animal("Pip", "cat", 1)) == "Pip (cat), 1y"


def test_summary_counts_animals_and_species() -> None:
    service = ShelterService(build_demo_shelter())
    assert service.summary() == "Gerenuk House: 3 animal(s), 2 species"


def test_summary_of_an_empty_shelter_reports_zeroes() -> None:
    service = ShelterService(Shelter(name="Empty"))
    assert service.summary() == "Empty: 0 animal(s), 0 species"


def test_seniors_returns_only_old_animals() -> None:
    service = ShelterService(build_demo_shelter())
    assert [a.name for a in service.seniors()] == ["Bruno"], "only Bruno is 8 or older"
