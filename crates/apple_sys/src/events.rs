use crate::Error;
use std::{
    ffi::{c_char, c_void},
    mem::{align_of, offset_of, size_of},
    ptr::{self, NonNull},
};

/// A fixed-size record shared by the native producer and Rust consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Event {
    /// Nanoseconds on the producer's event clock.
    pub time_ns: u64,
    /// Service-specific event kind.
    pub kind: u32,
    /// Service-specific flags.
    pub flags: u32,
    /// Inline service data. Producers copy bytes into the record.
    pub payload: [u8; 64],
}

impl Default for Event {
    fn default() -> Self {
        Self {
            time_ns: 0,
            kind: 0,
            flags: 0,
            payload: [0; 64],
        }
    }
}

const _: () = {
    assert!(size_of::<Event>() == 80);
    assert!(align_of::<Event>() == 8);
    assert!(offset_of!(Event, kind) == 8);
    assert!(offset_of!(Event, payload) == 16);
};

/// Input activity from a capture source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// Pointer moved with no button held.
    Move,
    /// Pointer button pressed.
    Down,
    /// Pointer button released.
    Up,
    /// Pointer moved with a button held.
    Drag,
    /// Scroll wheel or trackpad scroll.
    Scroll,
    /// A keyboard press.
    Key,
    /// Modifier state changed.
    Flags,
}

/// One normalized, take-relative input event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputEvent {
    /// Nanoseconds since the first screen frame with paused time removed.
    pub time_ns: u64,
    /// Input category.
    pub kind: InputKind,
    /// Coordinates within the captured source, from its upper-left corner.
    pub point: [f64; 2],
    /// Horizontal and vertical scroll deltas in points.
    pub scroll: [f64; 2],
    /// Native modifier flags for keyboard events.
    pub modifiers: u64,
    /// Mouse button or virtual key code, as appropriate for the kind.
    pub code: u32,
    text: [u16; 8],
    text_len: u32,
}

#[repr(C)]
struct InputPayload {
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    modifiers: u64,
    code: u32,
    text_len: u32,
    text: [u16; 8],
}

const _: () = assert!(size_of::<InputPayload>() == 64);

impl InputEvent {
    /// Decodes one record from `Recording::read_input` without allocating.
    pub fn decode(record: &Event) -> Option<Self> {
        let kind = match record.kind {
            1 => InputKind::Move,
            2 => InputKind::Down,
            3 => InputKind::Up,
            4 => InputKind::Drag,
            5 => InputKind::Scroll,
            6 => InputKind::Key,
            7 => InputKind::Flags,
            _ => return None,
        };
        // SAFETY: the payload ABI is 64 initialized bytes consisting only of
        // native-endian integers and floats. The pointer need not be aligned.
        let payload = unsafe {
            record
                .payload
                .as_ptr()
                .cast::<InputPayload>()
                .read_unaligned()
        };
        if payload.text_len > 8 {
            return None;
        }
        Some(Self {
            time_ns: record.time_ns,
            kind,
            point: [payload.x, payload.y],
            scroll: [payload.dx, payload.dy],
            modifiers: payload.modifiers,
            code: payload.code,
            text: payload.text,
            text_len: payload.text_len,
        })
    }

    /// Returns the key's UTF-16 text when it produced a printable character.
    pub fn text(&self) -> &[u16] {
        &self.text[..self.text_len as usize]
    }
}

/// Called on the producer's thread when an empty queue becomes ready. The
/// callback receives only a numeric token and must return promptly.
pub type Wake = extern "C" fn(u64);

/// Owns the native event ring and its single-consumer acknowledgement position.
pub struct EventRing {
    handle: NonNull<c_void>,
    peeked: usize,
}

unsafe extern "C" {
    fn splice_events_create(
        capacity: usize,
        wake: Option<Wake>,
        token: u64,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_events_release(ring: *mut c_void);
    fn splice_events_push(ring: *mut c_void, event: *const Event) -> bool;
    fn splice_events_peek(ring: *mut c_void, out: *mut Event, capacity: usize) -> usize;
    fn splice_events_ack(ring: *mut c_void, count: usize) -> bool;
    fn splice_events_dropped(ring: *mut c_void) -> u64;
    fn splice_ax_caret(out: *mut f64) -> bool;
}

impl EventRing {
    /// Allocates storage for a fixed number of events and registers an optional wake.
    pub fn new(capacity: usize, wake: Option<Wake>, token: u64) -> Result<Self, Error> {
        let mut diagnostic = ptr::null_mut();
        // SAFETY: the diagnostic is a writable out pointer and the callback has no borrowed data.
        let handle = unsafe { splice_events_create(capacity, wake, token, &mut diagnostic) };
        let handle = NonNull::new(handle).ok_or_else(|| {
            // SAFETY: the bridge transfers its optional diagnostic when creation fails.
            unsafe { Error::from_native(diagnostic, "Could not create the event ring.") }
        })?;
        Ok(Self { handle, peeked: 0 })
    }

    /// Copies a record into the queue. A full queue drops the new event and counts the loss.
    pub fn try_push(&self, event: Event) -> bool {
        // SAFETY: self owns the live ring, and the native call copies the event before returning.
        unsafe { splice_events_push(self.handle.as_ptr(), &event) }
    }

    /// Copies queued records into a caller-owned buffer without consuming them.
    pub fn peek(&mut self, output: &mut [Event]) -> usize {
        // SAFETY: the native queue writes at most output.len() live records to this buffer.
        self.peeked =
            unsafe { splice_events_peek(self.handle.as_ptr(), output.as_mut_ptr(), output.len()) };
        self.peeked
    }

    /// Consumes at most the outstanding events copied by the latest peek.
    pub fn ack(&mut self, count: usize) -> bool {
        if count > self.peeked {
            return false;
        }
        // SAFETY: the native queue is live, and this is the single consumer.
        if !unsafe { splice_events_ack(self.handle.as_ptr(), count) } {
            return false;
        }
        self.peeked -= count;
        true
    }

    /// Number of events dropped because the queue reached its capacity.
    pub fn dropped(&self) -> u64 {
        // SAFETY: self owns the live ring.
        unsafe { splice_events_dropped(self.handle.as_ptr()) }
    }
}

impl Drop for EventRing {
    fn drop(&mut self) {
        // SAFETY: the constructor transferred one retained handle to this owner.
        unsafe { splice_events_release(self.handle.as_ptr()) };
    }
}

/// The focused text element's caret (or selection) in global screen points, `[x, y, w, h]`
/// with a top-left origin, from Accessibility (T45.4). `None` without the Accessibility
/// permission (it never prompts) or without a focused text element.
pub fn caret() -> Option<[f64; 4]> {
    let mut rect = [0.0_f64; 4];
    // SAFETY: the bridge writes at most four doubles into `rect` and keeps no pointer.
    unsafe { splice_ax_caret(rect.as_mut_ptr()) }.then_some(rect)
}
