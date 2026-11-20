# @lume/plugin-kit

What a Lume plugin's view is built with. A view is a page of the plugin's own, shown in
a sandboxed frame as a panel or in a pane. The kit gives it:

- `connect()`: the way to the plugin's code (`host.call`, `host.on`), what is kept for
  the view between its runs (`state`), streams the plugin's code let it use
  (`streams.open`), the window's look, and the keyboard;
- hooks that read those from React: `useHostEvent`, `useViewState`, `useStream`,
  `useTheme`;
- controls that follow the window's look: `Button`, `IconButton`, `SplitButton`,
  `Segmented`, `Switch`, `Checkbox`, `Tabs`, `List`, `VirtualList`, `Tree`, `SplitView`,
  `TextArea`, `Popover`, `Menu`, `MenuButton`, `Badge`.

```tsx
import "@lume/plugin-kit/kit.css";

import { Button, connect, ViewProvider } from "@lume/plugin-kit";
import { createRoot } from "react-dom/client";

const view = await connect();
createRoot(document.getElementById("root")!).render(
  <ViewProvider view={view}>
    <Button onClick={() => view.host.call("refresh")}>Refresh</Button>
  </ViewProvider>,
);
```

A view that is not written with React imports `@lume/plugin-kit/bridge`, which is
`connect()` and nothing of React's.

A view's page loads its own files and nothing else, and its scripts are plain files: no
script written into the page, and nothing fetched. Bundle the view into one script and
one stylesheet and name them from the page with `<script src>` and `<link>`.

`npm run build` makes `dist/`; `npm pack` builds first and packs it.
