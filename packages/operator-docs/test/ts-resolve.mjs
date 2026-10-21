// Lets `node --test` import the package's TypeScript sources: Node strips
// the types itself, but the sources import without an extension, as the
// sites' bundler resolves them.
import { registerHooks } from "node:module";

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (/^\.\.?\//.test(specifier) && !/\.[cm]?[jt]sx?$/.test(specifier)) {
      try {
        return nextResolve(`${specifier}.ts`, context);
      } catch {
        // Not a .ts file: resolved as written.
      }
    }
    return nextResolve(specifier, context);
  },
});
