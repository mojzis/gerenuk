"""A small, deliberately uneven package used as a gerenuk test fixture.

The shape matters twice over. `service.py` contains symbols in each state
`gerenuk audit` cares about — used from production, used only from tests, and
used nowhere at all. `pipelines.py` and `api.py` then add a two-hop chain out of
`describe` for `gerenuk impacted-tests` to walk, plus one registry-decorated
dead end. `tests/conftest.py` adds the fourth shape: a chain that reaches its
tests only through a pytest fixture, which no type checker can follow and which
`gerenuk run` therefore has to reconstruct by name.

Changing the reference counts here will change the integration expectations in
`tests/audit.rs` and the assertions in `scripts/impact-smoke.sh` and
`scripts/run-smoke.sh`.
"""

from sample_pkg.models import Animal, Shelter
from sample_pkg.service import ShelterService, describe

__all__ = ["Animal", "Shelter", "ShelterService", "describe"]
