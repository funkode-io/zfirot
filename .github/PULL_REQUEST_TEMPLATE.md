<!-- PR title MUST be a Conventional Commit — the squash merge derives the
     commit message from it. e.g. feat(presentation): …  fix(domain): …
     Add the issue number too: feat(presentation): render linked PRs (#79)

     Delete every <!-- --> comment before opening the PR: they render
     invisibly on the page but survive into the squash-merge commit message,
     and CI fails while any remain. -->

## Metadata

<!-- Link the issue so it auto-closes on merge (only fires when this PR targets
     `main`; a stacked PR auto-closes once it retargets). If there is genuinely
     no issue (e.g. a build fix), replace the line with: Closes: none — <reason> -->
Closes #

<!-- Stacked PR? Uncomment and point at the base PR:
> Stacked on #<PR>. Review the incremental diff; base retargets to `main` when it merges.
-->

## What changed

<!-- Briefly map each layer you touched; delete the lines that don't apply. -->
- **domain:**
- **application:**
- **infrastructure:**
- **presentation:**

## Acceptance criteria

<!-- Copy the issue's acceptance criteria verbatim; tick each as you meet it. -->
- [ ]
