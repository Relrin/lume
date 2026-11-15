import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";

/**
 * Text that writes a colour: `#rgb`, `#rrggbb` and their forms with opacity, and the
 * functions that make one. Written as a pattern for a selector.
 */
const COLOUR =
  "/(^|[^&0-9a-zA-Z_])#([0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})([^0-9a-zA-Z_]|$)|(rgb|rgba|hsl|hsla|oklch|oklab)[(]/";

export default tseslint.config(
  {
    ignores: [
      "**/dist/**",
      "**/node_modules/**",
      "target/**",
      "vendor/**",
      ".tools/**",
      "crates/lume-app/gen/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
  },
  {
    // The window is drawn with design tokens and nothing else: a colour written into
    // its code would not follow the theme. The test harness and the renderer lab draw
    // fixed pictures and are left out.
    files: ["packages/ui/src/**/*.{ts,tsx}"],
    ignores: ["packages/ui/src/harness/**", "packages/ui/src/lab/**"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector: `Literal[value=${COLOUR}]`,
          message: "A colour belongs in the theme. Use a design token.",
        },
        {
          selector: `TemplateElement[value.raw=${COLOUR}]`,
          message: "A colour belongs in the theme. Use a design token.",
        },
      ],
    },
  },
);
