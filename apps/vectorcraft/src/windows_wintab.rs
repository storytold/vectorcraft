//! Windows: an airbrush pen's finger wheel. `POINTER_PEN_INFO` has no field for it
//! (<https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info>), so
//! [`crate::windows_stylus`] keeps reading the pressure, end, tilt and rotation from pointer
//! messages, and this module reads only the wheel from the tablet driver's Wintab interface, as
//! its tangential pressure (`pkTangentPressure`), when the driver installs one. The Wintab
//! reference describes the context, packet and message calls used here
//! (<https://developer-docs.wacom.com/docs/icbt/windows/wintab/wintab-reference/>), and the
//! Windows Ink comparison says the wheel comes only through Wintab
//! (<https://developer-support.wacom.com/hc/en-us/articles/12844630619415-Windows-Ink>).
//!
//! - `Wintab32.dll` is loaded at run time from the system directory only, the first time a pen
//!   comes over a window. Without it, without one of its exports, or when a call fails, the pens
//!   read no wheel and draw as before; the user sees no error.
//! - The context is opened from the driver's default context for the window that owns the pen's
//!   window ([`configure`]), with packet messages on and the system cursor off: it moves no cursor
//!   and makes no press. When the pen comes over another top-level window, such as a separate
//!   document window, the context closes and opens again for that window, and the readings start
//!   afresh ([`next_context`]). Each `WT_PACKET` for that window and context is read with `WTPacket` into
//!   a buffer larger than any packet, and decoded from the fields the context asked for
//!   ([`decode`]).
//! - Only a cursor the driver reports as an airbrush (`(CSR_TYPE & 0x0F06) == 0x0902`, from the
//!   Wintab basics page: <https://developer-docs.wacom.com/docs/icbt/windows/wintab/wintab-basics/>)
//!   whose packets carry the tangential pressure, on a device whose `DVC_TPRESSURE` range is valid,
//!   reads a wheel ([`wheel_axis`]). The wheel is the value's place in that range, 0..1
//!   ([`Axis::wheel`]); a value outside it reads as none.
//! - A pointer message of a pen takes the wheel of the airbrush read since the readings started
//!   afresh, however long ago its last packet came, since a pen held still sends none. When
//!   several cursors were read, it takes the one whose latest packet came within [`MAX_SKEW_MS`] of
//!   the message, and none when two are that recent or none is ([`Readings::wheel_at`]). As on
//!   macOS, 0 counts as no wheel until the pen has reported another value since it came near: the
//!   readings start afresh when a cursor
//!   comes near or leaves (`WT_PROXIMITY`), another cursor comes (`WT_CSRCHANGE`), the pen turns
//!   to its other end, a press is canceled, the pen leaves the window or detection range, or the
//!   context closes. These rules are VectorCraft's own.
//!
//! The context closes when the driver closes it or its information changes (`WT_INFOCHANGE`), and
//! is dropped when its window no longer exists; a later pen message opens it again.
#![cfg_attr(not(windows), allow(dead_code))]

/// Wintab's default message base (`WT_DEFBASE`, `wintab.h`), which [`configure`] sets.
const WT_DEFBASE: u32 = 0x7FF0;
/// `WT_PACKET`, `WT_CTXCLOSE`, `WT_PROXIMITY`, `WT_INFOCHANGE` and `WT_CSRCHANGE` at that base.
const WT_PACKET: u32 = WT_DEFBASE;
const WT_CTXCLOSE: u32 = WT_DEFBASE + 2;
const WT_PROXIMITY: u32 = WT_DEFBASE + 5;
const WT_INFOCHANGE: u32 = WT_DEFBASE + 6;
const WT_CSRCHANGE: u32 = WT_DEFBASE + 7;

/// `CXO_SYSTEM`, `CXO_PEN`, `CXO_MESSAGES` and `CXO_CSRMESSAGES` (`wintab.h`), in `lcOptions`.
const CXO_SYSTEM: u32 = 0x0001;
const CXO_PEN: u32 = 0x0002;
const CXO_MESSAGES: u32 = 0x0004;
const CXO_CSRMESSAGES: u32 = 0x0008;

/// The packet fields (`PK_*`, `pktdef.h`) in the order a packet holds them, each with its size in
/// bytes: `pkContext` is a handle, `pkOrientation` and `pkRotation` three 32-bit values each, and
/// the others one 32-bit value.
const PK_CONTEXT: u32 = 0x0001;
const PK_STATUS: u32 = 0x0002;
const PK_TIME: u32 = 0x0004;
const PK_CHANGED: u32 = 0x0008;
const PK_SERIAL_NUMBER: u32 = 0x0010;
const PK_CURSOR: u32 = 0x0020;
const PK_BUTTONS: u32 = 0x0040;
const PK_X: u32 = 0x0080;
const PK_Y: u32 = 0x0100;
const PK_Z: u32 = 0x0200;
const PK_NORMAL_PRESSURE: u32 = 0x0400;
const PK_TANGENT_PRESSURE: u32 = 0x0800;
const PK_ORIENTATION: u32 = 0x1000;
const PK_ROTATION: u32 = 0x2000;
const FIELDS: [(u32, usize); 14] = [
    (PK_CONTEXT, std::mem::size_of::<usize>()),
    (PK_STATUS, 4),
    (PK_TIME, 4),
    (PK_CHANGED, 4),
    (PK_SERIAL_NUMBER, 4),
    (PK_CURSOR, 4),
    (PK_BUTTONS, 4),
    (PK_X, 4),
    (PK_Y, 4),
    (PK_Z, 4),
    (PK_NORMAL_PRESSURE, 4),
    (PK_TANGENT_PRESSURE, 4),
    (PK_ORIENTATION, 12),
    (PK_ROTATION, 12),
];

