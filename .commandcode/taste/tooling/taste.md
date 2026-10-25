# Taste — Tooling

- Runs Python test/lint suites inside the project's Docker `test` service when the host lacks the toolchain (e.g. `uv` not installed), rather than installing host dependencies. Confidence: 0.8
- Parallelizes independent verification (Docker pytest, cargo fmt/clippy/test, Vitest) and runs long suites in the background instead of serially. Confidence: 0.8
- Uses the repo's own lint/format gates as the definition of "done" (ruff check + format, cargo fmt/clippy `-D warnings`, ESLint, Prettier, `git diff --check`) and re-runs them after each refactor. Confidence: 0.85
- Validates edited CI workflow YAML and JS scripts immediately with a parser (`yaml.safe_load`, `node --check`) instead of waiting for CI. Confidence: 0.8
- Keeps a local `upstream` remote and fetches tags so release diffs can be computed against the last published tag (`git diff v0.3.5..HEAD`) for release notes. Confidence: 0.75
- Compares against the previous released version's source to detect regressions dropped during a refactor (e.g. `git show v0.3.5:<file>`) before rewriting behavior. Confidence: 0.75
