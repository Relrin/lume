/**
 * How the window's components reach what was started for the page.
 */

import { createContext, useContext } from "react";

import type { ShellApp } from "./boot.ts";

/** The page's stores, views and commands, for the components under a shell. */
export const ShellContext = createContext<ShellApp | null>(null);

/** The page's stores, views and commands. Only for components rendered inside a shell. */
export function useShell(): ShellApp {
  const app = useContext(ShellContext);
  if (!app) throw new Error("this component is rendered outside the shell");
  return app;
}
