// Token values for the showcase captions, read from tokens.css itself so
// that the page never repeats a raw value (docs/design-system.md, 7.1).

import tokensCss from '../theme/tokens.css?raw';
import { parseThemeTokens, resolveToken, tokensForTheme, type ThemeName, type TokenMap } from '../theme/tokens';

const THEME_TOKENS = parseThemeTokens(tokensCss);

const TOKENS_BY_THEME: Readonly<Record<ThemeName, TokenMap>> = {
  dark: tokensForTheme(THEME_TOKENS, 'dark'),
  light: tokensForTheme(THEME_TOKENS, 'light'),
};

/** The value of a token in a theme, following `var()` references. */
export function tokenValue(theme: ThemeName, name: string): string {
  return resolveToken(TOKENS_BY_THEME[theme], name);
}

/** The value of a theme-independent token (spacing, sizes, fonts, tile colors). */
export function sharedTokenValue(name: string): string {
  return resolveToken(THEME_TOKENS.shared, name);
}
