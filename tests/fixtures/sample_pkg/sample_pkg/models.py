"""Data types for the fixture package."""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Animal:
    """One resident of a shelter."""

    name: str
    species: str
    age_years: int

    def is_senior(self) -> bool:
        """Whether the animal counts as senior for adoption purposes."""
        return self.age_years >= 8

    def label(self) -> str:
        """Short human label, e.g. ``Nala (cat)``."""
        return f"{self.name} ({self.species})"


@dataclass
class Shelter:
    """A named collection of animals."""

    name: str
    animals: list[Animal] = field(default_factory=list)

    def add(self, animal: Animal) -> None:
        """Register an animal with this shelter."""
        self.animals.append(animal)

    def species(self) -> set[str]:
        """Distinct species currently housed."""
        return {animal.species for animal in self.animals}
