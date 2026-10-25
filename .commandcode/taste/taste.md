# Taste

## Communication

- Writes instructions in Japanese and expects Japanese replies, plans, and commit-style summaries (documentation touched by a change should also be updated in Japanese, with the English mirror kept in sync). Confidence: 0.7
- Gives short, terse directives and leaves investigation/design details to the assistant (e.g. one-line feature switch requests that require exploring the codebase first). Confidence: 0.6
- Asks blunt readiness questions ("リリースできるレベルかな？") and expects a verdict backed by a per-item status table (✅/⚠️/❌) plus the concrete blockers, not a narrative summary. Confidence: 0.8