/// The fields the context asks for (`lcPktData`), all absolute: the status, the cursor, the
/// position and the tangential pressure.
const PACKET_DATA: u32 = PK_STATUS | PK_CURSOR | PK_X | PK_Y | PK_TANGENT_PRESSURE;
/// The changes that make a packet (`lcMoveMask`): a move, or a turn of the wheel in place.
const MOVE_MASK: u32 = PK_X | PK_Y | PK_TANGENT_PRESSURE;
/// The bytes `WTPacket` writes into: more than a packet of every field.
const PACKET_BUFFER: usize = 256;

/// `TPS_PROXIMITY` and `TPS_QUEUE_ERR` (`wintab.h`), in `pkStatus`: the cursor is out of the
/// context, and the queue overflowed.
const TPS_PROXIMITY: u32 = 0x0001;
const TPS_QUEUE_ERR: u32 = 0x0002;

/// The `LOGCONTEXTW` (`wintab.h`) as 32-bit words: a 40-character name, then `lcOptions`,
/// `lcStatus`, `lcLocks`, `lcMsgBase`, `lcDevice`, `lcPktRate`, `lcPktData`, `lcPktMode`,
/// `lcMoveMask`, `lcBtnDnMask` and `lcBtnUpMask`, and 22 words of extents and sensitivities.
const LOGCONTEXT_WORDS: usize = 53;
const _: () = assert!(LOGCONTEXT_WORDS * 4 == 212, "sizeof(LOGCONTEXTW)");
const LC_OPTIONS: usize = 20;
const LC_MSG_BASE: usize = 23;
const LC_PKT_DATA: usize = 26;
const LC_PKT_MODE: usize = 27;
const LC_MOVE_MASK: usize = 28;
const LC_BTN_DN_MASK: usize = 29;
const LC_BTN_UP_MASK: usize = 30;

/// `WTInfoW` categories and indexes (`wintab.h`): the interface's device count, the default
/// context, a device's first cursor, cursor count and tangential-pressure range, and a cursor's
/// packet fields and type.
const WTI_INTERFACE: u32 = 1;
const IFC_NDEVICES: u32 = 4;
const WTI_DEFCONTEXT: u32 = 3;
const WTI_DEVICES: u32 = 100;
const DVC_NCSRTYPES: u32 = 3;
const DVC_FIRSTCSR: u32 = 4;
const DVC_TPRESSURE: u32 = 16;
const WTI_CURSORS: u32 = 200;
const CSR_PKTDATA: u32 = 3;
const CSR_TYPE: u32 = 20;
/// Cursor and device numbers stay below the next category's base.
const MAX_INDEX: u32 = 100;
/// At most this many devices are looked through for a cursor's device.
const MAX_DEVICES: u32 = 16;

/// A packet and a pointer message at most this far apart, in the message clock's milliseconds,
/// belong together.
const MAX_SKEW_MS: u32 = 200;
/// At most this many cursors' readings are kept.
const MAX_READINGS: usize = 4;

/// Sets up the driver's default context, `words`, for the wheel: packet and cursor messages on,
/// the system cursor and pen messages off, the [`PACKET_DATA`] fields in absolute mode, packets on
/// a move or a turn of the wheel, and no button events.
fn configure(words: &mut [u32; LOGCONTEXT_WORDS]) {
    words[LC_OPTIONS] = (words[LC_OPTIONS] | CXO_MESSAGES | CXO_CSRMESSAGES) & !(CXO_SYSTEM | CXO_PEN);
    words[LC_MSG_BASE] = WT_DEFBASE;
    words[LC_PKT_DATA] = PACKET_DATA;
    words[LC_PKT_MODE] = 0;
    words[LC_MOVE_MASK] = MOVE_MASK;
    words[LC_BTN_DN_MASK] = 0;
    words[LC_BTN_UP_MASK] = 0;
}

/// The size in bytes of a packet holding the fields of `mask`.
fn packet_size(mask: u32) -> usize {
    FIELDS.iter().filter(|(f, _)| mask & f != 0).map(|(_, n)| n).sum()
}

