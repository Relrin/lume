/**
 * The values the test suites run with. Each is defined here and nowhere else.
 *
 * They decide how long a suite takes and how steady its numbers are, not what passes:
 * the limits a measurement is held to are kept with the benchmarks' budgets.
 */

/** Colour channels may differ by this much between two pictures and still count as equal. */
export const COMPARE_TOLERANCE = 2;

/**
 * A picture is compared with its reference in square blocks of this many pixels a side.
 * A block is about a third of a letter: small enough that a wrong letter changes it,
 * large enough that the softness of an edge does not.
 */
export const REFERENCE_BLOCK_PIXELS = 4;

/**
 * How far the mean of a colour channel over a block may be from the reference's. Text
 * is drawn by the system's rasteriser, whose edges differ by this much between machines
 * and between versions of a webview; a letter missing or moved changes it several times
 * over.
 */
export const REFERENCE_BLOCK_TOLERANCE = 24;

/** The share of a picture's blocks that may differ from its reference's. */
export const REFERENCE_MAX_DIFFERENT = 0.005;

/**
 * The largest frame a reference picture is kept for, in cells. The full screens drawn
 * for timing are megabytes as pictures, and show nothing the smaller frames do not.
 */
export const REFERENCE_MAX_CELLS = 80 * 24;

/** The device pixel ratio pictures are drawn at, whatever the display's is. */
export const SNAPSHOT_PIXEL_RATIO = 1;

/** Full redraws timed for each renderer tier, and the ones before them that are not. */
export const REDRAW_FRAMES = 120;
export const REDRAW_WARMUP_FRAMES = 8;

/** The grid the redraw and flood measurements use. */
export const BENCH_COLS = 200;
export const BENCH_ROWS = 60;

/** How much a flood prints when only the way to the page is timed. */
export const FLOOD_TRANSPORT_BYTES = 256 * 1024 * 1024;

/** How much a flood prints when the terminal takes it in as well. */
export const FLOOD_TERMINAL_BYTES = 64 * 1024 * 1024;

/** How long the terminal is drawn under a flood to count its frames. */
export const FLOOD_DRAW_MS = 4000;

/** Keys typed one straight after another to time their echo. */
export const ECHO_BURST_SAMPLES = 2000;

/** Keys typed with a pause before each, as a person types, and the pause. */
export const ECHO_SPACED_SAMPLES = 300;
export const ECHO_SPACED_PAUSE_MS = 40;

/** Keys typed while another session floods. */
export const ECHO_FLOOD_SAMPLES = 500;

/** Keys typed at a real shell. */
export const ECHO_SHELL_SAMPLES = 200;

/** How long one echo may take before the measurement gives up on it. */
export const ECHO_TIMEOUT_MS = 5000;

/** How long the panes of the window's page are flooded while its renders are counted. */
export const COMMIT_TEST_MS = 2000;

/** How long the page is given to follow a change before its state is looked at. */
export const CHROME_SETTLE_MS = 300;

/** How long the page may take to start and attach to its panes' sessions. */
export const CHROME_START_TIMEOUT_MS = 20_000;

/** How much output each pane is sent at a time while it is flooded. */
export const CHROME_FLOOD_CHUNK_BYTES = 4096;
