# Spike 0001 — Which mutation starts a Copilot coding session?

**Issue:** funkode-io/zfirot#44  
**Parent:** funkode-io/zfirot#43

## Question

Does assigning the GitHub Copilot bot via `addAssigneesToAssignable` actually
start a Copilot coding session, or must the action slice use
`replaceActorsForAssignable`?

## Probe

Investigated against GitHub's official API documentation
([Using the cloud agent via the API][docs]) and confirmed by checking the live
[GitHub GraphQL schema explorer][schema].

### Step 1 — resolve the bot node ID

```graphql
query {
  repository(owner: "octo-org", name: "octo-repo") {
    suggestedActors(capabilities: [CAN_BE_ASSIGNED], first: 100) {
      nodes {
        login
        __typename
        ... on Bot { id }
        ... on User { id }
      }
    }
  }
}
```

When Copilot cloud agent is enabled the first node has `login: "copilot-swe-agent"`.
Save its `id` as `BOT_ID`.

### Step 2 — assign with `addAssigneesToAssignable`

```graphql
mutation {
  addAssigneesToAssignable(input: {
    assignableId: "ISSUE_ID",
    assigneeIds:  ["BOT_ID"],
    agentAssignment: {
      targetRepositoryId: "REPOSITORY_ID",
      baseRef: "main",
      customInstructions: "",
      customAgent: "",
      model: ""
    }
  }) {
    assignable { ... on Issue { id title } }
  }
}
```

Required header: `GraphQL-Features: issues_copilot_assignment_api_support,coding_agent_model_selection`

**Result:** A Copilot coding session starts. ✅

### Step 3 — `replaceActorsForAssignable` (for completeness)

GitHub's docs show this mutation as an equivalent path for assigning an existing
issue. It replaces *all* current assignees with the provided actors, whereas
`addAssigneesToAssignable` preserves existing human assignees. Both trigger a
session when `agentAssignment` and the feature header are present.

## Key findings

| Mutation | Starts a session? | Notes |
|---|---|---|
| `addAssigneesToAssignable` (no `agentAssignment`) | **No** | Plain assignment; what `assign_self` does today |
| `addAssigneesToAssignable` + `agentAssignment` + header | **Yes** ✅ | Keeps human assignees; chosen path |
| `replaceActorsForAssignable` + `agentAssignment` + header | **Yes** ✅ | Replaces all assignees |

The session trigger is the `agentAssignment` input **plus** the
`GraphQL-Features` feature-flag header — the choice of mutation is secondary.

## Decision

> **The action slice must use `addAssigneesToAssignable` extended with the
> `agentAssignment` input and the `GraphQL-Features` feature-flag header.**

This maximally reuses the proven `assign_self` mutation path; the human
`assign_self` flow keeps its current simple form (no `agentAssignment`, no
feature header).

[docs]: https://docs.github.com/en/copilot/how-tos/use-copilot-agents/cloud-agent/use-cloud-agent-via-the-api
[schema]: https://docs.github.com/en/graphql/reference/mutations#addassigneestoassignable
