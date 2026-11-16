/**
 * The lists an argument's value is chosen from, as the palette offers them.
 *
 * What a command is given is an id; what a person picks by is a name. For a workspace
 * the two are the same. For a profile they are not: `auto.ssh.work` is offered as
 * "SSH: work".
 */

import type { Profile, ProfileList, WorkspaceList } from "../core/types.ts";
import type { Choice } from "./args.ts";

/** What kind of place a profile's terminal is in, as a list calls it. */
export function profileGroup(profile: Pick<Profile, "launcher">): string {
  switch (profile.launcher.type) {
    case "wsl":
      return "WSL";
    case "ssh":
      return "SSH";
    default:
      return "Local";
  }
}

/** The workspaces there are to choose from. */
export function workspaceChoices(list: WorkspaceList): Choice[] {
  return list.workspaces.map((workspace) => ({ value: workspace.name, label: workspace.name }));
}

/** The profiles there are to choose from. A hidden one is left out. */
export function profileChoices(list: Pick<ProfileList, "profiles">): Choice[] {
  return list.profiles
    .filter((profile) => !profile.hidden)
    .map((profile) => ({
      value: profile.id,
      label: `${profileGroup(profile)}: ${profile.name}`,
    }));
}
