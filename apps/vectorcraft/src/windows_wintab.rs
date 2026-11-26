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
//! - Nothing here runs until the active tool draws with a brush or tip that varies with the
//!   finger wheel (Stylus Wheel), as the app says through [`set_wheel_used`]: until then pen
//!   messages load no library, install no hook and open no context, and when the tool stops
//!   using the wheel the context closes ([`unused_context`]).
//! - `Wintab32.dll` is loaded at run time from the system directory only, the first time a pen
//!   comes over a window while the wheel is used. Without it, without one of its exports, or when
//!   a call fails, the pens read no wheel and draw as before; the user sees no error.
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
//! - A pointer message of a pen takes the wheel of the airbrush whose latest packet came within
//!   [`MAX_SKEW_MS`] of the message, and none when two are that recent or none is
//!   ([`Readings::wheel_for`]). The first pen to read a wheel that way after the readings start
//!   afresh takes the readings: other pens read none until they start afresh, and that pen keeps
//!   reading the one airbrush read, however long ago its last packet came, since a pen held still
//!   sends none. As on
//!   macOS, 0 counts as no wheel until the pen has reported another value since it came near: the
//!   readings start afresh when a cursor
//!   comes near or leaves (`WT_PROXIMITY`), another cursor comes (`WT_CSRCHANGE`), the pen turns
//!   to its other end, a press of the pen is canceled, the pen leaves the window or detection
//!   range, or the context closes. These rules are VectorCraft's own.
//!
//! The context is dropped when the driver closes it (`WT_CTXCLOSE`), closed when the driver's
//! information changes (`WT_INFOCHANGE`) unless the driver closed it too, and dropped when its
//! window no longer exists; a later pen message opens it again. The driver sends those two notices
//! to the window procedure, where a sent message does not pass winit's message hook, so the
//! context's window gets a window procedure in front of its own that notes them ([`Notices`]).
//! When that procedure cannot be put in front, no context is opened for the window
//! ([`hooked_open`]). An open that leaves no context is tried again for the same window by a pen
//! message [`RETRY_MS`] or more later ([`next_context`]). A hook on the messages Windows sends to
//! the thread's windows passes a pen's `WM_POINTERLEAVE` to [`crate::windows_stylus`], and closes
//! the context when another app becomes the active one (`WM_ACTIVATEAPP`). No pen message opens a
//! context while this app is not active, so the context does not sit in front of another app's in
//! the driver's overlap order. A packet that `WTPacket` cannot read starts the readings afresh
//! ([`Readings::read`]), and so does a packet sent after the context's queue overflowed
//! (`TPS_QUEUE_ERR`, [`Readings::packet`]).
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
/// At most this many pens' ends are kept.
const MAX_PENS: usize = 16;

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

/// What the driver said about a cursor's wheel range.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lookup {
    /// The range, or `None` for a cursor the driver describes without a wheel.
    Known(Option<Axis>),
    /// A `WTInfoW` call failed at this message time.
    Failed { at: u32 },
}

/// Each cursor's wheel range, as the driver reported it, until the driver's information changes.
#[derive(Clone, Debug, Default)]
struct Cursors(Vec<(u32, Lookup)>);

impl Cursors {
    /// The wheel range of `cursor` for a packet at the message time `time`, asking the driver with
    /// `query` the first time, or again once [`retry_due`] after a query that failed (`Err`).
    /// Until then a cursor whose query failed reads no wheel.
    fn axis(&mut self, cursor: u32, time: u32, query: impl FnOnce() -> Result<Option<Axis>, ()>) -> Option<Axis> {
        if let Some(i) = self.0.iter().position(|(c, _)| *c == cursor) {
            match self.0[i].1 {
                Lookup::Known(axis) => return axis,
                Lookup::Failed { at } if !retry_due(at, time) => return None,
                Lookup::Failed { .. } => {
                    self.0.remove(i);
                }
            }
        }
        let lookup = match query() {
            Ok(axis) => Lookup::Known(axis),
            Err(()) => Lookup::Failed { at: time },
        };
        if self.0.len() >= MAX_READINGS * 4 {
            self.0.remove(0);
        }
        self.0.push((cursor, lookup));
        match lookup {
            Lookup::Known(axis) => axis,
            Lookup::Failed { .. } => None,
        }
    }

    fn clear(&mut self) {
        self.0.clear();
    }
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
    /// The pointer id of each pen whose pointer messages were read since the readings started
    /// afresh, with the end its last message read (`true` for the eraser end), at most
    /// [`MAX_PENS`] of them.
    ends: Vec<(u32, bool)>,
    /// The pointer id of the pen whose message first read the wheel since the readings started
    /// afresh; another pen's messages read none.
    pen: Option<u32>,
    /// The cursor whose wheel that pen read; the pen reads no other cursor's wheel until the
    /// readings start afresh.
    cursor: Option<u32>,
}

