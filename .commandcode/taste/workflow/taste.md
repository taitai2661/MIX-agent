# Taste — Workflow

- Judges release-readiness by evidence, not impression: inspect CI, run the actual test/lint/build gates, and record the outcome in `docs/verification.md` before declaring a release possible. Confidence: 0.85
- Requires verification records to separate what was really executed from what was only covered by unit tests or a successful build (real provider / real Docker / real device must be listed as unverified). Confidence: 0.8
- Wants a written, approved plan before implementation: numbered work items with concrete file paths/line targets, an explicit blocker list, a residual-risk section, and post-release verification steps, then tracked as separate tasks. Confidence: 0.8
- Investigates before answering (parallel explore subagents, git history/tags, CI and registry APIs) rather than asking the user for details the repo can answer. Confidence: 0.75
- Turns irreversible release decisions (publish target, version number, signing/notarization) into explicit multiple-choice questions with a recommended default before tagging or publishing. Confidence: 0.75
- Treats a red main branch found during release prep as in-scope: fixes the pre-existing failures instead of shipping around them. Confidence: 0.7
