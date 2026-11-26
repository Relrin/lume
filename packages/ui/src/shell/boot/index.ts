/**
 * Starting the window's page: once for each page, however often a component mounts.
 *
 * What is slow or can be had only once is done here: loading the terminal engine, asking
 * for a graphics device, opening the carrier. In development React mounts components
 * twice, and a second carrier would retire the first, so none of this may follow a
 * component's life.
 *
 * The page owns nothing that lasts. The layout, the configuration and the commands are
 * the core's; the page asks for each once and follows what it is told from then on. A
 * page that is reloaded starts here again and finds every session still running.
 *
 * The start is a list of phases, in the order below; each says why it comes where it
 * does. What is made later but called back earlier is handed out as a `late` cell.
 */

import { DEFAULT_TOKENS } from "@lume/tokens";

import type { Commands } from "../../commands/run.ts";
import { CoreClient } from "../../core/client.ts";
import { checkFrameContract } from "../../frame-contract.ts";
import { TabAreaController } from "../../layout/controller.ts";
import { setMotionSource } from "../../motion/slots.ts";
import type { PaneViews } from "../../panes/views.ts";
import { assets } from "../../plugins/assets.ts";
import { followPluginStatus } from "../../plugins/status.ts";
import type { SearchController } from "../../search/controller.ts";
import { startStep } from "../../start-marks.ts";
import { PromptStore } from "../../terminal/prompts.ts";
import { TauriCarrier, type Carrier } from "../../transport/carrier.ts";
import { closeRequested, confirmRequested, type ClosingApp } from "../closing.ts";
import { NoticeStore } from "../notices.ts";
import { workOf } from "../work.ts";
import { buildApp, type ShellApp } from "./app.ts";
import { Contributions } from "./contributions.ts";
import { askSlowly, takeCopies, watchVisible } from "./core.ts";
import { createCardDrag, createCommands, createSearch, createViews } from "./controllers.ts";
import { startCursor } from "./cursor.ts";
import { Disposer } from "./disposer.ts";
import { keyboardLayout, startVt } from "./engine.ts";
import { followFocus } from "./focus.ts";
import { followCore } from "./follow.ts";
import { CoreGate } from "./gate.ts";
import { createInput } from "./input.ts";
import { late } from "./late.ts";
import { followConfig, settleHost, startHost, TerminalLook } from "./render.ts";
import { answering, remoteTarget } from "./requests.ts";
import { appearanceFor, createStores } from "./stores.ts";

export type { ShellApp } from "./app.ts";
export { startVt } from "./engine.ts";

/** The page's prompts. Created with the module so a component can show them at once. */
export const prompts = new PromptStore();

/** The page's notices. Created with the module, like the prompts. */
export const notices = new NoticeStore();

/**
 * Starts a window's page on `carrier`. The application's own page does this once, on
 * the carrier into the core; a test starts one on a carrier of its own.
 */
