/**
 * The shell's icons: small line drawings in the colour of the text around them, and the
 * tiles that show what program a pane runs.
 *
 * Each is decoration beside a label or inside a button that names itself, so none is
 * announced on its own.
 */

import { useContext, useSyncExternalStore, type CSSProperties } from "react";

import type { IconRef, PluginImage } from "../core/types.ts";
import { BrandMark, Glyph } from "../icons/Glyph.tsx";
import type { LineGlyphName } from "../icons/glyphs.generated.ts";
import { pictureOf, tileOf, type PictureTile, type TileId } from "../icons/tiles.ts";
import type { GroupIcon } from "../layout/types.ts";
import { PluginIcon } from "../plugins/PluginIcon.tsx";
import { TILE_SIZES, type TilePlace } from "../tuning.ts";
import { ShellContext } from "./context.ts";

/** The pictures a group of tabs can show, by what the layout calls them. */
const GROUP_GLYPHS: Readonly<Record<GroupIcon, LineGlyphName>> = {
  bug: "bug",
  flame: "flame",
  rocket: "rocket",
  flask: "flask-conical",
  wrench: "wrench",
  layers: "layers",
  server: "server",
  book: "book-open",
};

/** A group's picture. */
export function GroupGlyph({ icon, size = 12 }: { icon: GroupIcon; size?: number }) {
  return <Glyph name={GROUP_GLYPHS[icon]} size={size} weight={2.3} />;
}

/** A line drawing for a menu's row, at the size menus draw them. */
export function MenuGlyph({ name, size = 14 }: { name: LineGlyphName; size?: number }) {
  return <Glyph name={name} size={size} weight={1.9} />;
}

/** A padlock: a tab that is not closed by asking. */
export function LockIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="lock" size={size} weight={1.9} />;
}

/** A bell struck through: a tab or a group that does not ask for attention. */
export function BellOffIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="bell-off" size={size} weight={1.9} />;
}

/** A shell's prompt: what a terminal is. */
export function TerminalIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="terminal" size={size} />;
}

export function PlusIcon({ size = 14 }: { size?: number }) {
  return <Glyph name="plus" size={size} />;
}

export function CloseIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="x" size={size} />;
}

/** A pane divided into a left and a right half. */
export function SplitRightIcon({ size = 14 }: { size?: number }) {
  return <Glyph name="columns-2" size={size} weight={1.8} />;
}

/** A pane divided into an upper and a lower half. */
export function SplitDownIcon({ size = 14 }: { size?: number }) {
  return <Glyph name="rows-2" size={size} weight={1.8} />;
}

/** Two arrows pointing apart: a pane grown to fill its tab. */
export function ZoomIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="maximize-2" size={size} weight={1.8} />;
}

/** A ring with a dot: the application's mark. */
export function LumeMark({ size = 18 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="2" />
      <circle cx="12" cy="12" r="3.5" fill="currentColor" />
    </svg>
  );
}

/** Two overlapping squares: a pane lifted off the others. */
export function FloatIcon({ size = 14 }: { size?: number }) {
  return <Glyph name="copy" size={size} weight={1.8} />;
}

/** A short line: a pane put away at the corner. */
export function PutAwayIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="minus" size={size} />;
}

/** Three lines: the application's menu. */
export function MenuIcon({ size = 16 }: { size?: number }) {
  return <Glyph name="menu" size={size} />;
}

export function ChevronDownIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="chevron-down" size={size} weight={2.2} />;
}

export function ChevronRightIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="chevron-right" size={size} weight={2.2} />;
}

export function CheckIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="check" size={size} weight={2.2} />;
}

/** A line: a window put away. */
export function MinimiseIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="minus" size={size} />;
}

/** A square: a window grown to fill the screen. */
export function MaximiseIcon({ size = 11 }: { size?: number }) {
  return <Glyph name="square" size={size} />;
}

/** A square in front of another: a window put back to the size it had. */
export function RestoreIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="copy" size={size} />;
}

/** A bar folded against an edge: a panel put away. */
export function CollapseIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="panel-left" size={size} weight={1.9} />;
}

export function SearchIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="search" size={size} />;
}

/** A cog: how the application as a whole behaves. */
export function GeneralIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="settings" size={size} weight={1.8} />;
}

/** A disc, one half of it filled: how things look. */
export function AppearanceIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="contrast" size={size} weight={1.8} />;
}

/** A line that forks: a branch of a repository. */
export function BranchIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="git-branch" size={size} weight={1.9} />;
}

/** A heartbeat: something running. */
export function PulseIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="activity" size={size} />;
}

