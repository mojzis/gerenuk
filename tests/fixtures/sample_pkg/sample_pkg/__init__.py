"""A small, deliberately uneven package used as a gerenuk test fixture.

The shape matters: `service.py` contains symbols in each state gerenuk cares
about — used from production, used only from tests, and used nowhere at all.
Changing the reference counts here will change the integration expectations in
`tests/audit.rs`.
"""

from sample_pkg.models import Animal, Shelter
from sample_pkg.service import ShelterService, describe

__all__ = ["Animal", "Shelter", "ShelterService", "describe"]
