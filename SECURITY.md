# Security policy

## Reporting a vulnerability

Please report security problems privately, not in public issues or pull requests.

- Use GitHub's private reporting: the repository's **Security** tab, then **Report a vulnerability**
  (https://github.com/b070nd/seatwise/security/advisories/new).

Include what you found, how to reproduce it, the version (`seat --version`) and what you think the impact is. Do not
include real org UUIDs, emails or tokens.

We aim to confirm receipt within a few working days, tell you whether we consider it a vulnerability, and agree a
disclosure date with you. Our default is to publish a fix and an advisory within 90 days of the report, sooner if the
fix is simple. We credit reporters who want credit.

This is a volunteer project. These are goals, not guarantees.

## Supported versions

Only the latest release receives security fixes.

## Scope

In scope: the `seat` binary, its hooks, the skills it installs, its configuration handling, and the redaction of account
identifiers.

Out of scope: weaknesses in Claude Code, Codex or their services, a compromised host or user account, and anything that
needs the attacker to already control seatwise's config file. The hooks are a guard against mistakes, not a sandbox:
see [docs/security.md](docs/security.md) for what seatwise does and does not defend against.

## Good-faith research

If you make a good-faith effort to follow this policy, avoid harming other users, and give us reasonable time to fix an
issue before disclosure, we will not pursue or support action against you for your research.
