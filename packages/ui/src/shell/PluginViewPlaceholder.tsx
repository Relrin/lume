/**
 * What a panel shows in place of a plugin's view, for as long as the window cannot
 * show the view itself: which plugin the view belongs to and what the view is called.
 */

/** Stands where the view `view` of the plugin `plugin` would be. */
export function PluginViewPlaceholder({ plugin, view }: { plugin: string; view: string }) {
  return (
    <div className="panel-placeholder">
      <p className="panel-placeholder-name">{view}</p>
      <p>
        This panel belongs to the plugin <span className="panel-placeholder-plugin">{plugin}</span>,
        whose view cannot be shown here yet.
      </p>
    </div>
  );
}
