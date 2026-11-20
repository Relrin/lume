/**
 * What a Lume plugin's view is built with.
 *
 * - `connect()` and what it gives: calls to the plugin's code and events from it, what
 *   is kept for the view, streams, the window's look, the keyboard (`./bridge.ts`; also
 *   on its own as `@lume/plugin-kit/bridge`, for a view not written with React);
 * - hooks that read those from React (`./hooks.tsx`);
 * - controls that follow the window's look (`./components`), styled by
 *   `@lume/plugin-kit/kit.css`.
 */

export * from "./bridge";
export {
  type StreamStatus,
  useHostEvent,
  useStream,
  useTheme,
  useView,
  useViewState,
  ViewProvider,
} from "./hooks";

export {
  Badge,
  type BadgeProps,
  Button,
  type ButtonProps,
  Checkbox,
  type CheckboxProps,
  type Checked,
  IconButton,
  type IconButtonProps,
  Segmented,
  type SegmentedOption,
  type SegmentedProps,
  Switch,
  type SwitchProps,
  type TabItem,
  Tabs,
  type TabsProps,
  TextArea,
  type TextAreaProps,
} from "./components/controls";
export { CheckMark, ChevronDown, ChevronRight, cx, DashMark } from "./components/icons";
export {
  flattenTree,
  List,
  type ListProps,
  type RowState,
  Tree,
  type TreeNode,
  type TreeProps,
  type TreeRowState,
  VirtualList,
  type VirtualListProps,
  visibleRange,
} from "./components/lists";
export {
  Menu,
  MenuButton,
  type MenuButtonProps,
  type MenuItem,
  type MenuProps,
  Popover,
  type PopoverProps,
  SplitButton,
  type SplitButtonProps,
} from "./components/popover";
export { clampShare, SplitView, type SplitViewProps } from "./components/SplitView";
