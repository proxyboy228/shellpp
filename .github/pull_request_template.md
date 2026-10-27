<!-- markdownlint-disable-file MD041 -->
<!-- Remove sections that do not apply. Never paste secrets, real keys or passwords here. -->

## Summary

<!-- What does this change do, and why? -->

## Related issue

Closes #

## Type of change

<!-- Conventional Commit type. It must match the type in the PR title. -->

- [ ] `feat`: new feature
- [ ] `fix`: bug fix
- [ ] `docs`: documentation or specs only
- [ ] `refactor`: code change that neither fixes a bug nor adds a feature
- [ ] `perf`: performance improvement
- [ ] `test`: tests only
- [ ] `build`: build system or dependencies
- [ ] `ci`: CI configuration
- [ ] `chore`: other maintenance
- [ ] `revert`: reverts a previous change
- [ ] Breaking change (`!` in the PR title and a `BREAKING CHANGE:` footer)

## Spec impact

<!--
List the spec sections this PR implements or changes, for example:
SPEC.md 3.1, docs/ui-behavior-spec.md 4.2, docs/design-system.md 2.3.
If the implemented behavior differs from the spec, update the spec in this PR.
When a spec changes, update the matching document under docs/ru/ as well, or state that the translation still needs updating.
-->

- Sections implemented or changed:
- [ ] The behavior matches the specs, or the specs are updated in this PR.

## How tested

<!-- Describe the tests you ran (unit, integration, manual) and on which operating systems. The full local check is scripts\verify.ps1 (CONTRIBUTING.md, "Tests"). -->

## Checklist

- [ ] The PR title follows [Conventional Commits](https://www.conventionalcommits.org/), for example `feat(sftp): resume interrupted uploads`.
- [ ] Tests are added or updated for the changed behavior.
- [ ] No secrets, real keys, passwords or `.env` files are included. Tests generate keys on the fly.
- [ ] The gitleaks pre-commit hook passed.
- [ ] Clean room: no code, screenshots, assets or texts from proprietary clients. Behavior is derived from the specs only.
- [ ] New dependencies, if any, are permissively licensed (MIT, Apache-2.0, BSD, ISC, MPL-2.0; OFL for fonts) and are added to `THIRD_PARTY_NOTICES.md` (the Rust crate tables, also the server's in `server/THIRD_PARTY_NOTICES.md`: `node scripts/rust-notices.mjs --write`).
- [ ] Documentation is updated.
- [ ] No telemetry, phone-home, license checks or remote feature disabling is introduced.
