---
name: tdd
description: "Drive a change test-first — a failing test (red) before the code that makes it pass (green), then refactor. Use when implementing at an agreed seam, or when another skill calls for test-first work."
---

# TDD

Write the test first. The loop is **red → green → refactor**, one small behaviour per turn, and you write production code only to make a failing test pass.

## The loop

1. **Red.** Write one small test for the next slice of behaviour and run it. Watch it fail for the reason you expect — a test that passes before the code exists is testing nothing.
2. **Green.** Write the least code that turns the test green, and run it. Green is the signal the step is done, not "it looks right".
3. **Refactor.** With the test green, clean up naming, duplication, and shape, re-running the test after each change so it stays green.

Repeat for the next slice. Keep the loop **tight**: small tests, fast runs, one reason to fail per test.

## Rules

- Test behaviour **through the interface** (the seam), so a refactor of the implementation keeps the test green.
- Run the single test file on every red and green; run the full suite once before handing off.
- Let the failing test drive the next line of production code — no code ahead of a test that needs it.
