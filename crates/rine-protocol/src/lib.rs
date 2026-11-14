//! Rine protocol primitives for Minecraft Java Edition 26.3 (protocol 777).
//!
//! Design: packets are written straight into a reusable `BytesMut` with no
//! intermediate packet objects for hot clientbound traffic. Anything that is
//! broadcast (chunks, entity moves, block changes) is encoded ONCE into a
//! `Frame` (already length-prefixed and, if needed, compressed) and the same
//! `Bytes` is cloned (refcount bump) into every viewer's outbound queue.

pub mod buf;
pub mod frame;
pub mod nbt;
pub mod ids;
pub mod text;

pub use buf::{Reader, Writer, ProtoError, BlockPos};
pub use frame::{Frame, FrameDecoder, encode_frame};

pub const PROTOCOL_VERSION: i32 = 777;
pub const VERSION_NAME: &str = "26.3";