export function FolderIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="folder" size={size} weight={1.9} />;
}

/** A triangle with a mark in it: something that wants attention. */
export function AlertIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="triangle-exclamation-point" size={size} weight={1.9} />;
}

/** A filled dot. */
export function DotIcon({ size = 9 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="9" fill="currentColor" />
    </svg>
  );
}

/** A page with lines on it: an editor, or something that shows a file. */
export function EditorIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="file-text" size={size} weight={1.9} />;
}

/** A page with a folded corner: a file to open. */
export function FileIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="file" size={size} weight={1.8} />;
}

/** Sparks: a coding agent. */
export function SparkleIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="sparkles" size={size} />;
}

/** A globe: another machine. */
export function GlobeIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="globe" size={size} weight={1.8} />;
}

/** Stacked layers: a system inside this one. */
export function LayersIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="layers" size={size} weight={1.8} />;
}

/** A box: a container or a virtual machine. */
export function BoxIcon({ size = 13 }: { size?: number }) {
  return <Glyph name="box" size={size} weight={1.8} />;
}

/** A tick: something finished. */
export function DoneIcon({ size = 11 }: { size?: number }) {
  return <Glyph name="check" size={size} weight={3} />;
}

/** An eye: the list of what the window's panes are doing. */
export function EyeIcon({ size = 16 }: { size?: number }) {
  return <Glyph name="eye" size={size} weight={1.9} />;
}

/** The Command key's loop: the command palette. */
export function CommandIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="command" size={size} weight={1.9} />;
}

/** A circle with a mark in it: something waits on the user, or went wrong. */
export function AttentionIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="circle-exclamation-point" size={size} weight={2.4} />;
}

/** An arrow pointing on: go to what is named. */
export function ArrowRightIcon({ size = 11 }: { size?: number }) {
  return <Glyph name="arrow-right" size={size} weight={2.4} />;
}

/** An arrow going round: start something again. */
export function AgainIcon({ size = 12 }: { size?: number }) {
  return <Glyph name="rotate-cw" size={size} />;
}

/** Two arrows chasing each other: look again. */
export function RefreshIcon({ size = 15 }: { size?: number }) {
  return <Glyph name="refresh-cw" size={size} weight={1.8} />;
}

const NO_IMAGES: Readonly<Record<string, PluginImage>> = {};
const never = () => () => undefined;

/**
 * A picture a pane was given, or its view comes with, where a tile goes: a plugin's own
 * picture or one of the page's line drawings, on a tile of no colour of its own.
 */
function PictureTileView({ picture, place }: { picture: IconRef; place: TilePlace }) {
  const app = useContext(ShellContext);
  const read = () => app?.plugins.getState().shown?.images ?? NO_IMAGES;
  const images = useSyncExternalStore(app ? app.plugins.subscribe : never, read, read);
  const size = TILE_SIZES[place];
  const style = {
    "--tile-own-bg": "var(--lume-color-control)",
    "--tile-own-ink": "var(--lume-color-text)",
    "--tile-width": `${size.width}px`,
    width: size.width,
    height: size.height,
  } as CSSProperties;
  return (
    <span className="tile" data-tile="picture" style={style}>
      <PluginIcon icon={picture} images={images} size={size.glyph} weight={2.4} />
    </span>
  );
}

/**
 * The tile of a program: its drawing on its colour, sized for the place it is shown. A
 * pane that is known by a picture instead shows the picture in the tile's place.
 */
export function ProgramTile({ tile, place }: { tile: TileId | PictureTile; place: TilePlace }) {
  const picture = pictureOf(tile);
  if (picture !== undefined) return <PictureTileView picture={picture} place={place} />;
  return <NamedTile tile={tile as TileId} place={place} />;
}

function NamedTile({ tile, place }: { tile: TileId; place: TilePlace }) {
  const drawn = tileOf(tile);
  const size = TILE_SIZES[place];
  const style = {
    "--tile-own-bg": drawn.background,
    "--tile-own-ink": drawn.ink,
    "--tile-width": `${size.width}px`,
    width: size.width,
    height: size.height,
  } as CSSProperties;
  return (
    <span className="tile" data-tile={tile} data-edge={drawn.edge || undefined} style={style}>
      {drawn.glyph.kind === "brand" ? (
        <BrandMark name={drawn.glyph.name} size={size.glyph} frame={size} />
      ) : (
        <Glyph name={drawn.glyph.name} size={size.glyph} weight={2.4} frame={size} />
      )}
    </span>
  );
}
