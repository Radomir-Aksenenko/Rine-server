//! /gamerule (`GameRuleCommand`).

use crate::game::cmd::rules::{GameRules, Value, RULES};
use crate::game::cmd::{lit, Commands, Ctx};
use rine_command::{CmdError, CmdResult};
use rine_protocol::text::Text;

pub fn register(c: &mut Commands) {
    for r in RULES {
        for name in [r.name.to_string(), format!("minecraft:{}", r.name)] {
            c.exec(&format!("gamerule {name}"), query);
            c.exec(&format!("gamerule {name} value"), set);
        }
    }
}

fn rule_index(x: &Ctx) -> usize {
    GameRules::index(x.lit(1)).expect("gamerule literal")
}

fn show(v: Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
    }
}

fn result(v: Value) -> i32 {
    match v {
        Value::Bool(b) => b as i32,
        Value::Int(i) => i,
    }
}

fn query(x: &mut Ctx) -> CmdResult<i32> {
    let i = rule_index(x);
    let v = x.g.rules.get(i);
    x.success(Text::tr("commands.gamerule.query", vec![lit(RULES[i].name), lit(show(v))]), false);
    Ok(result(v))
}

fn set(x: &mut Ctx) -> CmdResult<i32> {
    let i = rule_index(x);
    let v = match RULES[i].default {
        Value::Bool(_) => Value::Bool(x.bool("value")),
        Value::Int(_) => Value::Int(x.int("value")),
    };
    if x.g.rules.get(i) == v {
        return Err(CmdError::tr("commands.gamerule.not_set", vec![lit(RULES[i].name), lit(show(v))]));
    }
    x.g.rules.set(i, v);
    crate::game::cmd::commands::on_rule_changed(x.g, RULES[i].name);
    x.success(Text::tr("commands.gamerule.set", vec![lit(RULES[i].name), lit(show(v))]), true);
    Ok(result(v))
}
