//! Sizes and limits of the protocol. Each value is defined here and nowhere else.

/// The largest payload one frame may carry. Larger messages are split across frames.
pub const MAX_PAYLOAD: usize = 64 * 1024;

/// How many bytes of terminal output a client lets the host send on one stream before it
/// must grant more. This bounds what a slow client has to buffer per stream.
///
/// It also bounds how long one stream can hold up another. Everything for a client
/// travels in one order, so a key's echo waits behind whatever a flooding stream already
/// has on the way: at most one window of it. Twice this window carries a flood about
/// forty per cent faster and makes that wait about forty per cent longer; a smaller one
/// starves the sender while it waits for each grant.
pub const CREDIT_WINDOW: u32 = 512 * 1024;

/// The most credit a sender will hold for one stream. A peer that grants beyond this is
/// broken: honouring it would let one stream buffer without bound.
pub const MAX_WINDOW: u32 = 16 * 1024 * 1024;

/// The largest control-plane message after its frames are joined.
pub const MAX_CONTROL_MESSAGE: usize = 1024 * 1024;

/// The largest hydrate message after its frames are joined.
pub const MAX_HYDRATE: usize = 64 * 1024 * 1024;

/// The most status records kept for one session. A program may report a record for
/// each of its jobs; beyond this the record reported longest ago is forgotten.
pub const STATUS_RECORDS_MAX: usize = 64;

/// The most the status records in a session's whole state may come to when the state
/// is sent in one piece, which has to fit one frame with room for the rest of the
/// state. A state with more goes without its oldest records.
pub const STATE_STATUSES_MAX_BYTES: usize = 32 * 1024;
