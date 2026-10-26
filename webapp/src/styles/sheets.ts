// The three stylesheets as the tests read them: each entry with its parts inlined.

/**
 * An entry sheet is only a list of `@import`s, which Vite resolves at build
 * time and `?raw` does not: read raw, `styles.css` would hand a test a list of
 * paths and not one rule. So each entry is rebuilt here from its parts, in the
 * order it imports them, which is the order the cascade sees.
 *
 * **Every part is read, or this throws.** A part no entry imports ships
 * nowhere and would be checked by nothing; one an entry names that does not
 * exist would leave a hole. Both fail at import, before any assertion runs.
 */
const entries = import.meta.glob<string>("./*.css", {
  query: "?raw",
  import: "default",
  eager: true,
});
const parts = import.meta.glob<string>("./*/*.css", {
  query: "?raw",
  import: "default",
  eager: true,
});

const IMPORT = /^@import "(\.\/[^"]+)";$/gm;
const unread = new Set(Object.keys(parts));

function inline(entry: string): string {
  const source = entries[entry];
  if (source === undefined) throw new Error(`no stylesheet ${entry}`);
  return source.replace(IMPORT, (_line, path: string) => {
    const part = parts[path];
    if (part === undefined) throw new Error(`${entry} imports ${path}`);
    unread.delete(path);
    return part;
  });
}

export const styles = inline("./styles.css");
export const refresh = inline("./refresh.css");
export const fluid = inline("./fluid.css");

if (unread.size > 0) {
  throw new Error(`no entry imports ${[...unread].join(", ")}`);
}