export async function startShell(carrier: Carrier): Promise<ShellApp> {
  checkFrameContract();
  const disposer = new Disposer();
  // Stores first: they need nothing, and every later phase reads or writes them.
  const stores = createStores();
  const { layout, config, plugins } = stores;
  const app = late<ShellApp>("the window's app");
  const views = late<PaneViews>("the panes' views");
  const search = late<SearchController>("the panes' searches");
  const commands = late<Commands>("the commands");

  // The gate comes before the core is opened, so that nothing the core says early is lost.
  const gate = new CoreGate();
  // The renderers' host starts before the core answers: nothing waits for a graphics device.
  const started = startHost();
  const opened = CoreClient.open(carrier, {
    ...gate.handlers,
    failure: (error) =>
      prompts.notice(`The application can no longer be reached: ${error.message}`),
  });
  // The first copies are asked for as soon as the core can be reached: they do not
  // wait for the terminal emulator, which is fetched and compiled meanwhile.
  const copies = opened.then((client) =>
    Promise.all([client.layoutView(), client.configView(), client.commandsList()]),
  );
  const [vt, keyLayout, client, [layoutView, configView, commandList]] = await Promise.all([
    startVt().then((vt) => {
      startStep("vt");
      return vt;
    }),
    keyboardLayout(),
    opened,
    copies,
  ]);
  // The render tier is settled before anything captures the host.
  const host = await settleHost(started, configView);
  disposer.add(() => host.dispose());

  // The first copies are taken in before anything reads them.
  const machine = takeCopies(stores, layoutView, configView, commandList);
  // What takes the core a moment is asked for now; profiles that come before the
  // views exist restyle them once they are made.
  askSlowly(client, stores, views, notices);
  // The window tells the core what it shows from the moment the core can be told.
  const visible = watchVisible(client, disposer);

  // The look and its fonts come after the configuration and before any terminal
  // measures a cell. Nothing awaits after this: the rest is there before going live.
  const look = await TerminalLook.load(config);
  const area = new TabAreaController({
    layout,
    config: () => config.getState().view,
    apply: (intent) => client.layoutApply(intent),
    defaults: DEFAULT_TOKENS,
  });
  // The bindings read keys from here on; they run commands, which are made further on.
  const input = createInput({
    ...stores,
    keyLayout,
    bindings: configView.keymap.bindings,
    commands,
  });
  // The cursor's effects draw with the settled host, so they come after it.
  const cursor = startCursor({ host, client, config, plugins, notices, disposer });
  // What the plugins add sets the cursor's effects up again, so it comes after them.
  const contributions = new Contributions({
    client,
    plugins,
    notices,
    changed: () => {
      assets.retry();
      cursor.configure();
    },
  });
  contributions.fetch();

  // The views capture the settled host, the look and the bindings, so they come here.
  const current = () => look.current;
  const appearanceOf = appearanceFor(stores, current);
  const paneViews = disposer.use(
    createViews({
      stores,
      vt,
      host,
      cursorFx: cursor.cursorFx,
      keyLayout,
      client,
      area,
      prompts,
      input,
      look: current,
      appearanceOf,
      search,
    }),
  );
  views.set(paneViews);
  // The searches and the commands work on the views, so they come after them.
  const paneSearch = createSearch(paneViews, disposer);
  search.set(paneSearch);
  const work = (pane: string) => workOf(app.get(), pane);
  const closing: ClosingApp = { layout, config, views: paneViews, prompts, client, notices, work };
  const paneCommands = createCommands({
    stores,
    client,
    views: paneViews,
    search: paneSearch,
    prompts,
    notices,
    closing,
  });
  commands.set(paneCommands);
  const cardDrag = createCardDrag(stores, area, client, notices);

  // The keyboard and the title listen before the layout's first copy is taken in, so
  // that the copy is followed like any change after it.
  const focus = followFocus({
    ...stores,
    prompts,
    views: paneViews,
    search: paneSearch,
    input,
    app,
    disposer,
  });
  // The request target is complete before going live answers what the core held.
  const target = remoteTarget({
    ...stores,
    host,
    area,
    views: paneViews,
    commands: paneCommands,
    prompts,
    look: current,
    appearanceOf,
    onWindowFocus: focus.onWindowFocus,
  });
  // Everything a notice touches exists now, before any notice is followed.
  const follow = followCore({
    ...stores,
    client,
    notices,
    prompts,
    contributions,
    configChanged: followConfig({ look, views: paneViews, area, cursor, input }),
    restyle: () => paneViews.restyle(),
    closeRequested: () => void closeRequested(closing),
    confirmRequested: (notice) =>
      void confirmRequested({ prompts, views: paneViews, commands: paneCommands }, notice),
  });
  disposer.add(followPluginStatus(plugins, stores.status, config));
  setMotionSource({ view: () => config.getState().view, shown: () => plugins.getState().shown });
  disposer.add(() => setMotionSource(null));

  // The app is whole before going live: the title and the questions read it.
  const shell = buildApp({
    stores,
    client,
    machine,
    area,
    views: paneViews,
    commands: paneCommands,
    prompts,
    notices,
    visible,
    cardDrag,
    search: paneSearch,
    cursorFx: cursor.cursorFx,
    forwardKey: input.forwardKey,
    stats: () => ({ ...host.stats(), wasmBytes: vt.memoryBytes }),
    dispose: disposer.dispose,
  });
  app.set(shell);

  // Going live: the window's state is asked for, the layout's copy taken in, and what
  // the core said and asked meanwhile is followed and answered, in that order.
  client.windowState().then(
    (state) => stores.windowState.getState().set(state),
    () => undefined,
  );
  layout.getState().reset(layoutView);
  startStep("layout");
  gate.goLive(follow, answering(target, client));
  // Ending the page stops following the core first.
  disposer.add(() => gate.close());
  focus.retitle();
  return shell;
}

let page: Promise<ShellApp> | undefined;

/** The window's page. The first call starts it; every call gets the same one. */
export function shellApp(): Promise<ShellApp> {
  page ??= startShell(new TauriCarrier());
  return page;
}
