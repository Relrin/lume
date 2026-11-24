/**
 * A check that the pointer reaches what is on show: a surface over the page that lets a
 * press beside a menu or a dialog put it away must lie behind that menu or dialog, or
 * every press in it puts it away too.
 *
 * Scenes are driven with the keyboard and with presses that are handed to an element
 * whatever lies over it, so nothing else notices.
 */

/** What catches the presses beside something. */
const BEHIND = '[class$="-backdrop"], [class$="-scrim"]';

/** What can be pressed. */
const PRESSED = "button, input, a[href], [role=option], [role=menuitem], [role=radio], [role=tab]";

function inside(box: DOMRect, x: number, y: number): boolean {
  return x >= box.left && x < box.right && y >= box.top && y < box.bottom;
}

/** What the pointer does not reach under `page`, each as a sentence. */
export function covered(page: Element): string[] {
  const problems: string[] = [];
  for (const behind of page.querySelectorAll(BEHIND)) {
    // One that holds what it is behind cannot lie over it.
    const front = behind.childElementCount === 0 ? behind.nextElementSibling : null;
    if (!front) continue;
    const box = front.getBoundingClientRect();
    for (const pressed of front.querySelectorAll(PRESSED)) {
      const part = pressed.getBoundingClientRect();
      const x = part.left + part.width / 2;
      const y = part.top + part.height / 2;
      // What is scrolled away is not there to be pressed.
      if (part.width === 0 || part.height === 0 || !inside(box, x, y)) continue;
      if (document.elementFromPoint(x, y) === behind) {
        const name = pressed.getAttribute("aria-label") ?? pressed.textContent;
        problems.push(`"${name}" lies under .${behind.className}, which takes its presses`);
      }
    }
  }
  return problems;
}