/// The 32-bit field `field` of a packet holding the fields of `mask`, or `None` when the mask
/// leaves it out, it is not a 32-bit field, or `bytes` is too short.
fn field(bytes: &[u8], mask: u32, field: u32) -> Option<u32> {
    if mask & field == 0 || bytes.len() < packet_size(mask) {
        return None;
    }
    let mut at = 0;
    for &(f, n) in &FIELDS {
        if f == field {
            let word = bytes.get(at..at + 4).filter(|_| n == 4)?;
            return Some(u32::from_ne_bytes(word.try_into().ok()?));
        }
        if mask & f != 0 {
            at += n;
        }
    }
    None
}

/// What a packet says about the wheel.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Packet {
    status: u32,
    cursor: u32,
    tangent: u32,
}

/// The status, cursor and tangential pressure of a packet holding the fields of `mask`.
fn decode(bytes: &[u8], mask: u32) -> Option<Packet> {
    Some(Packet { status: field(bytes, mask, PK_STATUS)?, cursor: field(bytes, mask, PK_CURSOR)?, tangent: field(bytes, mask, PK_TANGENT_PRESSURE)? })
}

/// A device's tangential-pressure range (`AXIS`'s `axMin` and `axMax`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Axis {
    min: i32,
    max: i32,
}

impl Axis {
    /// The wheel, 0..1, for the absolute value `raw`: 0 at `axMin`, 1 at `axMax`. `pktdef.h` gives
    /// an absolute tangential pressure as unsigned; with a range below 0 it is read as signed. A
    /// value outside the range reads as none.
    fn wheel(self, raw: u32) -> Option<f32> {
        let (min, max) = (i64::from(self.min), i64::from(self.max));
        if min >= max {
            return None;
        }
        let v = if min < 0 { i64::from(raw as i32) } else { i64::from(raw) };
        (min..=max).contains(&v).then(|| ((v - min) as f64 / (max - min) as f64) as f32)
    }
}

/// Whether a cursor of this `CSR_TYPE` is an airbrush.
fn is_airbrush(csr_type: u32) -> bool {
    csr_type & 0x0F06 == 0x0902
}

/// The wheel range of a cursor of `csr_type` whose packets carry the fields of `csr_pktdata`, on a
/// device whose `DVC_TPRESSURE` reads as `axis` (`axMin`, `axMax`, `axUnits`, `axResolution`):
/// `None` unless it is an airbrush with a tangential pressure and the range is not empty.
fn wheel_axis(csr_type: u32, csr_pktdata: u32, axis: Option<[u32; 4]>) -> Option<Axis> {
    let [min, max, ..] = axis?;
    let axis = Axis { min: min as i32, max: max as i32 };
    (is_airbrush(csr_type) && csr_pktdata & PK_TANGENT_PRESSURE != 0 && axis.min < axis.max).then_some(axis)
}

/// The device of `cursor`, from each device's first cursor and cursor count (`DVC_FIRSTCSR`,
/// `DVC_NCSRTYPES`), or `None` when no device or more than one has it.
fn device_of(cursor: u32, devices: &[(u32, u32)]) -> Option<usize> {
    let mut found = devices.iter().enumerate().filter(|(_, (first, n))| first.checked_add(*n).is_some_and(|end| (*first..end).contains(&cursor)));
    let (d, _) = found.next()?;
    found.next().is_none().then_some(d)
}

/// Whether two times of the message clock, which wraps, are at most [`MAX_SKEW_MS`] apart.
fn near(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32).unsigned_abs() <= MAX_SKEW_MS
}

/// The latest packet of a cursor near the context.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    cursor: u32,
    /// The cursor's wheel range, `None` for a cursor without a wheel.
    axis: Option<Axis>,
    raw: u32,
    /// The message time of the packet.
    time: u32,
    /// The cursor has reported a value other than 0 since the readings started afresh.
    moved: bool,
}

/// The latest packet of each cursor near the context, until the readings start afresh.
#[derive(Clone, Debug, Default)]
struct Readings {
    list: Vec<Reading>,
    /// The end of the pen the last pointer message read, `Some(true)` for the eraser end.
    eraser: Option<bool>,
}

impl Readings {
    /// A packet of `p.cursor`, whose wheel range is `axis`, at the message time `time`. A packet of
    /// a cursor out of the context, or after the queue overflowed, drops that cursor's reading.
    fn packet(&mut self, p: Packet, axis: Option<Axis>, time: u32) {
        let at = self.list.iter().position(|r| r.cursor == p.cursor);
        if p.status & (TPS_PROXIMITY | TPS_QUEUE_ERR) != 0 {
            if let Some(i) = at {
                self.list.remove(i);
            }
            return;
        }
        let moved = at.is_some_and(|i| self.list[i].moved && self.list[i].axis == axis) || p.tangent != 0;
        let reading = Reading { cursor: p.cursor, axis, raw: p.tangent, time, moved };
        match at {
            Some(i) => self.list[i] = reading,
            None => {
                if self.list.len() >= MAX_READINGS {
                    self.list.remove(0);
                }
                self.list.push(reading);
            }
        }
    }

