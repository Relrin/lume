/**
 * The button that opens another tab with the default terminal, as the row of tabs and
 * the column of tabs show it. A tab with another profile is opened from the
 * application's menu, under "New tab with", or from the palette that finds them.
 */

import { useShell } from "./context.ts";
import { useKeyHint } from "./hints.ts";
import { PlusIcon } from "./icons.tsx";

/**
 * Opens a tab with the default terminal. `column` says the tabs are listed down the
 * side of the window, where the button is a row with its name beside the picture.
 */
export function NewTabButton({ column = false }: { column?: boolean }) {
  const app = useShell();
  const hint = useKeyHint("lume.tab.new");
  return (
    <button
      type="button"
      className={column ? "control tab-column-new" : "control tab-new"}
      aria-label="New tab"
      title={`New tab${hint}`}
      onPointerDown={(event) => event.preventDefault()}
      onClick={() => app.commands.run("lume.tab.new")}
    >
      <PlusIcon />
      {column && <span>New tab</span>}
    </button>
  );
}
