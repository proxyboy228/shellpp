# Third-party notices

This file lists third-party software, fonts and other material that is
included in or distributed with this project, together with its license.

Rules (see `SPEC.md`, section "Clean room"):

- Only permissive licenses are allowed: MIT, Apache-2.0, BSD-2-Clause,
  BSD-3-Clause, ISC, MPL-2.0, Zlib, BSL-1.0, Unicode-3.0, CC0-1.0, Unlicense,
  0BSD, CDLA-Permissive-2.0 and public domain (SQLite); OFL-1.1 for fonts.
  GPL and AGPL code is forbidden in the client distribution (see
  `docs/decisions.md`, decision 4).
- Development-only packages that are never shipped may also use BlueOak-1.0.0
  and CC-BY-4.0. They are not listed here.
- Check the license before adding a dependency and add it here in the same
  pull request. CI runs `node scripts/check-licenses.mjs`, which fails when a
  production npm dependency has a license outside its allowlist.
- Where a license requires it (for example Apache-2.0 `NOTICE` files or the
  OFL), the full license text ships with the distributed application.

## Rust crates

| Crate | Version | License | Source |
|---|---|---|---|

## npm packages

| Package | Version | License | Source |
|---|---|---|---|
| @types/react | 19.3.0 | MIT | <https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/react> |
| csstype | 3.2.3 | MIT | <https://github.com/frenic/csstype> |
| lucide-react | 1.59.0 | ISC (some icons derived from Feather, MIT) | <https://github.com/lucide-icons/lucide> |
| react | 19.3.1 | MIT | <https://github.com/react/react> |
| react-dom | 19.3.1 | MIT | <https://github.com/react/react> |
| scheduler | 0.28.0 | MIT | <https://github.com/react/react> |

## Fonts

| Font | Version | License | Source |
|---|---|---|---|

## Other

| Name | Version | License | Source |
|---|---|---|---|