    /// Start afresh: the pen came near or left, another cursor came, a press was canceled, or the
    /// context closed.
    fn forget(&mut self) {
        self.list.clear();
        self.eraser = None;
    }

    /// The wheel for a pointer message of a pen at the message time `time` with its `eraser` end,
    /// if the cursor it comes from is an airbrush: the one cursor read since the readings started
    /// afresh, however long ago its last packet came (a pen held still sends none), or, when
    /// several were, the one whose latest packet is within [`MAX_SKEW_MS`] of the message. `None`
    /// when no cursor, or more than one, is that recent, or the cursor has read only 0 since the
    /// readings started afresh. A turn to the other end starts them afresh first.
    fn wheel_at(&mut self, time: u32, eraser: bool) -> Option<f32> {
        if self.eraser.is_some_and(|e| e != eraser) {
            self.forget();
        }
        self.eraser = Some(eraser);
        let r = match self.list.as_slice() {
            [one] => one,
            list => {
                let mut near_time = list.iter().filter(|r| near(time, r.time));
                let r = near_time.next()?;
                if near_time.next().is_some() {
                    return None;
                }
                r
            }
        };
        if !r.moved {
            return None;
        }
        r.axis?.wheel(r.raw)
    }
}

/// What a pen message does to the open context before it is opened for the pen's window.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Current {
    /// Nothing is open, or the context is the pen window's.
    Keep,
    /// The context's window is gone: drop the handle without a call on it.
    Drop,
    /// The context is another window's: close it with `WTClose`.
    Close,
}

/// The context step for a pen message to a window owned by `owner`: what happens to the open
/// context, whose window is `current` with whether that window still exists, and whether a context
/// is opened for `owner`. The context follows the window the pen draws in. An open is tried once
/// for a window until another window is tried or the driver closes or changes the context
/// (`tried`).
fn next_context(current: Option<(usize, bool)>, tried: Option<usize>, owner: usize) -> (Current, bool) {
    let step = match current {
        Some((_, false)) => Current::Drop,
        Some((w, true)) if w == owner => return (Current::Keep, false),
        Some(_) => Current::Close,
        None => Current::Keep,
    };
    (step, tried != Some(owner) || step != Current::Keep)
}

/// What a message does to the readings and the context.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Message {
    /// A packet with this serial number is waiting for the context `lparam` names.
    Packet {
        serial: u32,
        ctx: usize,
    },
    /// The readings start afresh for the context it names.
    Forget {
        ctx: usize,
    },
    /// The context it names has closed.
    Closed {
        ctx: usize,
    },
    /// The driver's information changed: the context closes and opens again.
    InfoChange,
    Other,
}

/// What the message `message` with `wparam` and `lparam` is to the Wintab reader. `WT_PACKET`
/// and `WT_CSRCHANGE` name the context in `lParam`, `WT_PROXIMITY` and `WT_CTXCLOSE` in `wParam`.
fn message(message: u32, wparam: usize, lparam: isize) -> Message {
    match message {
        WT_PACKET => Message::Packet { serial: wparam as u32, ctx: lparam as usize },
        WT_CSRCHANGE => Message::Forget { ctx: lparam as usize },
        WT_PROXIMITY => Message::Forget { ctx: wparam },
        WT_CTXCLOSE => Message::Closed { ctx: wparam },
        WT_INFOCHANGE => Message::InfoChange,
        _ => Message::Other,
    }
}

