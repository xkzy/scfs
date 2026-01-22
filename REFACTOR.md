Refactor & Roadmap Summary

This document summarizes recent refactor actions and next steps to organize the codebase and make roadmap progress precise.

Actions performed:
- Moved integration drain->rebuild test to tests/ for cargo discovery.
- Ran rustfmt and cargo fix to address formatting and apply automatic fixes.
- Created this developer-facing summary and updated session plan.

Next steps:
- Stabilize flaky crash tests by converting timing/op-count tests to deterministic synchronization.
- Re-organize additional test helpers under tests/ and remove test-only modules from src/ where appropriate.
- Address remaining TODO/PARTIAL markers prioritized by roadmap phase.

Notes:
- Keep changes minimal and surgical. Update plan.md and session-state after each stabilization.
