/** @type {import('prettier').Config} */
export default {
  semi: true,
  singleQuote: true,
  printWidth: 100,
  tabWidth: 2,
  trailingComma: 'all',
  arrowParens: 'always',
  bracketSpacing: true,
  endOfLine: 'lf',
  overrides: [
    { files: ['*.md'], options: { proseWrap: 'preserve' } },
    { files: ['*.{json,jsonc}'], options: { trailingComma: 'none' } },
  ],
};
