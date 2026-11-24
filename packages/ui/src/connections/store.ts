/**
 * Whether the dialog that adds or changes a connection is on show, and for what.
 */

import { createStore, type StoreApi } from "zustand/vanilla";

import type { ConnectionKind, EditedConnection } from "./model.ts";

/** What the dialog was asked to show. */
export interface ConnectionAsked {
  readonly kind: ConnectionKind;
  /** The address of a Rex server to begin with. */
  readonly address: string;
  /** What is changed. Nothing, to add something. */
  readonly edit: EditedConnection | null;
  /** Whether a key opened it, which shows it at once. */
  readonly keyboard: boolean;
  /** Counts the times it was opened: each opening begins afresh. */
  readonly opening: number;
}

/** The dialog's state. */
export interface ConnectionDialogState {
  readonly open: ConnectionAsked | null;
  /** Shows the dialog to add something. */
  add(kind: ConnectionKind, keyboard: boolean, address?: string): void;
  /** Shows the dialog to change something. */
  edit(edit: EditedConnection, keyboard: boolean): void;
  close(): void;
}

/** The dialog of this page: a window has one. */
export const connectionDialog: StoreApi<ConnectionDialogState> = createStore<ConnectionDialogState>(
  (set, get) => {
    let opening = 0;
    return {
      open: null,
      add: (kind, keyboard, address = "") =>
        set({ open: { kind, address, edit: null, keyboard, opening: ++opening } }),
      edit: (edit, keyboard) =>
        set({
          open: {
            kind: edit.kind,
            address: edit.kind === "rex" ? edit.address : "",
            edit,
            keyboard,
            opening: ++opening,
          },
        }),
      close: () => {
        if (get().open !== null) set({ open: null });
      },
    };
  },
);
