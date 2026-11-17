//! The latency tool's desktop on Windows: `SendInput` for keys and GDI for the screen.
//!
//! GDI reads the desktop as the compositor last put it together, which is the frame the
//! display shows next. A capture of the small area at the cursor takes well under a
//! millisecond, and the tool reports how long it took.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
    HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW,
    SendInput, VIRTUAL_KEY, VK_BACK,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetForegroundWindow, GetWindowTextW,
};

use super::{Desktop, Key, Rect};

/// The virtual key of the letter X.
const VK_X: VIRTUAL_KEY = 0x58;

/// The screen's device context, a memory context to copy it into, and the bitmap there.
pub struct Win32 {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    /// The bitmap the memory context had before ours, put back before ours is deleted.
    original: HGDIOBJ,
    size: (u32, u32),
}

impl Win32 {
    pub fn open() -> Result<Win32> {
        // SAFETY: plain calls with no pointers of ours. Per-monitor awareness makes window
        // areas and captures use the same physical pixels on a scaled display; it fails
        // only when awareness was already set, which leaves it as good.
        let (screen, memory) = unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let screen = GetDC(null_mut());
            (screen, CreateCompatibleDC(screen))
        };
        if screen.is_null() || memory.is_null() {
            bail!("could not open the screen for reading");
        }
        Ok(Win32 {
            screen,
            memory,
            bitmap: null_mut(),
            original: null_mut(),
            size: (0, 0),
        })
    }

    /// Makes the memory bitmap `width` by `height`, if it is not already.
    fn bitmap_of(&mut self, width: u32, height: u32) -> Result<()> {
        if self.size == (width, height) && !self.bitmap.is_null() {
            return Ok(());
        }
        // SAFETY: both contexts are ours and alive; the old bitmap is deselected before
        // it is deleted.
        unsafe {
            let bitmap = CreateCompatibleBitmap(self.screen, width as i32, height as i32);
            if bitmap.is_null() {
                bail!("could not make a bitmap of {width} by {height}");
            }
            let previous = SelectObject(self.memory, bitmap);
            if self.bitmap.is_null() {
                self.original = previous;
            } else {
                DeleteObject(self.bitmap);
            }
            self.bitmap = bitmap;
        }
        self.size = (width, height);
        Ok(())
    }
}

impl Drop for Win32 {
    fn drop(&mut self) {
        // SAFETY: everything here was made by `open` and `bitmap_of` and is released once.
        unsafe {
            if !self.bitmap.is_null() {
                SelectObject(self.memory, self.original);
                DeleteObject(self.bitmap);
            }
            DeleteDC(self.memory);
            ReleaseDC(null_mut(), self.screen);
        }
    }
}

impl Desktop for Win32 {
    fn focused(&mut self) -> Result<(u64, Rect)> {
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let mut origin = POINT { x: 0, y: 0 };
        // SAFETY: the window handle is only passed back to the system, which checks it;
        // the two out-pointers are to locals.
        let window = unsafe {
            let window = GetForegroundWindow();
            if window.is_null()
                || GetClientRect(window, &mut client) == 0
                || ClientToScreen(window, &mut origin) == 0
            {
                bail!("no window has the focus");
            }
            window
        };
        Ok((
            window as u64,
            Rect {
                x: origin.x,
                y: origin.y,
                width: (client.right - client.left).max(0) as u32,
                height: (client.bottom - client.top).max(0) as u32,
            },
        ))
    }

    fn title(&mut self, window: u64) -> Result<String> {
        let mut text = [0u16; 512];
        // SAFETY: the handle came from `focused` and the system checks it; `text` holds
        // the number of characters given.
        let length = unsafe { GetWindowTextW(window as _, text.as_mut_ptr(), text.len() as i32) };
        Ok(String::from_utf16_lossy(&text[..length.max(0) as usize]))
    }

    fn capture(&mut self, area: Rect) -> Result<Vec<u8>> {
        self.bitmap_of(area.width, area.height)?;
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.width as i32,
                // Negative: rows from the top down.
                biHeight: -(area.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default()],
        };
        let mut pixels = vec![0u8; area.width as usize * area.height as usize * 4];
        // SAFETY: the bitmap is `area`'s size and selected into the memory context, and
        // `pixels` holds exactly its rows at four bytes a pixel.
        let rows = unsafe {
            if BitBlt(
                self.memory,
                0,
                0,
                area.width as i32,
                area.height as i32,
                self.screen,
                area.x,
                area.y,
                SRCCOPY,
            ) == 0
            {
                bail!("could not copy the screen");
            }
            GetDIBits(
                self.memory,
                self.bitmap,
                0,
                area.height,
                pixels.as_mut_ptr().cast::<c_void>(),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        if rows != area.height as i32 {
            bail!("could not read the copied screen");
        }
        Ok(pixels)
    }

    fn press(&mut self, key: Key) -> Result<()> {
        let code = match key {
            Key::X => VK_X,
            Key::Backspace => VK_BACK,
        };
        // A keyboard sends the key's scan code with its virtual key, and webviews take
        // the physical key (`KeyboardEvent.code`) from it: without one, a terminal in a
        // webview sees text keys but not Backspace.
        // SAFETY: a plain call with no pointers.
        let scan = unsafe { MapVirtualKeyW(u32::from(code), MAPVK_VK_TO_VSC) } as u16;
        let event = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: code,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let events = [event(0), event(KEYEVENTF_KEYUP)];
        // SAFETY: `events` is an array of two initialised inputs of the size given.
        let sent = unsafe { SendInput(2, events.as_ptr(), size_of::<INPUT>() as i32) };
        if sent != 2 {
            bail!(
                "the key could not be sent; a window run as administrator does not take keys \
                 from a program that is not"
            );
        }
        Ok(())
    }
}
