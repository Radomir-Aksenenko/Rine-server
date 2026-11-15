//! `CommandResponseTracker`: sums the per-player results of a command and
//! picks the single-element or the multiple-element message.

use crate::game::cmd::Ctx;
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;

/// Which elements count: every tracked element (`ANY`) or only those with a
/// non-zero value (`NON_ZERO`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Any,
    NonZero,
}

/// Elements are player slots.
#[derive(Default)]
pub struct Tracker {
    pub total: i32,
    count: usize,
    only: Option<usize>,
    nz_count: usize,
    only_nz: Option<usize>,
}

impl Tracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn track(&mut self, slot: usize, value: i32) {
        self.total += value;
        self.count += 1;
        self.only = if self.count == 1 { Some(slot) } else { None };
        if value != 0 {
            self.nz_count += 1;
            self.only_nz = if self.nz_count == 1 { Some(slot) } else { None };
        }
    }

    pub fn track_bool(&mut self, slot: usize, v: bool) {
        self.track(slot, v as i32);
    }

    fn first(&self, k: Kind) -> Option<usize> {
        match k {
            Kind::Any => self.only,
            Kind::NonZero => self.only_nz,
        }
    }

    pub fn count(&self, k: Kind) -> usize {
        match k {
            Kind::Any => self.count,
            Kind::NonZero => self.nz_count,
        }
    }

    /// `sendFeedback(source, true, NON_ZERO, messages)`. `fixed` is the
    /// `onZero` error (throws when nothing had a non-zero value).
    pub fn send(&self, x: &mut Ctx, fixed: Option<&'static str>, single: &'static str, multi: &'static str, pre: &[Text]) -> CmdResult<i32> {
        if let Some(key) = fixed {
            if self.count(Kind::NonZero) == 0 {
                return Err(CmdError::key(key));
            }
        }
        let t = self.text(x, Kind::NonZero, single, multi, pre);
        x.success(t, true);
        Ok(self.total)
    }

    /// `throw dispatch(ANY, ...)`: the failure message for "nothing changed".
    pub fn fail(&self, x: &Ctx, single: &'static str, multi: &'static str, pre: &[Text]) -> CmdError {
        CmdError::new(self.text(x, Kind::Any, single, multi, pre))
    }

    /// `dispatch`: the single-element message with the player name appended
    /// after `pre`, or the multiple-element message with the count.
    pub fn text(&self, x: &Ctx, k: Kind, single: &'static str, multi: &'static str, pre: &[Text]) -> Text {
        match self.first(k) {
            Some(slot) => {
                let mut args = pre.to_vec();
                args.push(x.player_name(slot));
                Text::tr(single, args)
            }
            None => {
                let mut args = pre.to_vec();
                args.push(Text::plain(self.count(k).to_string()));
                Text::tr(multi, args)
            }
        }
    }
}

