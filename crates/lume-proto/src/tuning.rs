//! Sizes and limits of the protocol. Each value is defined here and nowhere else.

/// The largest payload one frame may carry. Larger messages are split across frames.
pub const MAX_PAYLOAD: usize = 64 * 1024;

/// How many bytes of terminal output a client lets the host send on one stream before it
/// must grant more. This bounds what a slow client has to buffer per stream.
pub const CREDIT_WINDOW: u32 = 1024 * 1024;

/// The most credit a sender will hold for one stream. A peer that grants beyond this is
/// broken: honouring it would let one stream buffer without bound.
pub const MAX_WINDOW: u32 = 16 * 1024 * 1024;

/// The largest control-plane message after its frames are joined.
pub const MAX_CONTROL_MESSAGE: usize = 1024 * 1024;

/// The largest hydrate message after its frames are joined.
pub const MAX_HYDRATE: usize = 64 * 1024 * 1024;
