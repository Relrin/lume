/**
 * The picture behind the panes, for a configuration that names one.
 *
 * The page never sees where the picture is kept. It asks the core for the picture the
 * configuration names and is given the file's bytes, which it shows only if they are
 * a kind of picture it draws. How faint and how blurred the picture is are design
 * tokens, so changing either restyles it without the picture being read again.
 */

import { useEffect, useRef } from "react";
import { useStore } from "zustand";

import { imageType } from "./background.ts";
import { useShell } from "./context.ts";
import { sentence } from "./notices.ts";

/** The picture, filling the window behind everything else in it. */
export function Background() {
  const app = useShell();
  // What the token says only tells one picture from another: it is not fetched.
  const named = useStore(app.config, (state) => state.view?.tokens["background.image"] ?? "");
  const picture = useRef<HTMLImageElement>(null);

  useEffect(() => {
    const element = picture.current;
    if (!element) return undefined;
    let current = true;
    let address: string | undefined;
    const clear = () => {
      element.removeAttribute("src");
      element.dataset["shown"] = "false";
    };
    if (named === "") {
      clear();
      return undefined;
    }
    app.client.backgroundImage().then(
      (bytes) => {
        if (!current) return;
        const type = imageType(bytes);
        if (!type) {
          clear();
          if (bytes.length > 0) {
            app.notices.push(
              "warning",
              "The picture behind the panes is not a PNG, JPEG, WebP or GIF file.",
            );
          }
          return;
        }
        address = URL.createObjectURL(new Blob([bytes.slice().buffer], { type }));
        element.src = address;
        element.dataset["shown"] = "true";
      },
      (error: unknown) => {
        if (!current) return;
        clear();
        const message = error instanceof Error ? error.message : String(error);
        app.notices.push(
          "warning",
          `The picture behind the panes could not be read. ${sentence(message)}`,
        );
      },
    );
    return () => {
      current = false;
      // The picture that was shown is let go once another takes its place.
      if (address !== undefined) URL.revokeObjectURL(address);
    };
  }, [app, named]);

  return (
    <img
      ref={picture}
      className="shell-background"
      alt=""
      aria-hidden="true"
      draggable={false}
      data-shown="false"
    />
  );
}
