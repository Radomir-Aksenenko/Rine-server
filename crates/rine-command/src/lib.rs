//! Vanilla 26.3 command grammar: Brigadier-compatible reader, dispatcher
//! and argument parsers over the tree captured from the vanilla server.
//! Independent of the game state; the server attaches executors.

pub mod args;
pub mod dispatch;
pub mod error;
pub mod lang;
pub mod packet;
pub mod reader;
pub mod tree;

pub use dispatch::{parse, Chain, Context, ParseResults};
pub use error::{CmdError, CmdResult};
pub use reader::Reader;
pub use tree::{NodeId, Tree};
