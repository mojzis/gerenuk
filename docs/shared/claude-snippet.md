### `gerenuk` — test selection and dead code

- `gerenuk run -- -q` runs only the tests the working tree's diff impacts
  (`--dry-run` to inspect; `gerenuk impacted-tests` explains why).
- `gerenuk audit <file>` confirms a symbol vulture flagged is really unused;
  its `only tests reach it` findings are ones vulture cannot produce.
