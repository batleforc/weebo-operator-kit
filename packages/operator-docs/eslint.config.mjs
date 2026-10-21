import { defineConfig, globalIgnores } from "eslint/config";
import tseslint from "typescript-eslint";

// The house TypeScript rules (no `any`, no `!`, no `as` but `as const`, no
// ignored promise, exhaustive switches, justified `@ts-expect-error` only),
// on the type-checked sources (`task docs:check`).
export default defineConfig([
  globalIgnores(["node_modules/**"]),
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [tseslint.configs.base],
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    linterOptions: { reportUnusedDisableDirectives: "error" },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-non-null-assertion": "error",
      "@typescript-eslint/consistent-type-assertions": ["error", { assertionStyle: "never" }],
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      "@typescript-eslint/switch-exhaustiveness-check": "error",
      "@typescript-eslint/ban-ts-comment": [
        "error",
        { "ts-expect-error": "allow-with-description", minimumDescriptionLength: 10 },
      ],
    },
  },
]);