#[cfg(windows)]
pub use win::{forget, msg_hook, pen_wheel};

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOTOWNER, GetAncestor, IsWindow, MSG};
    use windows::core::{s, w};

    use super::{
        Axis, CSR_PKTDATA, CSR_TYPE, Current, DVC_FIRSTCSR, DVC_NCSRTYPES, DVC_TPRESSURE, IFC_NDEVICES, LOGCONTEXT_WORDS, MAX_DEVICES, MAX_INDEX,
        MAX_READINGS, Message, PACKET_BUFFER, PACKET_DATA, Readings, WTI_CURSORS, WTI_DEFCONTEXT, WTI_DEVICES, WTI_INTERFACE, configure, decode,
        device_of, message, next_context, wheel_axis,
    };

    /// `WTInfoW`, `WTOpenW`, `WTClose` and `WTPacket` (`wintab.h`).
    type WtInfo = unsafe extern "system" fn(u32, u32, *mut c_void) -> u32;
    type WtOpen = unsafe extern "system" fn(*mut c_void, *mut c_void, i32) -> *mut c_void;
    type WtClose = unsafe extern "system" fn(*mut c_void) -> i32;
    type WtPacket = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;

    #[derive(Clone, Copy)]
    struct Api {
        info: WtInfo,
        open: WtOpen,
        close: WtClose,
        packet: WtPacket,
    }

    /// The bytes a packet is read into.
    #[repr(C, align(8))]
    struct PacketBuffer([u8; PACKET_BUFFER]);

    /// An open context and the window that owns it.
    struct Context {
        handle: *mut c_void,
        hwnd: *mut c_void,
    }

    #[derive(Default)]
    struct State {
        context: Option<Context>,
        /// The window an open was last tried for ([`next_context`]).
        tried: Option<*mut c_void>,
        /// Each cursor's wheel range, as the driver reported it.
        cursors: Vec<(u32, Option<Axis>)>,
        readings: Readings,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// The Wintab exports, or `None` without the driver's `Wintab32.dll` in the system directory.
    fn api() -> Option<Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        *API.get_or_init(|| {
            // SAFETY: Wintab32.dll is loaded from the system directory only, and stays loaded for
            // the process, so the function pointers below stay valid.
            let module = unsafe { LoadLibraryExW(w!("Wintab32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
            // SAFETY: the names are NUL-terminated, and the module is loaded.
            let (info, open, close, packet) = unsafe {
                (
                    GetProcAddress(module, s!("WTInfoW"))?,
                    GetProcAddress(module, s!("WTOpenW"))?,
                    GetProcAddress(module, s!("WTClose"))?,
                    GetProcAddress(module, s!("WTPacket"))?,
                )
            };
            type Proc = unsafe extern "system" fn() -> isize;
            // SAFETY: the exports have these signatures (`wintab.h`).
            unsafe {
                Some(Api {
                    info: std::mem::transmute::<Proc, WtInfo>(info),
                    open: std::mem::transmute::<Proc, WtOpen>(open),
                    close: std::mem::transmute::<Proc, WtClose>(close),
                    packet: std::mem::transmute::<Proc, WtPacket>(packet),
                })
            }
        })
    }

    /// `WTInfoW`'s answer for `category` and `index`, when it is exactly `N` 32-bit words.
    fn info<const N: usize>(api: Api, category: u32, index: u32) -> Option<[u32; N]> {
        // SAFETY: a null buffer asks for the size only.
        let size = unsafe { (api.info)(category, index, std::ptr::null_mut()) } as usize;
        if N > 128 || size != N * 4 {
            return None;
        }
        // Room for more than asked, should the driver write more than it said.
        let mut buf = [0u32; 128];
        // SAFETY: `buf` holds 512 bytes, more than the size the driver gave.
        let wrote = unsafe { (api.info)(category, index, buf.as_mut_ptr().cast()) } as usize;
        let mut out = [0u32; N];
        out.copy_from_slice(buf.get(..N)?);
        (wrote == N * 4).then_some(out)
    }

    /// The wheel range of `cursor`, `None` for a cursor without one or one the driver does not
    /// describe.
    fn cursor_axis(api: Api, cursor: u32) -> Option<Axis> {
        if cursor >= MAX_INDEX {
            return None;
        }
        let [csr_type] = info::<1>(api, WTI_CURSORS + cursor, CSR_TYPE)?;
        let [csr_pktdata] = info::<1>(api, WTI_CURSORS + cursor, CSR_PKTDATA)?;
        let [count] = info::<1>(api, WTI_INTERFACE, IFC_NDEVICES)?;
        let mut devices = vec![];
        for d in 0..count.min(MAX_DEVICES) {
            devices.push((info::<1>(api, WTI_DEVICES + d, DVC_FIRSTCSR)?[0], info::<1>(api, WTI_DEVICES + d, DVC_NCSRTYPES)?[0]));
        }
        let device = u32::try_from(device_of(cursor, &devices)?).ok()?;
        wheel_axis(csr_type, csr_pktdata, info::<4>(api, WTI_DEVICES + device, DVC_TPRESSURE))
    }

    fn is_window(hwnd: *mut c_void) -> bool {
        // SAFETY: `IsWindow` takes any handle value and reads no caller memory.
        unsafe { IsWindow(Some(HWND(hwnd))) }.as_bool()
    }

    impl State {
        fn close(&mut self, api: Api) {
            if let Some(c) = self.context.take() {
                // SAFETY: the handle came from `WTOpenW` and has not been closed.
                unsafe { (api.close)(c.handle) };
            }
            self.readings.forget();
        }

        /// Moves the context to the window that owns `hwnd` ([`next_context`]).
        fn open_for(&mut self, api: Api, hwnd: *mut c_void) {
            // SAFETY: `GetAncestor` takes any handle value and reads no caller memory.
            let owner = unsafe { GetAncestor(HWND(hwnd), GA_ROOTOWNER) }.0;
            let owner = if owner.is_null() { hwnd } else { owner };
            let current = self.context.as_ref().map(|c| (c.hwnd as usize, is_window(c.hwnd)));
            let (step, open) = next_context(current, self.tried.map(|w| w as usize), owner as usize);
            match step {
                Current::Keep => {}
                Current::Drop => {
                    self.context = None;
                    self.readings.forget();
                }
                Current::Close => self.close(api),
            }
            if !open {
                return;
            }
            self.tried = Some(owner);
            let Some(mut words) = info::<LOGCONTEXT_WORDS>(api, WTI_DEFCONTEXT, 0) else { return };
            configure(&mut words);
            // SAFETY: `words` is a `LOGCONTEXTW` of the size `WTInfoW` gave, and lives for the call.
            let handle = unsafe { (api.open)(owner, words.as_mut_ptr().cast(), 1) };
            if !handle.is_null() {
                self.context = Some(Context { handle, hwnd: owner });
            }
        }

        fn ours(&self, hwnd: *mut c_void, ctx: usize) -> Option<*mut c_void> {
            self.context.as_ref().filter(|c| c.hwnd == hwnd && c.handle as usize == ctx).map(|c| c.handle)
        }

        fn axis(&mut self, api: Api, cursor: u32) -> Option<Axis> {
            if let Some(&(_, axis)) = self.cursors.iter().find(|(c, _)| *c == cursor) {
                return axis;
            }
            let axis = cursor_axis(api, cursor);
            if self.cursors.len() >= MAX_READINGS * 4 {
                self.cursors.remove(0);
            }
            self.cursors.push((cursor, axis));
            axis
        }
    }

    /// winit's message hook for the Wintab messages: reads each packet of the context and starts
    /// the readings afresh on a cursor's change or proximity. It never consumes the message.
    pub fn msg_hook(msg: &MSG) {
        let what = message(msg.message, msg.wParam.0, msg.lParam.0);
        if what == Message::Other {
            return;
        }
        let Some(api) = api() else { return };
        STATE.with(|s| {
            let Ok(mut s) = s.try_borrow_mut() else { return };
            match what {
                Message::Packet { serial, ctx } => {
                    let Some(handle) = s.ours(msg.hwnd.0, ctx) else { return };
                    let mut buf = PacketBuffer([0; PACKET_BUFFER]);
                    // SAFETY: the handle is the open context's, and `buf` holds more bytes than a
                    // packet of every field.
                    if unsafe { (api.packet)(handle, serial, buf.0.as_mut_ptr().cast()) } == 0 {
                        return;
                    }
                    let Some(p) = decode(&buf.0, PACKET_DATA) else { return };
                    let axis = s.axis(api, p.cursor);
                    s.readings.packet(p, axis, msg.time);
                }
                Message::Forget { ctx } => {
                    if s.ours(msg.hwnd.0, ctx).is_some() {
                        s.readings.forget();
                    }
                }
                Message::Closed { ctx } => {
                    if s.context.as_ref().is_some_and(|c| c.handle as usize == ctx) {
                        s.context = None;
                        s.tried = None;
                        s.readings.forget();
                    }
                }
                Message::InfoChange => {
                    s.close(api);
                    s.tried = None;
                    s.cursors.clear();
                }
                Message::Other => {}
            }
        });
    }

    /// The wheel for a pointer message of a pen to `hwnd` at the message time `time`, with its
    /// `eraser` end ([`Readings::wheel_at`]), opening the context first when none is open.
    pub fn pen_wheel(hwnd: HWND, time: u32, eraser: bool) -> Option<f32> {
        let api = api()?;
        STATE.with(|s| {
            let mut s = s.try_borrow_mut().ok()?;
            s.open_for(api, hwnd.0);
            s.readings.wheel_at(time, eraser)
        })
    }

    /// Start the readings afresh: the pen left the window or detection range, or a press was
    /// canceled.
    pub fn forget() {
        STATE.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.readings.forget();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A packet of the fields of `mask`: each 32-bit field holds its value in `values`, or a filler
    /// when it has none, and the wider fields hold filler bytes.
    fn bytes(mask: u32, values: &[(u32, u32)]) -> Vec<u8> {
        let mut out = vec![];
        for &(f, n) in &FIELDS {
            if mask & f == 0 {
                continue;
            }
            let v = values.iter().find(|(k, _)| *k == f).map_or(0xAAAA_AAAA, |&(_, v)| v);
            if n == 4 {
                out.extend(v.to_ne_bytes());
            } else {
                out.extend(std::iter::repeat_n(0xEE, n));
            }
        }
        out
    }

    #[test]
    fn the_context_asks_for_the_wheel_without_the_system_cursor() {
        let mut words = [0u32; LOGCONTEXT_WORDS];
        words[LC_OPTIONS] = CXO_SYSTEM | CXO_PEN | 0x8000;
        words[LC_MSG_BASE] = 0x1234;
        words[LC_BTN_DN_MASK] = !0;
        words[LC_BTN_UP_MASK] = !0;
        words[LC_PKT_MODE] = PK_TANGENT_PRESSURE;
        words[31] = 77;
        configure(&mut words);
        assert_eq!(words[LC_OPTIONS], CXO_MESSAGES | CXO_CSRMESSAGES | 0x8000, "the driver's other options stay");
        assert_eq!(words[LC_MSG_BASE], WT_DEFBASE);
        assert_eq!(words[LC_PKT_DATA], PK_STATUS | PK_CURSOR | PK_X | PK_Y | PK_TANGENT_PRESSURE);
        assert_eq!(words[LC_PKT_MODE], 0, "absolute");
        assert_eq!(words[LC_MOVE_MASK] & PK_TANGENT_PRESSURE, PK_TANGENT_PRESSURE, "a turn in place makes a packet");
        assert_eq!((words[LC_BTN_DN_MASK], words[LC_BTN_UP_MASK]), (0, 0));
        assert_eq!(words[31], 77, "the extents stay the driver's");
    }

    #[test]
    fn packets_are_read_in_the_order_of_their_fields() {
        let want = Packet { status: 0, cursor: 3, tangent: 600 };
        let p = bytes(PACKET_DATA, &[(PK_STATUS, 0), (PK_CURSOR, 3), (PK_TANGENT_PRESSURE, 600)]);
        assert_eq!(p.len(), 20);
        assert_eq!(decode(&p, PACKET_DATA), Some(want));
        // Every field before the tangential pressure, the context handle and the orientation
        // after it.
        let all = FIELDS.iter().fold(0, |m, (f, _)| m | f);
        let p = bytes(all, &[(PK_STATUS, 0), (PK_CURSOR, 3), (PK_TANGENT_PRESSURE, 600)]);
        assert_eq!(p.len(), packet_size(all));
        assert_eq!(packet_size(all), std::mem::size_of::<usize>() + 4 * 11 + 24);
        assert_eq!(decode(&p, all), Some(want));
        assert_eq!(decode(&p[..p.len() - 1], all), None, "a short packet");
        assert_eq!(decode(&p, PK_STATUS | PK_CURSOR), None, "no tangential pressure");
        assert_eq!(field(&p, all, PK_ORIENTATION), None, "not a 32-bit field");
        assert!(packet_size(all) <= PACKET_BUFFER);
    }

    #[test]
    fn only_an_airbrush_with_a_range_reads_a_wheel() {
        let range = |min: i32, max: i32| Some([min as u32, max as u32, 0, 0]);
        let airbrush = 0x0902;
        assert_eq!(wheel_axis(airbrush, PACKET_DATA, range(0, 1023)), Some(Axis { min: 0, max: 1023 }));
        assert_eq!(wheel_axis(0x090A, PK_TANGENT_PRESSURE, range(-1023, 1023)), Some(Axis { min: -1023, max: 1023 }), "its other end");
        assert_eq!(wheel_axis(0x0802, PACKET_DATA, range(0, 1023)), None, "a general stylus");
        assert_eq!(wheel_axis(0x0804, PACKET_DATA, range(0, 1023)), None, "an Art Pen");
        assert_eq!(wheel_axis(airbrush, PK_X | PK_Y, range(0, 1023)), None, "no tangential pressure");
        assert_eq!(wheel_axis(airbrush, PACKET_DATA, None), None, "no range");
        assert_eq!(wheel_axis(airbrush, PACKET_DATA, range(0, 0)), None, "an empty range");
        assert_eq!(wheel_axis(airbrush, PACKET_DATA, range(1023, 0)), None, "a falling range");
    }

    #[test]
    fn the_wheel_is_its_place_in_the_range() {
        let axis = Axis { min: 0, max: 1000 };
        assert_eq!(axis.wheel(0), Some(0.0));
        assert_eq!(axis.wheel(250), Some(0.25));
        assert_eq!(axis.wheel(1000), Some(1.0));
        assert_eq!(axis.wheel(1001), None, "past the end");
        assert_eq!(axis.wheel(u32::MAX), None);
        let signed = Axis { min: -1000, max: 1000 };
        assert_eq!(signed.wheel(-1000i32 as u32), Some(0.0));
        assert_eq!(signed.wheel(0), Some(0.5));
        assert_eq!(signed.wheel(500), Some(0.75));
        assert_eq!(signed.wheel(i32::MIN as u32), None);
        let wide = Axis { min: i32::MIN, max: i32::MAX };
        assert_eq!(wide.wheel(i32::MAX as u32), Some(1.0), "no overflow");
        assert_eq!(Axis { min: 5, max: 5 }.wheel(5), None);
    }

    #[test]
    fn a_cursor_belongs_to_one_device() {
        let devices = [(0, 3), (3, 3)];
        assert_eq!(device_of(1, &devices), Some(0));
        assert_eq!(device_of(4, &devices), Some(1));
        assert_eq!(device_of(6, &devices), None);
        assert_eq!(device_of(1, &[(0, 3), (1, 1)]), None, "two devices claim it");
        assert_eq!(device_of(u32::MAX, &[(u32::MAX - 1, 5)]), None, "a range past the end");
        assert_eq!(device_of(0, &[]), None);
    }

    const AXIS: Option<Axis> = Some(Axis { min: 0, max: 1000 });

    fn packet(cursor: u32, tangent: u32) -> Packet {
        Packet { status: 0, cursor, tangent }
    }

    #[test]
    fn a_pen_message_takes_the_wheel_of_the_recent_airbrush() {
        let mut r = Readings::default();
        assert_eq!(r.wheel_at(1000, false), None, "no packet");
        r.packet(packet(1, 250), AXIS, 1000);
        assert_eq!(r.wheel_at(1010, false), Some(0.25));
        assert_eq!(r.wheel_at(1000 - MAX_SKEW_MS, false), Some(0.25), "a packet read just after the message");
        assert_eq!(r.wheel_at(60_000, false), Some(0.25), "a pen held still before it presses sends no packet");
        r.packet(packet(1, 1000), AXIS, u32::MAX - 5);
        r.packet(packet(2, 500), None, u32::MAX);
        assert_eq!(r.wheel_at(10, false), None, "two cursors at once, across the clock's wrap");
        r.packet(packet(1, 1000), AXIS, 5000);
        assert_eq!(r.wheel_at(5010, false), Some(1.0), "the recent one of two");
        assert_eq!(r.wheel_at(9000, false), None, "neither recent");
        r.forget();
        r.packet(packet(2, 500), None, 100);
        assert_eq!(r.wheel_at(100, false), None, "a cursor without a wheel");
        r.packet(packet(1, 2000), AXIS, 300);
        assert_eq!(r.wheel_at(500, false), None, "a value past the range");
    }

    #[test]
    fn zero_is_no_wheel_until_the_pen_reports_another_value() {
        let mut r = Readings::default();
        r.packet(packet(1, 0), AXIS, 100);
        assert_eq!(r.wheel_at(100, false), None);
        r.packet(packet(1, 300), AXIS, 110);
        r.packet(packet(1, 0), AXIS, 120);
        assert_eq!(r.wheel_at(120, false), Some(0.0), "a real 0 after another value");
        r.forget();
        r.packet(packet(1, 0), AXIS, 130);
        assert_eq!(r.wheel_at(130, false), None, "afresh after the pen came near");
    }

    #[test]
    fn the_readings_start_afresh_for_the_other_end_and_out_of_the_context() {
        let mut r = Readings::default();
        r.packet(packet(1, 300), AXIS, 100);
        assert_eq!(r.wheel_at(100, false), Some(0.3));
        assert_eq!(r.wheel_at(100, true), None, "the other end");
        r.packet(packet(1, 300), AXIS, 110);
        assert_eq!(r.wheel_at(110, true), Some(0.3));
        r.packet(Packet { status: TPS_PROXIMITY, ..packet(1, 300) }, AXIS, 120);
        assert_eq!(r.wheel_at(120, true), None, "out of the context");
        r.packet(packet(1, 300), AXIS, 130);
        r.packet(Packet { status: TPS_QUEUE_ERR, ..packet(1, 300) }, AXIS, 140);
        assert_eq!(r.wheel_at(140, true), None, "the queue overflowed");
        // The eraser end leaves; the tip's first message after it keeps the tip's packet.
        r.forget();
        r.packet(packet(1, 300), AXIS, 150);
        assert_eq!(r.wheel_at(150, false), Some(0.3), "after the readings started afresh");
        for c in 0..10 {
            r.packet(packet(c, 300), AXIS, 1000 * c);
        }
        assert_eq!(r.list.len(), MAX_READINGS, "bounded");
        assert_eq!(r.wheel_at(9000, false), Some(0.3));
    }

    #[test]
    fn the_context_follows_the_window_the_pen_draws_in() {
        let (main, doc) = (0x10, 0x20);
        assert_eq!(next_context(None, None, main), (Current::Keep, true), "the first pen message opens it");
        assert_eq!(next_context(Some((main, true)), Some(main), main), (Current::Keep, false), "the same window keeps it");
        assert_eq!(next_context(Some((main, true)), Some(main), doc), (Current::Close, true), "another document window");
        assert_eq!(next_context(Some((doc, true)), Some(doc), main), (Current::Close, true), "and back");
        assert_eq!(next_context(Some((main, false)), Some(main), doc), (Current::Drop, true), "a window that is gone");
        assert_eq!(next_context(Some((main, false)), Some(main), main), (Current::Drop, true), "its handle reused");
        assert_eq!(next_context(None, Some(main), main), (Current::Keep, false), "an open that failed is not tried again");
        assert_eq!(next_context(None, Some(main), doc), (Current::Keep, true), "but is for another window");
    }

    #[test]
    fn wintab_messages_name_their_context() {
        assert_eq!(message(WT_PACKET, 42, 0x5000), Message::Packet { serial: 42, ctx: 0x5000 });
        assert_eq!(message(WT_CSRCHANGE, 42, 0x5000), Message::Forget { ctx: 0x5000 });
        assert_eq!(message(WT_PROXIMITY, 0x5000, 1), Message::Forget { ctx: 0x5000 });
        assert_eq!(message(WT_CTXCLOSE, 0x5000, 0), Message::Closed { ctx: 0x5000 });
        assert_eq!(message(WT_INFOCHANGE, 0, 0), Message::InfoChange);
        // WM_POINTERUPDATE and the context-open message.
        assert_eq!(message(0x0245, 0, 0), Message::Other);
        assert_eq!(message(WT_DEFBASE + 1, 0, 0), Message::Other);
    }
}
