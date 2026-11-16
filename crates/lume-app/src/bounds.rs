//! Where a window goes on the desktop when it is opened again.
//!
//! A window's place is kept with its layout: where its top left corner was, in the
//! desktop's own pixels, and how large it was, in the pixels its screen showed it in.
//! The desktop it is opened on again may not be the one it was closed on. A screen may
//! have been unplugged, turned, or set to another scale, so the place is fitted to the
//! screens there are before it is used, and a window is never opened where it cannot
//! be seen or taken hold of.
//!
//! Nothing here asks the system anything: the screens are given, and the answer is a
//! rectangle. The windows module does the asking and the placing.

use lume_layout::Bounds;

use crate::tuning::{BOUNDS_MIN_VISIBLE, WINDOW_MIN_HEIGHT, WINDOW_MIN_WIDTH};

/// One screen: the part of it a window may cover, in the desktop's pixels, and how
/// many of those make one of the pixels a window's size is counted in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// Where to open a window, in the desktop's pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    /// The size of what the window shows, without its frame.
    pub width: u32,
    pub height: u32,
    pub maximised: bool,
    pub fullscreen: bool,
}

/// What a window says of itself when asked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// Its top left corner, in the desktop's pixels.
    pub x: i32,
    pub y: i32,
    /// The size of what it shows, in the desktop's pixels.
    pub width: u32,
    pub height: u32,
    /// The scale of the screen it is on.
    pub scale: f64,
    pub maximised: bool,
    pub fullscreen: bool,
    pub minimised: bool,
}

fn scaled(length: u32, scale: f64) -> i64 {
    (f64::from(length) * scale).round() as i64
}

/// How much of a rectangle lies on a screen: the width and the height of what they
/// share.
fn shared(screen: &Screen, x: i64, y: i64, width: i64, height: i64) -> (i64, i64) {
    let left = x.max(i64::from(screen.x));
    let top = y.max(i64::from(screen.y));
    let right = (x + width).min(i64::from(screen.x) + i64::from(screen.width));
    let bottom = (y + height).min(i64::from(screen.y) + i64::from(screen.height));
    ((right - left).max(0), (bottom - top).max(0))
}

/// Fits a kept place to the screens there are. The first screen is the main one.
/// Without any screen there is nothing to fit to, and the system is left to choose.
///
/// The window goes to the screen most of it was on. If too little of it would show on
/// any screen for it to be seen and taken hold of, it is put in the middle of the main
/// one. It is made no larger than its screen and no smaller than a window may be, and
/// its top edge, which is what a window is moved by, is never above its screen.
pub fn clamp_bounds(bounds: &Bounds, screens: &[Screen]) -> Option<Placement> {
    let main = screens.first()?;
    let x = i64::from(bounds.x);
    let y = i64::from(bounds.y);

    // Each screen is tried at its own scale: the window's size in the desktop's
    // pixels depends on the screen it is on.
    let mut best: Option<(&Screen, i64)> = None;
    for screen in screens {
        let (width, height) = (
            scaled(bounds.width, screen.scale),
            scaled(bounds.height, screen.scale),
        );
        let (across, down) = shared(screen, x, y, width, height);
        let least = scaled(BOUNDS_MIN_VISIBLE, screen.scale);
        if across < least.min(width) || down < least.min(height) {
            continue;
        }
        let area = across * down;
        if best.is_none_or(|(_, most)| area > most) {
            best = Some((screen, area));
        }
    }
    let (screen, lost) = match best {
        Some((screen, _)) => (screen, false),
        None => (main, true),
    };

    let fit = |length: u32, least: u32, most: u32| -> i64 {
        let least = scaled(least, screen.scale).min(i64::from(most));
        scaled(length, screen.scale).clamp(least, i64::from(most))
    };
    let width = fit(bounds.width, WINDOW_MIN_WIDTH, screen.width);
    let height = fit(bounds.height, WINDOW_MIN_HEIGHT, screen.height);

    let (left, top) = (i64::from(screen.x), i64::from(screen.y));
    let (right, bottom) = (
        left + i64::from(screen.width),
        top + i64::from(screen.height),
    );
    let (x, y) = if lost {
        (
            left + (right - left - width) / 2,
            top + (bottom - top - height) / 2,
        )
    } else {
        (x, y.max(top))
    };
    Some(Placement {
        x: i32::try_from(x).unwrap_or(screen.x),
        y: i32::try_from(y).unwrap_or(screen.y),
        width: u32::try_from(width).unwrap_or(screen.width),
        height: u32::try_from(height).unwrap_or(screen.height),
        maximised: bounds.maximised,
        fullscreen: bounds.fullscreen,
    })
}

