import eslint from '@eslint/js';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  { ignores: ['**/out/**', '**/node_modules/**', '**/.vscode-test/**', 'packages/vscode/vendor/**'] },
  eslint.configs.recommended,
  tseslint.configs.recommended
);
