//! The latency tool's desktop on Linux under X11: XTEST for keys and `GetImage` for the
//! screen.
//!
//! A Wayland session gives no program another window's pixels or a way to type into it
//! without a portal asking the user, so the tool asks for an X11 session instead of
//! measuring through XWayland, which would add a step native Wayland windows do not have.

use anyhow::{Context, Result, bail};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, ImageFormat, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode, Window,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::{Desktop, Key, Rect};

/// The key symbols of the letter x and of BackSpace.
const XK_X: u32 = 0x0078;
const XK_BACKSPACE: u32 = 0xff08;

pub struct X11 {
    connection: RustConnection,
    root: Window,
    screen: (u16, u16),
    active_window: u32,
    x: Keycode,
    backspace: Keycode,
}

impl X11 {
    pub fn open() -> Result<X11> {
        if std::env::var("XDG_SESSION_TYPE").is_ok_and(|session| session == "wayland") {
            bail!(
                "this is a Wayland session, where no program may read another window's pixels \
                 or type into it; log in to an X11 session to measure"
            );
        }
        let (connection, number) =
            x11rb::connect(None).context("connecting to the X server (is DISPLAY set?)")?;
        let (root, screen) = {
            let screen = &connection.setup().roots[number];
            (
                screen.root,
                (screen.width_in_pixels, screen.height_in_pixels),
            )
        };
        connection
            .xtest_get_version(2, 2)?
            .reply()
            .context("the X server has no XTEST extension, which types the keys")?;
        let active_window = connection
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let x = keycode(&connection, XK_X)?;
        let backspace = keycode(&connection, XK_BACKSPACE)?;
        Ok(X11 {
            connection,
            root,
            screen,
            active_window,
            x,
            backspace,
        })
    }
}

/// The key that types `keysym` with the keyboard's present layout.
fn keycode(connection: &RustConnection, keysym: u32) -> Result<Keycode> {
    let setup = connection.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = connection
        .get_keyboard_mapping(min, max - min + 1)?
        .reply()?;
    let per_key = usize::from(map.keysyms_per_keycode.max(1));
    map.keysyms
        .chunks(per_key)
        .position(|syms| syms.contains(&keysym))
        .map(|index| min + index as u8)
        .with_context(|| format!("no key on this keyboard types the key symbol {keysym:#x}"))
}

impl Desktop for X11 {
    fn focused(&mut self) -> Result<(u64, Rect)> {
        let reply = self
            .connection
            .get_property(false, self.root, self.active_window, AtomEnum::WINDOW, 0, 1)?
            .reply()?;
        let window = reply
            .value32()
            .and_then(|mut values| values.next())
            .filter(|&window| window != 0)
            .context("the window manager does not say which window has the focus")?;
        let geometry = self.connection.get_geometry(window)?.reply()?;
        let origin = self
            .connection
            .translate_coordinates(window, self.root, 0, 0)?
            .reply()?;
        // Cut to the screen: the server refuses to read pixels outside it.
        let window_area = Rect {
            x: i32::from(origin.dst_x),
            y: i32::from(origin.dst_y),
            width: u32::from(geometry.width),
            height: u32::from(geometry.height),
        };
        let screen = Rect {
            x: 0,
            y: 0,
            width: u32::from(self.screen.0),
            height: u32::from(self.screen.1),
        };
        Ok((u64::from(window), window_area.grown(0, screen)))
    }

    fn title(&mut self, window: u64) -> Result<String> {
        let window = window as Window;
        let name = self
            .connection
            .intern_atom(false, b"_NET_WM_NAME")?
            .reply()?
            .atom;
        let utf8 = self
            .connection
            .intern_atom(false, b"UTF8_STRING")?
            .reply()?
            .atom;
        let modern = self
            .connection
            .get_property(false, window, name, utf8, 0, 1024)?
            .reply()?;
        if !modern.value.is_empty() {
            return Ok(String::from_utf8_lossy(&modern.value).into_owned());
        }
        let legacy = self
            .connection
            .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)?
            .reply()?;
        Ok(String::from_utf8_lossy(&legacy.value).into_owned())
    }

    fn capture(&mut self, area: Rect) -> Result<Vec<u8>> {
        let image = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                area.x as i16,
                area.y as i16,
                area.width as u16,
                area.height as u16,
                !0,
            )?
            .reply()?;
        if image.data.len() != area.width as usize * area.height as usize * 4 {
            bail!("the screen is not 24 or 32 bits a pixel, which is all this tool reads");
        }
        Ok(image.data)
    }

    fn press(&mut self, key: Key) -> Result<()> {
        let code = match key {
            Key::X => self.x,
            Key::Backspace => self.backspace,
        };
        for kind in [KEY_PRESS_EVENT, KEY_RELEASE_EVENT] {
            self.connection
                .xtest_fake_input(kind, code, CURRENT_TIME, self.root, 0, 0, 0)?;
        }
        self.connection.flush()?;
        Ok(())
    }
}