impl Readings {
    /// A packet of `p.cursor`, whose wheel range is `axis`, at the message time `time`. A packet of
    /// a cursor out of the context drops that cursor's reading. A packet sent after the context's
    /// queue overflowed starts every reading afresh, until the next packet.
    fn packet(&mut self, p: Packet, axis: Option<Axis>, time: u32) {
        if p.status & TPS_QUEUE_ERR != 0 {
            self.forget();
            return;
        }
        let at = self.list.iter().position(|r| r.cursor == p.cursor);
        if p.status & TPS_PROXIMITY != 0 {
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

    /// A `WT_PACKET` read at the message time `time`: the packet with its cursor's wheel range, or
    /// `None` when `WTPacket` failed or the packet could not be decoded. Then no reading is known to
    /// be current, and the readings start afresh until the next packet.
    fn read(&mut self, read: Option<(Packet, Option<Axis>)>, time: u32) {
        match read {
            Some((p, axis)) => self.packet(p, axis, time),
            None => self.forget(),
        }
    }

    /// Start afresh: the pen came near or left, another cursor came, a press was canceled, a packet
    /// could not be read, the context's queue overflowed, or the context closed.
    fn forget(&mut self) {
        self.list.clear();
        self.ends.clear();
        self.pen = None;
        self.cursor = None;
    }

    /// The pen whose leaving starts the readings afresh, for a message of `pen`: the pen that took
    /// the readings, or `pen` when none has.
    fn owner_or(&self, pen: u32) -> u32 {
        self.pen.unwrap_or(pen)
    }

    /// The wheel for a pointer message of the pen with the pointer id `pen` at the message time
    /// `time` with its `eraser` end, if the cursor it comes from is an airbrush: the one cursor whose
    /// latest packet is within [`MAX_SKEW_MS`] of the message, or, for the pen that took the
    /// readings, the one cursor read since they started afresh, however long ago its last packet
    /// came (a pen held still sends none). `None` when no cursor, or more than one, is that recent,
    /// or the cursor has read only 0 since the readings started afresh, or another pen's message
    /// read a wheel first since then, or the pen read another cursor's wheel since then. The same
    /// pen's turn to its other end starts them afresh first; another pen's message with another end
    /// does not.
    fn wheel_for(&mut self, time: u32, pen: u32, eraser: bool) -> Option<f32> {
        if self.pen.is_some_and(|p| p != pen) {
            return None;
        }
        if self.ends.iter().any(|&(p, e)| p == pen && e != eraser) {
            self.forget();
        }
        match self.ends.iter_mut().find(|(p, _)| *p == pen) {
            Some(end) => end.1 = eraser,
            None => {
                if self.ends.len() >= MAX_PENS {
                    self.ends.remove(0);
                }
                self.ends.push((pen, eraser));
            }
        }
        let r = match self.list.as_slice() {
            [one] if self.pen == Some(pen) => one,
            list => {
                let mut near_time = list.iter().filter(|r| near(time, r.time));
                let r = near_time.next()?;
                if near_time.next().is_some() {
                    return None;
                }
                r
            }
        };
        if !r.moved || self.cursor.is_some_and(|c| c != r.cursor) {
            return None;
        }
        let wheel = r.axis?.wheel(r.raw)?;
        self.pen = Some(pen);
        self.cursor = Some(r.cursor);
        Some(wheel)
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

/// After an open that left no context, the same window is tried again by the first pen message at
/// least this many milliseconds of the message clock later.
const RETRY_MS: u32 = 2000;

/// The context step for a pen message to a window owned by `owner` at the message time `time`:
/// what happens to the open context, whose window is `current` with whether that window still
/// exists, and whether a context is opened for `owner`. The context follows the window the pen
/// draws in. While no context is open, the window and time of the last open tried (`tried`) keep
/// pen messages to that window from trying again until [`RETRY_MS`] have passed; another window,
/// or the driver closing or changing the context, tries at once.
/// A pen message to a window of an app that is not the active one (`active` false) opens none, so
/// the context never sits in front of the active app's own in the driver's overlap order.
fn next_context(current: Option<(usize, bool)>, tried: Option<(usize, u32)>, owner: usize, time: u32, active: bool) -> (Current, bool) {
    let step = match current {
        Some((_, false)) => Current::Drop,
        Some((w, true)) if w == owner => return (Current::Keep, false),
        Some(_) => Current::Close,
        None => Current::Keep,
    };
    let due = match tried {
        Some((w, at)) if w == owner => retry_due(at, time),
        _ => true,
    };
    (step, active && (due || step != Current::Keep))
}

/// The context step for a pen message while the active tool reads no wheel ([`Wheel::Unused`]):
/// an open context closes, or is dropped when its window is gone (`current` as for
/// [`next_context`]), and none opens.
fn unused_context(current: Option<(usize, bool)>) -> Current {
    match current {
        Some((_, false)) => Current::Drop,
        Some(_) => Current::Close,
        None => Current::Keep,
    }
}

/// Whether the active tool draws with a brush or tip that varies with the finger wheel, as the
/// app last said ([`set_wheel_used`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Wheel {
    /// No pen message loads `Wintab32.dll`, installs the thread's hook or opens a context, and a
    /// context still open closes.
    #[default]
    Unused,
    /// Pen messages open the context as [`next_context`] says.
    Used,
}

/// Whether a pen message calls the driver at all: while the wheel is used, or to close a context
/// still open after it stopped being used.
fn calls_driver(wheel: Wheel, open: bool) -> bool {
    wheel == Wheel::Used || open
}

/// Whether a step that failed at the message time `at` is tried again at `time`: [`RETRY_MS`] or
/// more later, on the message clock, which wraps.
fn retry_due(at: u32, time: u32) -> bool {
    time.wrapping_sub(at) >= RETRY_MS
}

/// At most this many contexts' closing notices wait for [`Notices::take`].
const MAX_NOTICES: usize = 8;

/// The driver's notices about contexts (`WT_CTXCLOSE`, `WT_INFOCHANGE`), and another app becoming
/// the active one (`WM_ACTIVATEAPP`), since they were last taken. The driver sends them to the window procedure, at times while the reader is in a driver
/// call of its own, so they are noted here and acted on at the next Wintab or pen message.
#[derive(Clone, Debug, Default, PartialEq)]
struct Notices {
    /// The contexts the driver closed or is about to close.
    closed: Vec<usize>,
    /// The driver's information changed.
    info_changed: bool,
    /// The readings were to start afresh while they were in use ([`Notices::forget_later`]).
    forget: bool,
    /// Another app became the active one.
    deactivated: bool,
}

/// What the driver's notices do to the open context. Both can come in one batch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Notice {
    /// The driver closed it: drop the handle without `WTClose`, and open a context again at the
    /// next pen message.
    closed: bool,
    /// The driver's information changed: close it if it is still open, forget each cursor's wheel
    /// range, and open a context again at the next pen message.
    info_changed: bool,
    /// Start the readings afresh.
    forget: bool,
    /// Another app became the active one: close the context, and open one again at the next pen
    /// message once this app is active again.
    deactivated: bool,
}

impl Notices {
    /// Notes `message` if it is a closing, information-change or deactivation notice.
    fn note(&mut self, message: Message) {
        match message {
            Message::Closed { ctx } if !self.closed.contains(&ctx) => {
                if self.closed.len() >= MAX_NOTICES {
                    self.closed.remove(0);
                }
                self.closed.push(ctx);
            }
            Message::InfoChange => self.info_changed = true,
            Message::Deactivated => self.deactivated = true,
            _ => {}
        }
    }

    /// The reader closed `ctx` itself: its closing notice is not the driver's, and a context opened
    /// later with the same handle value stays open.
    fn closed_by_reader(&mut self, ctx: usize) {
        self.closed.retain(|&c| c != ctx);
    }

    /// The readings are to start afresh, at a time they are in use, for example when the pen leaves
    /// the window during a driver call: the next [`Notices::take`] says so.
    fn forget_later(&mut self) {
        self.forget = true;
    }

    /// Takes the notices: what they do to the open context `current`.
    fn take(&mut self, current: Option<usize>) -> Notice {
        let notices = std::mem::take(self);
        Notice {
            closed: current.is_some_and(|c| notices.closed.contains(&c)),
            info_changed: notices.info_changed,
            forget: notices.forget,
            deactivated: notices.deactivated,
        }
    }
}

/// Opens a context with `open` only when `hook` put the window procedure that notes the driver's
/// notices in front of the window's own and, asked after every other driver call, this app is
/// still the active one (`active`); otherwise opens none, and the window is tried again as
/// [`next_context`] says.
fn hooked_open<H>(hook: impl FnOnce() -> bool, active: impl FnOnce() -> bool, open: impl FnOnce() -> Option<H>) -> Option<H> {
    if hook() && active() { open() } else { None }
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
    /// Another app became the active one: the context closes.
    Deactivated,
    Other,
}

/// `WM_ACTIVATEAPP` (`winuser.h`): sent to the app's top-level windows when another app's window
/// is activated (`wParam` false) or one of the app's own is (true)
/// (<https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-activateapp>).
const WM_ACTIVATEAPP: u32 = 0x001C;

/// What the message `message` with `wparam` and `lparam` is to the Wintab reader. `WT_PACKET`
/// and `WT_CSRCHANGE` name the context in `lParam`, `WT_PROXIMITY` and `WT_CTXCLOSE` in `wParam`.
fn message(message: u32, wparam: usize, lparam: isize) -> Message {
    match message {
        WM_ACTIVATEAPP if wparam == 0 => Message::Deactivated,
        WT_PACKET => Message::Packet { serial: wparam as u32, ctx: lparam as usize },
        WT_CSRCHANGE => Message::Forget { ctx: lparam as usize },
        WT_PROXIMITY => Message::Forget { ctx: wparam },
        WT_CTXCLOSE => Message::Closed { ctx: wparam },
        WT_INFOCHANGE => Message::InfoChange,
        _ => Message::Other,
    }
}

#[cfg(windows)]
pub use win::{forget, msg_hook, pen_wheel, set_wheel_used, wheel_pen};

#[cfg(windows)]
#[allow(unsafe_code)]
mod win {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CWPSTRUCT, CallNextHookEx, CallWindowProcW, DefWindowProcW, GA_ROOTOWNER, GWLP_WNDPROC, GetAncestor, GetForegroundWindow, GetPropW,
        GetWindowLongPtrW, GetWindowThreadProcessId, IsWindow, MSG, RemovePropW, SetPropW, SetWindowLongPtrW, SetWindowsHookExW, WH_CALLWNDPROC,
        WM_NCDESTROY, WNDPROC,
    };
    use windows::core::{PCWSTR, s, w};

    use super::{
        Axis, CSR_PKTDATA, CSR_TYPE, Current, Cursors, DVC_FIRSTCSR, DVC_NCSRTYPES, DVC_TPRESSURE, IFC_NDEVICES, LOGCONTEXT_WORDS, MAX_DEVICES,
        MAX_INDEX, Message, Notice, Notices, PACKET_BUFFER, PACKET_DATA, PK_TANGENT_PRESSURE, Readings, WTI_CURSORS, WTI_DEFCONTEXT, WTI_DEVICES,
        WTI_INTERFACE, Wheel, calls_driver, configure, decode, device_of, hooked_open, is_airbrush, message, next_context, retry_due, unused_context,
        wheel_axis,
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
        /// The window an open was last tried for, and the message time it was tried at
        /// ([`next_context`]).
        tried: Option<(*mut c_void, u32)>,
        cursors: Cursors,
        readings: Readings,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
        /// Kept apart from [`STATE`], so the window procedure can note a notice the driver sends
        /// while [`STATE`] is borrowed for a driver call.
        static NOTICES: RefCell<Notices> = RefCell::new(Notices::default());
        /// Whether the active tool uses the wheel ([`set_wheel_used`]), kept apart from [`STATE`] so
        /// the app can say so while [`STATE`] is borrowed.
        static WHEEL: Cell<Wheel> = const { Cell::new(Wheel::Unused) };
    }

    /// The window property that holds the window procedure [`wnd_proc`] passes messages on to.
    const PREV_PROC: PCWSTR = w!("VectorCraft.Wintab.PrevProc");

    /// Notes `what` for the next [`State::take_notices`] when it is a notice about contexts.
    fn note(what: Message) {
        if !matches!(what, Message::Closed { .. } | Message::InfoChange | Message::Deactivated) {
            return;
        }
        NOTICES.with(|n| {
            if let Ok(mut n) = n.try_borrow_mut() {
                n.note(what);
            }
        });
    }

    /// The window procedure put in front of the context window's own: the driver sends
    /// `WT_CTXCLOSE` and `WT_INFOCHANGE` (<https://developer-docs.wacom.com/docs/icbt/windows/wintab/wintab-reference/>),
    /// and a sent message reaches the window procedure without passing winit's message hook. It
    /// notes them and passes every message on.
    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        note(message(msg, wparam.0, lparam.0));
        // SAFETY: `GetPropW` takes any handle value and a NUL-terminated name.
        let prev = unsafe { GetPropW(hwnd, PREV_PROC) }.0;
        if msg == WM_NCDESTROY {
            // SAFETY: as above; the window is being destroyed.
            let _ = unsafe { RemovePropW(hwnd, PREV_PROC) };
        }
        if prev.is_null() {
            // SAFETY: the default procedure takes any message of a window of this thread.
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // SAFETY: the property holds the window procedure `subclass` replaced, which stays valid
        // while the window exists.
        let prev = unsafe { std::mem::transmute::<*mut c_void, WNDPROC>(prev) };
        // SAFETY: `prev` is the window's own procedure, called with the message it was sent.
        unsafe { CallWindowProcW(prev, hwnd, msg, wparam, lparam) }
    }

    /// Another app became the active one: closes the context now, unless the reader is in a call of
    /// its own, when the noted [`Message::Deactivated`] closes it at the next Wintab or pen message.
    /// No pen messages come while another app is active, so waiting for one would leave the context
    /// in front of that app's own.
    fn deactivate() {
        let Some(api) = api() else { return };
        STATE.with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.take_notices(api);
            }
        });
    }

    /// Whether this app is the active one: the foreground window belongs to this thread, which
    /// runs every window of the app.
    fn is_active() -> bool {
        // SAFETY: `GetForegroundWindow` takes no arguments.
        let foreground = unsafe { GetForegroundWindow() };
        // SAFETY: `GetWindowThreadProcessId` takes any handle value, and no process id is asked for.
        !foreground.0.is_null() && unsafe { GetWindowThreadProcessId(foreground, None) } == unsafe { GetCurrentThreadId() }
    }

    /// The thread's hook on the messages Windows sends to its windows' procedures, which winit's
    /// message hook does not see: another app becoming the active one (`WM_ACTIVATEAPP`, sent to a
    /// top-level window of the app, <https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-activateapp>)
    /// closes the context ([`deactivate`]), and a pen's `WM_POINTERENTER` and `WM_POINTERLEAVE`
    /// (<https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerleave>) go to
    /// [`crate::windows_stylus::sent_message`]. The hook sees them whichever of the app's windows
    /// they are sent to (<https://learn.microsoft.com/en-us/windows/win32/winmsg/callwndproc>).
    unsafe extern "system" fn call_wnd_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: for `HC_ACTION` (0), `lParam` points to the message's `CWPSTRUCT`, which
            // lives for the call; `as_ref` gives `None` for a null pointer.
            if let Some(cwp) = unsafe { (lparam.0 as *const CWPSTRUCT).as_ref() } {
                if message(cwp.message, cwp.wParam.0, cwp.lParam.0) == Message::Deactivated {
                    note(Message::Deactivated);
                    deactivate();
                }
                crate::windows_stylus::sent_message(cwp.message, cwp.wParam.0);
            }
        }
        // SAFETY: every hook procedure passes the call on, with what it was given.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    /// Installs [`call_wnd_proc`] for this thread, for a pen message at the message time `time`,
    /// unless it is installed, and returns whether it is. It stays for the life of the thread. After
    /// a failed install, a pen message tries again once [`retry_due`].
    fn hook_thread(time: u32) -> bool {
        thread_local! {
            /// Installed, or the message time of the last failed install.
            static HOOK: std::cell::Cell<Result<(), Option<u32>>> = const { std::cell::Cell::new(Err(None)) };
        }
        match HOOK.get() {
            Ok(()) => return true,
            Err(Some(at)) if !retry_due(at, time) => return false,
            Err(_) => {}
        }
        // SAFETY: a hook of this thread, whose procedure is in this executable, so no module
        // handle is passed.
        let hooked = unsafe { SetWindowsHookExW(WH_CALLWNDPROC, Some(call_wnd_proc), None, GetCurrentThreadId()) }.is_ok();
        HOOK.set(if hooked { Ok(()) } else { Err(Some(time)) });
        hooked
    }

    /// Closes the context for a deactivation noted while the reader was in a call of its own
    /// ([`deactivate`]), once that call has ended.
    fn settle() {
        if NOTICES.with(|n| n.try_borrow().is_ok_and(|n| n.deactivated)) {
            deactivate();
        }
    }

    /// Puts [`wnd_proc`] in front of the window procedure of `hwnd`, once per window, and returns
    /// whether it is there. The window's property holds the procedure it replaced, and goes with the
    /// window.
    fn subclass(hwnd: *mut c_void) -> bool {
        let hwnd = HWND(hwnd);
        // SAFETY: `GetPropW` and `GetWindowLongPtrW` take any handle value and read no caller memory.
        if !unsafe { GetPropW(hwnd, PREV_PROC) }.0.is_null() {
            return true;
        }
        // SAFETY: as above.
        let prev = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) } as isize;
        if prev == 0 {
            return false;
        }
        // SAFETY: the window is this thread's (winit's), and the property's value is a procedure
        // address, not memory the system frees.
        if unsafe { SetPropW(hwnd, PREV_PROC, Some(HANDLE(prev as *mut c_void))) }.is_err() {
            return false;
        }
        type Proc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
        // SAFETY: `wnd_proc` is a window procedure, and the window is this thread's.
        if unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wnd_proc as Proc as usize as _) } as isize == 0 {
            // SAFETY: as for `SetPropW`.
            let _ = unsafe { RemovePropW(hwnd, PREV_PROC) };
            return false;
        }
        true
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

    /// The wheel range of `cursor`: `Ok(None)` for a cursor the driver describes without one, and
    /// `Err` when a `WTInfoW` call it needs fails.
    fn cursor_axis(api: Api, cursor: u32) -> Result<Option<Axis>, ()> {
        if cursor >= MAX_INDEX {
            return Ok(None);
        }
        let [csr_type] = info::<1>(api, WTI_CURSORS + cursor, CSR_TYPE).ok_or(())?;
        let [csr_pktdata] = info::<1>(api, WTI_CURSORS + cursor, CSR_PKTDATA).ok_or(())?;
        if !is_airbrush(csr_type) || csr_pktdata & PK_TANGENT_PRESSURE == 0 {
            return Ok(None);
        }
        let [count] = info::<1>(api, WTI_INTERFACE, IFC_NDEVICES).ok_or(())?;
        let mut devices = vec![];
        for d in 0..count.min(MAX_DEVICES) {
            let [first] = info::<1>(api, WTI_DEVICES + d, DVC_FIRSTCSR).ok_or(())?;
            let [n] = info::<1>(api, WTI_DEVICES + d, DVC_NCSRTYPES).ok_or(())?;
            devices.push((first, n));
        }
        let Some(device) = device_of(cursor, &devices).and_then(|d| u32::try_from(d).ok()) else { return Ok(None) };
        let range = info::<4>(api, WTI_DEVICES + device, DVC_TPRESSURE).ok_or(())?;
        Ok(wheel_axis(csr_type, csr_pktdata, Some(range)))
    }

    /// Whether a context is open, or may be: [`STATE`] is borrowed for a driver call.
    fn context_open() -> bool {
        STATE.with(|s| !s.try_borrow().is_ok_and(|s| s.context.is_none()))
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
                NOTICES.with(|n| {
                    if let Ok(mut n) = n.try_borrow_mut() {
                        n.closed_by_reader(c.handle as usize);
                    }
                });
            }
            self.readings.forget();
        }

        /// Acts on the driver's notices since the last look ([`Notices::take`]).
        fn take_notices(&mut self, api: Api) {
            let current = self.context.as_ref().map(|c| c.handle as usize);
            let notice = NOTICES.with(|n| n.try_borrow_mut().map_or(Notice::default(), |mut n| n.take(current)));
            if notice.closed {
                self.context = None;
                self.tried = None;
                self.readings.forget();
            }
            if notice.info_changed {
                self.close(api);
                self.tried = None;
                self.cursors.clear();
            }
            if notice.forget {
                self.readings.forget();
            }
            if notice.deactivated {
                self.close(api);
                self.tried = None;
            }
        }

        /// Moves the context to the window that owns `hwnd`, for a pen message at the message time
        /// `time` ([`next_context`]), or closes it when the active tool does not use the wheel
        /// (`wheel`, [`unused_context`]).
        fn open_for(&mut self, api: Api, hwnd: *mut c_void, time: u32, wheel: Wheel) {
            // SAFETY: `GetAncestor` takes any handle value and reads no caller memory.
            let owner = unsafe { GetAncestor(HWND(hwnd), GA_ROOTOWNER) }.0;
            let owner = if owner.is_null() { hwnd } else { owner };
            let current = self.context.as_ref().map(|c| (c.hwnd as usize, is_window(c.hwnd)));
            let tried = self.tried.map(|(w, at)| (w as usize, at));
            let (step, open) = match wheel {
                Wheel::Used => next_context(current, tried, owner as usize, time, is_active()),
                Wheel::Unused => (unused_context(current), false),
            };
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
            self.tried = Some((owner, time));
            let Some(mut words) = info::<LOGCONTEXT_WORDS>(api, WTI_DEFCONTEXT, 0) else { return };
            configure(&mut words);
            // Whether the app is active is asked again here, after the close and `WTInfoW` above.
            let handle = hooked_open(
                || subclass(owner),
                is_active,
                || {
                    // SAFETY: `words` is a `LOGCONTEXTW` of the size `WTInfoW` gave, and lives for
                    // the call.
                    let handle = unsafe { (api.open)(owner, words.as_mut_ptr().cast(), 1) };
                    (!handle.is_null()).then_some(handle)
                },
            );
            if let Some(handle) = handle {
                self.context = Some(Context { handle, hwnd: owner });
            }
        }

        fn ours(&self, hwnd: *mut c_void, ctx: usize) -> Option<*mut c_void> {
            self.context.as_ref().filter(|c| c.hwnd == hwnd && c.handle as usize == ctx).map(|c| c.handle)
        }

        fn axis(&mut self, api: Api, cursor: u32, time: u32) -> Option<Axis> {
            self.cursors.axis(cursor, time, || cursor_axis(api, cursor))
        }
    }

    /// winit's message hook for the Wintab messages: reads each packet of the context and starts
    /// the readings afresh on a cursor's change or proximity. A closing or information-change
    /// notice that comes through the queue is noted as [`wnd_proc`] notes a sent one. It never
    /// consumes the message.
    pub fn msg_hook(msg: &MSG) {
        let what = message(msg.message, msg.wParam.0, msg.lParam.0);
        if what == Message::Other {
            return;
        }
        if !calls_driver(WHEEL.get(), context_open()) {
            return;
        }
        let Some(api) = api() else { return };
        note(what);
        STATE.with(|s| {
            let Ok(mut s) = s.try_borrow_mut() else { return };
            s.take_notices(api);
            match what {
                Message::Packet { serial, ctx } => {
                    let Some(handle) = s.ours(msg.hwnd.0, ctx) else { return };
                    let mut buf = PacketBuffer([0; PACKET_BUFFER]);
                    // SAFETY: the handle is the open context's, and `buf` holds more bytes than a
                    // packet of every field.
                    let read = unsafe { (api.packet)(handle, serial, buf.0.as_mut_ptr().cast()) } != 0;
                    let read = read.then(|| decode(&buf.0, PACKET_DATA)).flatten().map(|p| (p, s.axis(api, p.cursor, msg.time)));
                    s.readings.read(read, msg.time);
                }
                Message::Forget { ctx } => {
                    if s.ours(msg.hwnd.0, ctx).is_some() {
                        s.readings.forget();
                    }
                }
                Message::Closed { .. } | Message::InfoChange | Message::Deactivated | Message::Other => {}
            }
        });
        settle();
    }

    /// The wheel for a pointer message of the pen with the pointer id `pen` to `hwnd` at the message
    /// time `time`, with its `eraser` end ([`Readings::wheel_for`]), opening the context first when
    /// none is open. While the active tool does not use the wheel ([`set_wheel_used`]), it reads
    /// none and calls the driver only to close a context still open ([`calls_driver`]). The first
    /// call that opens a context installs [`call_wnd_proc`] ([`hook_thread`]); until it is
    /// installed, no context opens and the pens read no wheel.
    pub fn pen_wheel(hwnd: HWND, time: u32, pen: u32, eraser: bool) -> Option<f32> {
        let used = WHEEL.get();
        if !calls_driver(used, context_open()) {
            return None;
        }
        let api = api()?;
        if used == Wheel::Used && !hook_thread(time) {
            return None;
        }
        let wheel = STATE.with(|s| {
            let mut s = s.try_borrow_mut().ok()?;
            s.take_notices(api);
            s.open_for(api, hwnd.0, time, used);
            s.readings.wheel_for(time, pen, eraser)
        });
        settle();
        wheel.filter(|_| used == Wheel::Used)
    }

    /// The app says whether the active tool draws with a brush or tip that varies with the finger
    /// wheel (`used`). Until it says so, pen messages read no wheel and leave the driver alone; when
    /// it stops, the context closes now, or at the next pen message while the reader is in a call
    /// of its own.
    pub fn set_wheel_used(used: bool) {
        let wheel = if used { Wheel::Used } else { Wheel::Unused };
        if WHEEL.replace(wheel) == wheel {
            return;
        }
        STATE.with(|s| {
            let Ok(mut s) = s.try_borrow_mut() else { return };
            s.tried = None;
            if wheel == Wheel::Unused
                && s.context.is_some()
                && let Some(api) = api()
            {
                // A context the driver closed is dropped without `WTClose`.
                s.take_notices(api);
                s.close(api);
            }
        });
        settle();
    }

    /// The pen whose leaving starts the readings afresh, for a message of the pen with the pointer
    /// id `pen` ([`Readings::owner_or`]).
    pub fn wheel_pen(pen: u32) -> u32 {
        STATE.with(|s| s.try_borrow().map_or(pen, |s| s.readings.owner_or(pen)))
    }

    /// Start the readings afresh: the pen left the window or detection range, or a press was
    /// canceled. While the readings are in use, as in a driver call, they start afresh at the next
    /// Wintab or pen message ([`Notices::forget_later`]).
    pub fn forget() {
        STATE.with(|s| match s.try_borrow_mut() {
            Ok(mut s) => s.readings.forget(),
            Err(_) => NOTICES.with(|n| {
                if let Ok(mut n) = n.try_borrow_mut() {
                    n.forget_later();
                }
            }),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pointer id of the pen in the tests that use one pen.
    const PEN: u32 = 1;

    impl Readings {
        fn wheel_at(&mut self, time: u32, eraser: bool) -> Option<f32> {
            self.wheel_for(time, PEN, eraser)
        }
    }

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
    fn a_pen_keeps_the_airbrush_whose_wheel_it_read() {
        let mut r = Readings::default();
        r.packet(packet(1, 250), AXIS, 1000);
        assert_eq!(r.wheel_at(1000, false), Some(0.25));
        // Another tablet's airbrush moves while the pen is held still.
        r.packet(packet(2, 750), AXIS, 5000);
        assert_eq!(r.wheel_at(5010, false), None, "the other cursor's wheel");
        r.packet(packet(1, 500), AXIS, 6000);
        assert_eq!(r.wheel_at(6010, false), Some(0.5), "its own cursor again");
        r.forget();
        r.packet(packet(2, 750), AXIS, 7000);
        assert_eq!(r.wheel_at(7010, false), Some(0.75), "after the readings started afresh");
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
        r.forget();
        for c in 0..10 {
            r.packet(packet(c, 300), AXIS, 1000 * c);
        }
        assert_eq!(r.list.len(), MAX_READINGS, "bounded");
        assert_eq!(r.wheel_at(9000, false), Some(0.3));
    }

    /// [`next_context`] while this app is the active one.
    fn step(current: Option<(usize, bool)>, tried: Option<(usize, u32)>, owner: usize, time: u32) -> (Current, bool) {
        next_context(current, tried, owner, time, true)
    }

    #[test]
    fn the_context_follows_the_window_the_pen_draws_in() {
        let (main, doc) = (0x10, 0x20);
        let t = 1000;
        assert_eq!(step(None, None, main, t), (Current::Keep, true), "the first pen message opens it");
        assert_eq!(step(Some((main, true)), Some((main, t)), main, t), (Current::Keep, false), "the same window keeps it");
        assert_eq!(step(Some((main, true)), Some((main, t)), main, t + 60_000), (Current::Keep, false), "however long it is open");
        assert_eq!(step(Some((main, true)), Some((main, t)), doc, t), (Current::Close, true), "another document window");
        assert_eq!(step(Some((doc, true)), Some((doc, t)), main, t), (Current::Close, true), "and back");
        assert_eq!(step(Some((main, false)), Some((main, t)), doc, t), (Current::Drop, true), "a window that is gone");
        assert_eq!(step(Some((main, false)), Some((main, t)), main, t), (Current::Drop, true), "its handle reused");
        assert_eq!(step(None, Some((main, t)), main, t + 1), (Current::Keep, false), "an open that failed is not tried at once");
        assert_eq!(step(None, Some((main, t)), doc, t + 1), (Current::Keep, true), "but is for another window");
    }

    #[test]
    fn a_window_whose_open_failed_is_tried_again_later() {
        let main = 0x10;
        let t = 1000;
        for dt in [0, 1, 500, RETRY_MS - 1] {
            assert_eq!(step(None, Some((main, t)), main, t + dt), (Current::Keep, false), "{dt} ms later");
        }
        assert_eq!(step(None, Some((main, t)), main, t + RETRY_MS), (Current::Keep, true), "after the retry interval");
        assert_eq!(step(None, Some((main, t)), main, t + 60_000), (Current::Keep, true), "or long after");
        // The message clock wraps after about 49.7 days.
        let late = u32::MAX - 100;
        assert_eq!(step(None, Some((main, late)), main, late.wrapping_add(50)), (Current::Keep, false), "across the wrap");
        assert_eq!(step(None, Some((main, late)), main, late.wrapping_add(RETRY_MS)), (Current::Keep, true), "and due across it");
    }

    #[test]
    fn a_failed_step_is_retried_after_the_interval() {
        assert!(!retry_due(1000, 1000), "at once");
        assert!(!retry_due(1000, 1000 + RETRY_MS - 1));
        assert!(retry_due(1000, 1000 + RETRY_MS), "after the interval");
        assert!(retry_due(u32::MAX - 10, (u32::MAX - 10).wrapping_add(RETRY_MS)), "across the clock's wrap");
        assert!(!retry_due(u32::MAX - 10, 5), "and not early across it");
    }

    #[test]
    fn no_context_opens_while_another_app_is_active() {
        let (main, doc) = (0x10, 0x20);
        let t = 1000;
        assert_eq!(next_context(None, None, main, t, false), (Current::Keep, false), "a pen hovering over an inactive window");
        assert_eq!(next_context(None, Some((main, t)), main, t + RETRY_MS, false), (Current::Keep, false), "not even when a retry is due");
        assert_eq!(next_context(Some((main, true)), None, doc, t, false), (Current::Close, false), "another window's context closes");
        assert_eq!(next_context(Some((main, false)), None, main, t, false), (Current::Drop, false), "a gone window's is dropped");
        assert_eq!(next_context(None, None, main, t, true), (Current::Keep, true), "and the app's return opens it");
    }

    #[test]
    fn the_driver_is_left_alone_until_the_wheel_is_used() {
        assert!(!calls_driver(Wheel::Unused, false), "no brush or tip on Stylus Wheel");
        assert!(calls_driver(Wheel::Used, false), "one is: the context opens");
        assert!(calls_driver(Wheel::Unused, true), "none any more: the open context closes");
        assert_eq!(Wheel::default(), Wheel::Unused, "at start-up");
        let main = 0x10;
        assert_eq!(unused_context(None), Current::Keep, "nothing open");
        assert_eq!(unused_context(Some((main, true))), Current::Close, "the pen window's context closes");
        assert_eq!(unused_context(Some((main, false))), Current::Drop, "a gone window's is dropped");
    }

    #[test]
    fn another_app_becoming_active_closes_the_context() {
        assert_eq!(message(WM_ACTIVATEAPP, 0, 0), Message::Deactivated);
        assert_eq!(message(WM_ACTIVATEAPP, 1, 0), Message::Other, "this app becoming active");
        let ours = 0x5000;
        let mut n = Notices::default();
        n.note(Message::Deactivated);
        assert_eq!(n.take(Some(ours)), Notice { deactivated: true, ..Notice::default() });
        assert_eq!(n.take(Some(ours)), Notice::default(), "once");
        n.note(Message::Deactivated);
        n.note(Message::Closed { ctx: ours });
        assert_eq!(n.take(Some(ours)), Notice { closed: true, deactivated: true, ..Notice::default() }, "beside the driver's close");
    }

    #[test]
    fn a_packet_that_cannot_be_read_starts_the_readings_afresh() {
        let mut r = Readings::default();
        r.read(Some((packet(1, 750), AXIS)), 100);
        assert_eq!(r.wheel_at(100, false), Some(0.75));
        r.read(None, 110);
        assert_eq!(r.wheel_at(110, false), None, "the last reading is not kept after a failed read");
        r.read(Some((packet(1, 500), AXIS)), 120);
        assert_eq!(r.wheel_at(120, false), Some(0.5), "the next packet reads again");
    }

    #[test]
    fn the_drivers_notices_drop_or_reopen_the_context() {
        let (ours, other) = (0x5000, 0x6000);
        let keep = Notice::default();
        let closed = Notice { closed: true, ..Notice::default() };
        let reopen = Notice { info_changed: true, ..Notice::default() };
        let mut n = Notices::default();
        assert_eq!(n.take(Some(ours)), keep, "no notice");
        n.note(Message::Closed { ctx: other });
        assert_eq!(n.take(Some(ours)), keep, "another context closed");
        n.note(Message::Closed { ctx: ours });
        assert_eq!(n.take(Some(ours)), closed, "the driver closed it");
        assert_eq!(n.take(Some(ours)), keep, "taken once");
        n.note(Message::Closed { ctx: ours });
        assert_eq!(n.take(None), keep, "nothing open");
        assert_eq!(n, Notices::default(), "and the notice is gone");
        n.note(Message::InfoChange);
        assert_eq!(n.take(Some(ours)), reopen);
        n.note(Message::InfoChange);
        assert_eq!(n.take(None), reopen, "cursor ranges are read again even with nothing open");
        // The reader closes its context, and the driver's notice of that arrives during the call;
        // a context opened next under the same handle value stays open.
        n.note(Message::Closed { ctx: ours });
        n.closed_by_reader(ours);
        assert_eq!(n.take(Some(ours)), keep);
        for ctx in 0..20 {
            n.note(Message::Closed { ctx });
            n.note(Message::Closed { ctx });
        }
        assert_eq!(n.closed.len(), MAX_NOTICES, "bounded, each context once");
        assert_eq!(n.take(Some(19)), closed, "the latest kept");
        n.note(Message::Packet { serial: 1, ctx: ours });
        n.note(Message::Forget { ctx: ours });
        n.note(Message::Other);
        assert_eq!(n, Notices::default(), "other messages are not notices");
    }

    #[test]
    fn a_close_and_an_information_change_in_one_batch_both_count() {
        let ours = 0x5000;
        let both = Notice { closed: true, info_changed: true, ..Notice::default() };
        let mut n = Notices::default();
        n.note(Message::Closed { ctx: ours });
        n.note(Message::InfoChange);
        assert_eq!(n.take(Some(ours)), both, "the closed handle is dropped, not closed again");
        n.note(Message::InfoChange);
        n.note(Message::Closed { ctx: ours });
        assert_eq!(n.take(Some(ours)), both, "in either order");
        n.note(Message::Closed { ctx: 0x6000 });
        n.note(Message::InfoChange);
        assert_eq!(n.take(Some(ours)), Notice { info_changed: true, ..Notice::default() }, "another context's close");
    }

    #[test]
    fn a_leave_while_the_readings_are_in_use_starts_them_afresh_next() {
        let ours = 0x5000;
        let mut n = Notices::default();
        n.forget_later();
        assert_eq!(n.take(Some(ours)), Notice { forget: true, ..Notice::default() }, "the next look starts the readings afresh");
        assert_eq!(n.take(Some(ours)), Notice::default(), "once");
        n.forget_later();
        assert_eq!(n.take(None), Notice { forget: true, ..Notice::default() }, "with nothing open too");
        n.forget_later();
        n.note(Message::Closed { ctx: ours });
        assert_eq!(n.take(Some(ours)), Notice { closed: true, forget: true, ..Notice::default() }, "beside a close");
    }

    #[test]
    fn a_context_opens_only_behind_the_window_procedure() {
        let opens = std::cell::Cell::new(0);
        let open = || {
            opens.set(opens.get() + 1);
            Some(7)
        };
        assert_eq!(hooked_open(|| true, || true, open), Some(7));
        assert_eq!(opens.get(), 1);
        let open = || {
            opens.set(opens.get() + 1);
            Some(7)
        };
        assert_eq!(hooked_open(|| false, || true, open), None, "no context without the procedure");
        assert_eq!(opens.get(), 1, "and the driver is not asked for one");
        assert_eq!(hooked_open(|| true, || true, || None::<u8>), None, "an open that fails");
    }

    #[test]
    fn no_context_opens_when_another_app_became_active_before_the_open() {
        let opens = std::cell::Cell::new(0);
        let open = || {
            opens.set(opens.get() + 1);
            Some(7)
        };
        assert_eq!(hooked_open(|| true, || false, open), None);
        assert_eq!(opens.get(), 0, "the driver is not asked for one");
    }

    #[test]
    fn another_pens_end_does_not_start_the_readings_afresh() {
        let other = 2;
        let mut r = Readings::default();
        assert_eq!(r.wheel_for(100, other, true), None, "an eraser end before any packet");
        r.packet(packet(1, 300), AXIS, 110);
        assert_eq!(r.wheel_at(120, false), Some(0.3), "another pen's tip still reads the packet");
        assert_eq!(r.wheel_at(130, true), None, "the same pen turned over starts them afresh");
        // Pen 1's tip, then pen 2's, then pen 1's eraser, before either reads a wheel.
        r.forget();
        assert_eq!(r.wheel_at(200, false), None, "no packet yet");
        assert_eq!(r.wheel_for(210, other, false), None);
        r.packet(packet(1, 300), AXIS, 220);
        assert_eq!(r.wheel_at(230, true), None, "pen 1's turn is seen past pen 2's message");
        for pen in 10..40 {
            r.wheel_for(5000, pen, false);
        }
        assert_eq!(r.ends.len(), MAX_PENS, "bounded");
    }

    #[test]
    fn a_failed_cursor_lookup_is_asked_again_later() {
        let asks = std::cell::Cell::new(0);
        let failing = || {
            asks.set(asks.get() + 1);
            Err(())
        };
        let mut c = Cursors::default();
        assert_eq!(c.axis(1, 1000, failing), None);
        assert_eq!(c.axis(1, 1000 + RETRY_MS - 1, || Ok(AXIS)), None, "not asked again before the interval");
        assert_eq!(asks.get(), 1);
        assert_eq!(c.axis(1, 1000 + RETRY_MS, || Ok(AXIS)), AXIS, "asked again after it");
        assert_eq!(c.axis(1, 9000, || Ok(None)), AXIS, "a range the driver gave is kept");
        assert_eq!(c.axis(2, 9000, || Ok(None)), None, "a cursor without a wheel");
        assert_eq!(c.axis(2, 9000 + RETRY_MS, || Ok(AXIS)), None, "is not asked again");
        c.clear();
        assert_eq!(c.axis(2, 9000, || Ok(AXIS)), AXIS, "until the driver's information changes");
        for cursor in 10..40 {
            c.axis(cursor, 9000, || Ok(None));
        }
        assert_eq!(c.0.len(), MAX_READINGS * 4, "bounded");
    }

    #[test]
    fn the_readings_belong_to_the_pen_that_took_them_first() {
        let other = 2;
        let mut r = Readings::default();
        r.packet(packet(1, 300), AXIS, 100);
        assert_eq!(r.wheel_for(5000, other, false), None, "a pen message long after the packet does not take them");
        assert_eq!(r.wheel_for(150, other, false), Some(0.3), "one near the packet does");
        assert_eq!(r.wheel_at(5010, false), None, "another pen with no packets of its own");
        assert_eq!(r.wheel_for(5020, other, false), Some(0.3), "and the pen that took them still reads them");
        r.forget();
        assert_eq!(r.wheel_at(5030, false), None, "no packet since the readings started afresh");
        r.packet(packet(1, 400), AXIS, 5040);
        assert_eq!(r.wheel_at(5040, false), Some(0.4), "then another pen takes them");
    }

    #[test]
    fn a_pen_takes_the_readings_only_when_it_reads_a_wheel() {
        let other = 2;
        let mut r = Readings::default();
        assert_eq!(r.wheel_for(100, other, false), None, "no packet yet");
        assert_eq!(r.owner_or(PEN), PEN, "and no pen took the readings");
        r.packet(packet(1, 300), AXIS, 110);
        assert_eq!(r.wheel_at(110, false), Some(0.3), "the pen with the reading");
        assert_eq!(r.owner_or(other), PEN, "the pen whose leaving starts the readings afresh");
        r.forget();
        assert_eq!(r.owner_or(other), other);
    }

    #[test]
    fn a_queue_overflow_starts_every_reading_afresh() {
        let mut r = Readings::default();
        r.packet(packet(1, 300), AXIS, 100);
        r.packet(packet(2, 500), None, 110);
        r.packet(Packet { status: TPS_QUEUE_ERR, ..packet(2, 500) }, None, 120);
        assert!(r.list.is_empty(), "the queue is the context's, not the cursor's");
        assert_eq!(r.wheel_at(5000, false), None, "the airbrush's old wheel is not taken");
        r.packet(packet(1, 400), AXIS, 5000);
        assert_eq!(r.wheel_at(5000, false), Some(0.4), "the next packet reads again");
        r.packet(packet(2, 500), None, 5010);
        r.packet(Packet { status: TPS_PROXIMITY, ..packet(2, 500) }, None, 5020);
        assert_eq!(r.list.len(), 1, "a cursor that leaves drops only its own reading");
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
