/**
 * Whether a pane has something to show that the user has not looked at.
 *
 * A pane the user is not looking at gathers what happens in it: it printed something,
 * its program rang the bell or now waits on the user, a command ended, the program
 * itself ended. What it has gathered is shown on its card and its tab until the user
 * looks at it. A pane the user is looking at gathers nothing.
 *
 * Only news counts. A report of the whole, as a session sends when the page attaches to
 * it, says how things stand and is never something that has just happened.
 */

import type { ExitStatus, SessionEvent } from "../transport/types.ts";

/**
 * What a pane has gathered: nothing; output; something that wants the user; or that
 * and something went wrong.
 */
export type Activity = "idle" | "output" | "attention" | "attention-error";

/** How much each kind says, so that a pane never shows less than it has gathered. */
const WEIGHT: Readonly<Record<Activity, number>> = {
  idle: 0,
  output: 1,
  attention: 2,
  "attention-error": 3,
};

/** The greater of two. */
export function raise(held: Activity, wanted: Activity): Activity {
  return WEIGHT[wanted] > WEIGHT[held] ? wanted : held;
}

/** The greatest of several, or `idle` for none. */
export function greatest(activities: Iterable<Activity>): Activity {
  let most: Activity = "idle";
  for (const activity of activities) most = raise(most, activity);
  return most;
}

/** What a status record held before a report, so that only a change counts. */
interface Before {
  readonly statuses: readonly { readonly id?: string; readonly state: string }[];
}

/** What a piece of news, on its own, asks of a pane the user is not looking at. */
function wanted(event: SessionEvent, before: Before): Activity {
  switch (event.type) {
    case "activity":
      return "output";
    case "bell":
      return "attention";
    case "command_finished":
      return event.exit !== undefined && event.exit !== 0 ? "attention-error" : "attention";
    case "program_status": {
      if (event.state !== "blocked" && event.state !== "error") return "idle";
      const was = before.statuses.find((record) => record.id === event.id)?.state;
      if (was === event.state) return "idle";
      return event.state === "error" ? "attention-error" : "attention";
    }
    default:
      return "idle";
  }
}

/**
 * What a pane has gathered once `event` has arrived. `before` is what the pane held
 * before the event; `attended`, whether the user is looking at the pane.
 */
export function nextActivity(
  held: Activity,
  event: SessionEvent,
  before: Before,
  attended: boolean,
): Activity {
  if (attended) return held;
  return raise(held, wanted(event, before));
}

/** What a pane has gathered once its program has ended. */
export function exitActivity(held: Activity, status: ExitStatus, attended: boolean): Activity {
  if (attended) return held;
  const clean = typeof status === "object" && "code" in status && status.code === 0;
  return raise(held, clean ? "attention" : "attention-error");
}
