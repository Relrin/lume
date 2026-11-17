//! The layout's tuning values. Each is defined here and nowhere else.

/// What a whole is divided into. A split's ratios are counted in these and add up to
/// exactly this, and a floating pane's rectangle is measured in them against the area
/// it floats over. Whole numbers keep layouts exactly comparable: dividing and merging
/// splits again and again cannot make the shares drift.
pub const UNITS: u32 = 1_000_000;

/// The smallest share of a split one of its children may have: a twentieth.
pub const SHARE_MIN: u32 = UNITS / 20;

/// How far the ratios of a resize may be from adding up to a whole and still be taken.
/// They are brought to a whole exactly; ratios further off are a mistake, not rounding.
pub const RATIO_TOLERANCE: u32 = 1_000;

/// The most children one split may have.
pub const SPLIT_CHILDREN_MAX: usize = 20;

/// The most panes one tab may hold.
pub const TAB_PANES_MAX: usize = 64;

/// How deep the splits of one tab may nest.
pub const TREE_DEPTH_MAX: usize = 16;

/// The most tabs one window may hold.
pub const TABS_MAX: usize = 256;

/// The most panes that may float over one window.
pub const FLOATING_MAX: usize = 32;

/// The most windows one workspace may have.
pub const WINDOWS_MAX: usize = 16;

/// How many panes a window remembers having focused, most recent first.
pub const FOCUS_HISTORY_MAX: usize = 32;

/// The smallest a floating pane may be along either side.
pub const FLOAT_MIN: u32 = UNITS / 10;

/// The side of a pane that floats without being told where.
pub const FLOAT_DEFAULT: u32 = UNITS * 6 / 10;

/// How far each further pane floated without a place is moved from the one before.
pub const FLOAT_CASCADE: u32 = UNITS / 40;

/// The narrowest and the widest a dock area may be, and how wide a new one is, in
/// logical pixels.
pub const DOCK_SIZE_MIN: u16 = 120;
pub const DOCK_SIZE_MAX: u16 = 800;
pub const DOCK_SIZE_DEFAULT: u16 = 240;

/// The most panels one dock area may hold.
pub const DOCK_PANELS_MAX: usize = 16;

/// The longest name a tab or a pane may be given, in characters.
pub const TITLE_MAX_CHARS: usize = 256;

/// How many changes a reader of an authority's changes may fall behind before it is
/// given the whole layout instead.
pub const CHANGE_BACKLOG: usize = 256;

/// The most kept groups a workspace keeps put away.
pub const SHELF_MAX: usize = 32;

/// The version of the file a workspace is saved in.
pub const WORKSPACE_FILE_VERSION: u32 = 2;

/// How many files that could not be read are kept beside a workspace's file.
pub const CORRUPT_KEPT: usize = 3;

/// The longest name a workspace may have, in characters.
pub const WORKSPACE_NAME_MAX_CHARS: usize = 64;
