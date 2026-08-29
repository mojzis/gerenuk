"""Tests for :mod:`sample_pkg.api`.

Two hops from ``describe``: gerenuk's impacted-tests fixture expects this file
to be selected through ``Enricher.run`` and ``enrich_endpoint``.
"""

from sample_pkg.api import enrich_endpoint
from sample_pkg.cli import build_demo_shelter


def test_enrich_endpoint_describes_every_animal() -> None:
    lines = enrich_endpoint(build_demo_shelter()).splitlines()
    assert len(lines) == 3, "one line per animal in the demo shelter"
    assert "Bruno (dog), 11y — senior" in lines, "the senior marker survives the chain"
