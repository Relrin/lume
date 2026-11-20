/**
 * The button that opens another tab with the default terminal, at the end of the column
 * of tabs; the top bar has its own beside the window's buttons. A tab with another
 * profile is opened from the application's menu, under "New tab with", or from the
 * palette that finds them.
 */

import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import { PlusIcon } from "./icons.tsx";

/** Opens a tab with the default terminal: a row with its name beside the picture. */
export function NewTabButton() {
  const app = useShell();
  const hint = useKeyHint("lume.tab.new");
  return (
    <button
      type="button"
      className="control tab-column-new"
      aria-label="New tab"
      title={`New tab${hint}`}
      onPointerDown={(event) => event.preventDefault()}
      onClick={() => app.commands.run("lume.tab.new")}
    >
      <PlusIcon />
      <span>New tab</span>
    </button>
  );
}
