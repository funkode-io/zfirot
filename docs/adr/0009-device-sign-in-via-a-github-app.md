# Device sign-in via a public GitHub App, alongside the PAT

Zfirot adds a **Device sign-in** ("Sign in with GitHub": show a code, the user
enters it on GitHub) as the default **Sign-in method**, keeping the pasted
fine-grained Personal Access Token as a fallback. The device flow needs only a
public client ID built into the binary — no client secret and no redirect
server — so it keeps ADR 0001's no-backend stance. We back it with a **GitHub
App** (owned by the funkode org, installable by anyone, with the same Issues
read/write, Pull requests read, Contents read and Metadata read grants the PAT
needs), not the OAuth App that `gh` and most terminal agents use. The reason is
least privilege: an OAuth App token carries classic scopes — in practice `repo`,
which means write access to every repo the user can reach — and never expires.
That contradicts the narrow, fine-grained grant the PAT was chosen for, and it
would make ADR 0005's Under-scoped/Rotate model pointless.

## Consequences

- A repo is reachable only where the app has an **Installation**. That is a
  one-time step per account or org, and in a company org it needs an owner's
  approval. The app offers an install link instead of a plain "can't open".
- Tokens last 8 hours and are kept fresh silently with a 6-month refresh token,
  which device-flow tokens may use without a client secret. A refresh GitHub
  refuses is an **Invalid token**, which sends the user back to sign-in.
- When the required grants grow, the Installation's owner accepts the new
  permissions on GitHub. There is no Rotate for a Device sign-in.
- **Sign out** is local only: revoking a GitHub App token needs the client
  secret, which a backend-less app cannot hold.

## Considered options

- **OAuth App (the `gh` model)** — rejected: it works on any repo straight away,
  but with broad, non-expiring write access. Company orgs often block OAuth Apps
  until an admin approves them anyway, so it doesn't reliably "just work" there
  either.
- **Private GitHub App** — rejected: only funkode could install it, so Device
  sign-in would be useless on the evaluating company's repos.
