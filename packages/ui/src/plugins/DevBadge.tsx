/**
 * The mark of a plugin that is being worked on: its folder is used where it lies, and
 * what comes from it is not held to what the user once looked at. It is shown wherever
 * the plugin's things are named, so they are not taken for an installed plugin's.
 */
export function DevBadge() {
  return (
    <span className="dev-badge" title="From a linked folder: a plugin that is being worked on">
      Dev
    </span>
  );
}