/// The place to keep for a window, given what it says of itself now and what was kept
/// before.
///
/// A window that fills its screen, or is put away, says where it is for now. The place
/// worth keeping is the one it goes back to, so that one is kept from before, and only
/// what the window is doing is taken from the reading.
pub fn merge_bounds(kept: Option<Bounds>, reading: &Reading) -> Option<Bounds> {
    let scale = if reading.scale > 0.0 {
        reading.scale
    } else {
        1.0
    };
    let logical = |length: u32| (f64::from(length) / scale).round().max(1.0) as u32;
    let own = Bounds {
        x: reading.x,
        y: reading.y,
        width: logical(reading.width),
        height: logical(reading.height),
        maximised: false,
        fullscreen: false,
    };
    if reading.minimised {
        // A window that is put away has no place of its own to tell.
        return kept;
    }
    if reading.maximised || reading.fullscreen {
        let restored = kept.unwrap_or(own);
        return Some(Bounds {
            maximised: reading.maximised,
            fullscreen: reading.fullscreen,
            ..restored
        });
    }
    Some(own)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Screen = Screen {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
        scale: 1.0,
    };

    /// A second screen to the right of the main one, at one and a half times the scale.
    const DENSE: Screen = Screen {
        x: 1920,
        y: 0,
        width: 2560,
        height: 1400,
        scale: 1.5,
    };

    fn bounds(x: i32, y: i32, width: u32, height: u32) -> Bounds {
        Bounds {
            x,
            y,
            width,
            height,
            maximised: false,
            fullscreen: false,
        }
    }

    fn placed(bounds: &Bounds, screens: &[Screen]) -> (i32, i32, u32, u32) {
        let placement = clamp_bounds(bounds, screens).unwrap();
        (placement.x, placement.y, placement.width, placement.height)
    }

    #[test]
    fn a_place_that_is_on_a_screen_is_kept_as_it_is() {
        assert_eq!(
            placed(&bounds(100, 80, 960, 640), &[MAIN]),
            (100, 80, 960, 640)
        );
        // Hanging over an edge is how the user left it.
        assert_eq!(
            placed(&bounds(-300, 80, 960, 640), &[MAIN]),
            (-300, 80, 960, 640)
        );
    }

    #[test]
    fn without_a_screen_the_system_chooses() {
        assert_eq!(clamp_bounds(&bounds(0, 0, 960, 640), &[]), None);
    }

    #[test]
    fn a_window_whose_screen_is_gone_goes_to_the_middle_of_the_main_one() {
        // It was on a screen to the right that is no longer there.
        assert_eq!(
            placed(&bounds(2400, 200, 960, 640), &[MAIN]),
            (480, 200, 960, 640)
        );
        // Far above every screen.
        assert_eq!(
            placed(&bounds(100, -5000, 960, 640), &[MAIN]),
            (480, 200, 960, 640)
        );
    }

    #[test]
    fn a_window_of_which_too_little_shows_is_brought_back() {
        // Most of it is below the screen, but enough shows to take hold of.
        let (x, y, _, _) = placed(&bounds(100, 950, 960, 640), &[MAIN]);
        assert_eq!((x, y), (100, 950));
        // A sliver along the bottom is not enough.
        let (x, y, _, _) = placed(&bounds(100, 1030, 960, 640), &[MAIN]);
        assert_eq!((x, y), (480, 200));
        // Nor is a sliver at the left.
        let (x, _, _, _) = placed(&bounds(-900, 100, 960, 640), &[MAIN]);
        assert_eq!(x, 480);
        // A sliver on one screen and the rest on another is on the other.
        let (x, y, _, _) = placed(&bounds(1900, 1000, 960, 640), &[MAIN, DENSE]);
        assert_eq!((x, y), (1900, 1000));
        // Its top edge above the screen comes down to the screen's.
        let (_, y, _, _) = placed(&bounds(100, -40, 960, 640), &[MAIN]);
        assert_eq!(y, 0);
    }

    #[test]
    fn a_window_is_no_larger_than_its_screen_and_no_smaller_than_a_window_may_be() {
        assert_eq!(
            placed(&bounds(0, 0, 4000, 3000), &[MAIN]),
            (0, 0, 1920, 1040)
        );
        let (_, _, width, height) = placed(&bounds(10, 10, 20, 20), &[MAIN]);
        assert_eq!((width, height), (WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT));
    }

    #[test]
    fn a_window_on_a_denser_screen_is_sized_in_that_screens_pixels() {
        // 960 by 640 at one and a half times the scale is 1440 by 960 of the desktop's
        // pixels, wherever on that screen it is.
        assert_eq!(
            placed(&bounds(2200, 100, 960, 640), &[MAIN, DENSE]),
            (2200, 100, 1440, 960)
        );
        // The same window on the main screen has its own number of pixels.
        assert_eq!(
            placed(&bounds(200, 100, 960, 640), &[MAIN, DENSE]),
            (200, 100, 960, 640)
        );
        // With the dense screen gone it is centred on the main one, at that scale.
        assert_eq!(
            placed(&bounds(2200, 100, 960, 640), &[MAIN]),
            (480, 200, 960, 640)
        );
    }

    #[test]
    fn what_the_window_was_doing_is_kept_with_its_place() {
        let kept = Bounds {
            maximised: true,
            ..bounds(100, 80, 960, 640)
        };
        let placement = clamp_bounds(&kept, &[MAIN]).unwrap();
        assert!(placement.maximised);
        assert!(!placement.fullscreen);
        assert_eq!((placement.x, placement.y), (100, 80));
    }

    fn reading(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Reading {
        Reading {
            x,
            y,
            width,
            height,
            scale,
            maximised: false,
            fullscreen: false,
            minimised: false,
        }
    }

    #[test]
    fn a_windows_size_is_kept_in_the_pixels_its_screen_shows_it_in() {
        let kept = merge_bounds(None, &reading(2200, 100, 1440, 960, 1.5)).unwrap();
        assert_eq!(kept, bounds(2200, 100, 960, 640));
    }

    #[test]
    fn a_window_that_fills_its_screen_keeps_the_place_it_goes_back_to() {
        let before = bounds(100, 80, 960, 640);
        let filling = Reading {
            maximised: true,
            ..reading(-8, -8, 1936, 1056, 1.0)
        };
        assert_eq!(
            merge_bounds(Some(before), &filling),
            Some(Bounds {
                maximised: true,
                ..before
            })
        );
        // Back to itself, its own place is the one to keep again.
        assert_eq!(
            merge_bounds(
                Some(Bounds {
                    maximised: true,
                    ..before
                }),
                &reading(140, 90, 900, 600, 1.0)
            ),
            Some(bounds(140, 90, 900, 600))
        );
        // A window put away says nothing worth keeping.
        let away = Reading {
            minimised: true,
            ..reading(-32000, -32000, 160, 28, 1.0)
        };
        assert_eq!(merge_bounds(Some(before), &away), Some(before));
    }
}
