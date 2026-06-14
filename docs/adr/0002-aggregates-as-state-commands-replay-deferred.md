# Aggregates as state + commands, replay framework deferred

We model locally-owned aggregates (e.g. the tracked-repo set) as plain serde
state structs plus command types, and we **park the replay framework for now**.
In replay an aggregate is defined by its state and event sourcing lives in the
infrastructure layer, so writing aggregates as state + commands keeps the door
open to adopt replay later (via snapshots) without changing the domain.

This also sidesteps a trap: GitHub exposes no timeline history for the two
relationships the board depends on (parent sub-issues and blocked-by
dependencies), so translating GraphQL results into a faithful event stream is
impossible without fabricating poll-time timestamps. Keeping the domain
state-based avoids pretending we have history we don't.

## Consequences

- `Slice` state (Ready / WIP / Blocked / Done) is a pure derivation over current
  GitHub data, never an event.
- When a backend arrives, replay can be introduced in infrastructure; domain
  state and commands stay as-is.
