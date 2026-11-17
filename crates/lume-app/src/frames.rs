//! What the application draws of its windows' frames from the configuration: whose
//! title bar a window has, and how round the frame's corners are. A change of the
//! configuration has the windows redrawn only in what it changed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use lume_config::WindowCorners;

use crate::configsvc::ConfigService;
use crate::control::Shell;

/// The frames the windows were last drawn with.
pub struct Frames {
    custom: AtomicBool,
    corners: Mutex<WindowCorners>,
}

impl Frames {
    /// The frames the windows are opened with, which the configuration in use decides.
    pub fn new(config: &ConfigService) -> Self {
        Self {
            custom: AtomicBool::new(config.title_bar_custom()),
            corners: Mutex::new(config.current().config.window.corners),
        }
    }

    /// Follows the configuration in use: what changed in it is redrawn.
    pub fn follow(&self, config: &ConfigService, shell: &dyn Shell) {
        let custom = config.title_bar_custom();
        if self.custom.swap(custom, Ordering::Relaxed) != custom {
            shell.set_title_bar(custom);
        }
        let corners = config.current().config.window.corners;
        let mut held = self.corners.lock().unwrap_or_else(PoisonError::into_inner);
        if *held != corners {
            *held = corners;
            shell.set_corners(corners);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::{FakeShell, TempDir};

    #[test]
    fn a_change_to_the_frames_redraws_the_windows_and_nothing_else_does() {
        let dir = TempDir::new("frames");
        let config = ConfigService::load(dir.dirs(), None);
        let frames = Frames::new(&config);
        let shell = FakeShell::default();

        config.set("terminal.cursor-blink", &json!(false)).unwrap();
        frames.follow(&config, &shell);
        assert!(shell.corners().is_empty());
        assert!(shell.title_bars().is_empty());

        config.set("window.corners", &json!("square")).unwrap();
        frames.follow(&config, &shell);
        frames.follow(&config, &shell);
        assert_eq!(shell.corners(), vec![WindowCorners::Square]);

        config.unset("window.corners").unwrap();
        frames.follow(&config, &shell);
        assert_eq!(
            shell.corners(),
            vec![WindowCorners::Square, WindowCorners::Round]
        );

        let custom = config.title_bar_custom();
        let other = if custom { "system" } else { "custom" };
        config.set("window.title-bar", &json!(other)).unwrap();
        frames.follow(&config, &shell);
        assert_eq!(shell.title_bars(), vec![!custom]);
    }
}
