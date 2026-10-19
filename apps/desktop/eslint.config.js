import js from '@eslint/js';
import { defineConfig, globalIgnores } from 'eslint/config';
import reactHooks from 'eslint-plugin-react-hooks';
import globals from 'globals';
import tseslint from 'typescript-eslint';

export default defineConfig([
  globalIgnores(['dist', 'coverage']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [js.configs.recommended, tseslint.configs.recommended, reactHooks.configs.flat.recommended],
    languageOptions: {
      ecmaVersion: 2023,
      globals: globals.browser,
    },
    rules: {
      // Untrusted strings are rendered as text only (docs/security.md, section 13.1).
      'no-restricted-syntax': [
        'error',
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: 'dangerouslySetInnerHTML is not allowed (docs/security.md, section 13.1).',
        },
        {
          selector: 'AssignmentExpression > MemberExpression.left[property.name=/^(innerHTML|outerHTML)$/]',
          message: 'innerHTML and outerHTML are not assigned (docs/security.md, section 13.1).',
        },
        {
          selector: "CallExpression[callee.property.name='insertAdjacentHTML']",
          message: 'insertAdjacentHTML is not allowed (docs/security.md, section 13.1).',
        },
      ],
    },
  },
  {
    files: ['*.config.ts', 'src/test/**/*.ts', 'src/**/*.{test,spec}.{ts,tsx}'],
    languageOptions: {
      globals: globals.node,
    },
  },
  {
    // UI e2e specs (WebdriverIO + mocha, run in Node): wdio and mocha globals.
    files: ['e2e/**/*.ts'],
    languageOptions: {
      globals: {
        ...globals.node,
        ...globals.mocha,
        browser: 'readonly',
        driver: 'readonly',
        $: 'readonly',
        $$: 'readonly',
        expect: 'readonly',
      },
    },
  },
  {
    files: ['**/*.{js,mjs}'],
    extends: [js.configs.recommended],
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: 'module',
      globals: globals.node,
    },
  },
]);
