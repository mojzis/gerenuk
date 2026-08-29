"""The far end of the chain: what a caller two hops from ``describe`` looks like."""

from __future__ import annotations

from sample_pkg.models import Shelter
from sample_pkg.pipelines import Enricher


def enrich_endpoint(shelter: Shelter) -> str:
    """One description per line, for a shelter."""
    return "\n".join(Enricher(shelter).run())
