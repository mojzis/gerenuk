.PHONY: review fmt-check lint test test-fixture test-impact audit deny coverage mutants review-quick docs docs-serve

# Full review — run before pushing or merging
review: fmt-check lint test test-fixture audit deny
	@echo ""
	@echo "✅ All review checks passed"

# Quick review — skip slower network checks
review-quick: fmt-check lint test test-fixture
	@echo ""
	@echo "✅ Quick review passed"

fmt-check:
	@echo "📐 Checking formatting..."
	@cargo fmt --all -- --check

lint:
	@echo "🔍 Running clippy..."
	@cargo clippy --all-targets --all-features -- -D warnings

test:
	@echo "🧪 Running tests..."
	@if command -v cargo-nextest > /dev/null 2>&1; then \
		cargo nextest run --all-features; \
	else \
		cargo test --all-features; \
	fi

audit:
	@echo "🔒 Running security audit..."
	@if command -v cargo-audit > /dev/null 2>&1; then \
		cargo audit; \
	else \
		echo "⚠️  cargo-audit not installed. Run: cargo install cargo-audit"; \
	fi

deny:
	@echo "🚫 Checking dependency policies..."
	@if command -v cargo-deny > /dev/null 2>&1; then \
		cargo deny check; \
	else \
		echo "⚠️  cargo-deny not installed. Run: cargo install cargo-deny"; \
	fi

coverage:
	@echo "📊 Generating coverage report..."
	@if command -v cargo-llvm-cov > /dev/null 2>&1; then \
		cargo llvm-cov --all-features --workspace --html; \
		echo "Report: target/llvm-cov/html/index.html"; \
	else \
		echo "⚠️  cargo-llvm-cov not installed. Run: cargo install cargo-llvm-cov"; \
	fi

mutants:
	@echo "🧬 Running mutation testing on recent changes..."
	@if command -v cargo-mutants > /dev/null 2>&1; then \
		cargo mutants --in-diff HEAD~1..HEAD; \
	else \
		echo "⚠️  cargo-mutants not installed. Run: cargo install cargo-mutants"; \
	fi

# Run the fixture Python package's own pytest suite
test-fixture:
	@echo "🐍 Running fixture package tests..."
	@cd tests/fixtures/sample_pkg && uv run --with pytest --with hatchling python -m pytest -q

# Drive `impacted-tests` against the fixture with a REAL tyf, outside cargo test
# (which stubs it). Skips cleanly when tyf is not installed.
test-impact:
	@cargo build --quiet
	@bash scripts/impact-smoke.sh

# Build the mdBook site plus llms.txt into docs/book/html
docs:
	@echo "📚 Building docs..."
	@bash docs/gen-version.sh
	@mdbook build docs
	@bash docs/generate-llms-txt.sh

# Serve the docs locally with live reload
docs-serve:
	@mdbook serve docs --open
