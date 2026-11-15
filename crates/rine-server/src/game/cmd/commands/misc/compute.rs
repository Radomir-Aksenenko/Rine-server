//! /compute (`ComputeCommand`). The number-provider evaluation is ported for
//! inline constants (`1.5`, `3`). Named providers (registry ids such as
//! `minecraft:uniform` data) and the `block`/`entity` forms need the loot
//! context and the provider data, which Rine does not load; they report a
//! stub error.

use super::stub;
use crate::game::cmd::{Commands, Ctx};
use rine_command::args::misc::IdOrInline;
use rine_command::args::Arg;
use rine_command::error::java_float;
use rine_command::{CmdError, CmdResult};
use rine_protocol::nbt::Tag;
use rine_protocol::text::Text;

pub fn register(c: &mut Commands) {
    c.exec("compute default float provider", float);
    c.exec("compute default float provider scale", float);
    c.exec("compute default integer provider", int);
    c.exec("compute block computePos float provider", float_ctx);
    c.exec("compute block computePos float provider scale", float_ctx);
    c.exec("compute block computePos integer provider", int_ctx);
    c.exec("compute entity computeTarget float provider", float_ctx);
    c.exec("compute entity computeTarget float provider scale", float_ctx);
    c.exec("compute entity computeTarget integer provider", int_ctx);
}

/// Value of an inline constant provider; `None` for anything else.
fn constant(t: &Tag) -> Option<f64> {
    match t {
        Tag::Byte(v) => Some(*v as f64),
        Tag::Short(v) => Some(*v as f64),
        Tag::Int(v) => Some(*v as f64),
        Tag::Long(v) => Some(*v as f64),
        Tag::Float(v) => Some(*v as f64),
        Tag::Double(v) => Some(*v),
        _ => None,
    }
}

/// The provider argument as a constant, or a stub for anything Rine cannot evaluate.
fn provider_value(x: &Ctx) -> CmdResult<f64> {
    match x.arg("provider") {
        Arg::IdOrInline(IdOrInline::Inline(t)) => constant(t).ok_or_else(|| stub(x)),
        _ => Err(stub(x)),
    }
}

/// `ComputeCommand.computeAsFloat` for an unnamed (inline) provider.
fn float(x: &mut Ctx) -> CmdResult<i32> {
    let v = provider_value(x)?;
    let scale = if x.has("scale") { x.float("scale") } else { 1.0 };
    compute_float(x, v as f32, scale)
}

fn compute_float(x: &mut Ctx, original: f32, scale: f32) -> CmdResult<i32> {
    if !original.is_finite() {
        return Err(CmdError::tr("command.compute.result.unnamed.invalid", vec![Text::plain(java_float(original))]));
    }
    let result = (original * scale).floor() as i32;
    if result as f32 == original {
        x.success(Text::tr("command.compute.result.unnamed.exact", vec![Text::plain(result.to_string())]), false);
    } else {
        x.success(
            Text::tr(
                "command.compute.result.unnamed.rounded",
                vec![Text::plain(java_float(original)), Text::plain(result.to_string())],
            ),
            false,
        );
    }
    Ok(result)
}

/// `ComputeCommand.computeAsInt` for an unnamed (inline) provider.
fn int(x: &mut Ctx) -> CmdResult<i32> {
    let v = provider_value(x)? as i32;
    x.success(Text::tr("command.compute.result.unnamed.exact", vec![Text::plain(v.to_string())]), false);
    Ok(v)
}

/// `compute block|entity`: needs the loot context of the source.
fn float_ctx(x: &mut Ctx) -> CmdResult<i32> {
    Err(stub(x))
}

fn int_ctx(x: &mut Ctx) -> CmdResult<i32> {
    Err(stub(x))
}
