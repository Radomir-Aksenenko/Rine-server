//! Headless Minecraft 26.3 (protocol 777) test client for verifying and load-testing the server.
//!
//! Usage: rine-testbot <host:port> [--bots N] [--name Prefix] [--dump] [--seconds S] [--walk]
//!                     [--cmd CMD]... [--cmd-delay MS] [--stats] [--expect-clean]
//!
//! Every clientbound play packet is checked by the strict validator (validate.rs).
//! Parse errors are printed with packet name, id and body offset. Only the
//! first bot's commands are sent, one after another, 2 s apart.

mod bot;
mod chunk;
mod names;
mod nbt;
mod validate;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use bot::{run_bot, BotOpts, State};
use names::play_name;
use validate::Stats;

struct Cfg {
    addr: String,
    bots: usize,
    prefix: String,
    dump: bool,
    seconds: f64,
    walk: bool,
    cmds: Vec<String>,
    cmd_delay_ms: u64,
    stats: bool,
    expect_clean: bool,
}

fn usage() {
    eprintln!(
        "usage: rine-testbot <host:port> [--bots N] [--name Prefix] [--dump] [--seconds S] [--walk] \
         [--cmd CMD]... [--cmd-delay MS] [--stats] [--expect-clean]"
    );
}

fn parse_args() -> Result<Cfg> {
    let mut args = std::env::args().skip(1);
    let mut cfg = Cfg {
        addr: String::new(),
        bots: 1,
        prefix: "bot".to_string(),
        dump: false,
        seconds: 10.0,
        walk: false,
        cmds: Vec::new(),
        cmd_delay_ms: 1500,
        stats: false,
        expect_clean: false,
    };
    let mut have_addr = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bots" => {
                cfg.bots = args.next().context("--bots needs a value")?.parse::<usize>().context("--bots")?;
            }
            "--name" => cfg.prefix = args.next().context("--name needs a value")?,
            "--seconds" => {
                cfg.seconds = args.next().context("--seconds needs a value")?.parse::<f64>().context("--seconds")?;
            }
            "--cmd-delay" => {
                cfg.cmd_delay_ms =
                    args.next().context("--cmd-delay needs a value")?.parse::<u64>().context("--cmd-delay")?;
            }
            "--dump" => cfg.dump = true,
            "--walk" => cfg.walk = true,
            "--stats" => cfg.stats = true,
            "--expect-clean" => cfg.expect_clean = true,
            "--cmd" => cfg.cmds.push(args.next().context("--cmd needs a value")?),
            "-h" | "--help" => {
                usage();
                std::process::exit(0);
            }
            s if !s.starts_with("--") && !have_addr => {
                cfg.addr = s.to_string();
                have_addr = true;
            }
            other => bail!("unknown argument {other}"),
        }
    }
    if !have_addr {
        usage();
        bail!("missing <host:port>");
    }
    if cfg.bots == 0 {
        bail!("--bots must be at least 1");
    }
    if !(cfg.seconds > 0.0) {
        bail!("--seconds must be positive");
    }
    Ok(cfg)
}

/// Prints the merged validator statistics (--stats) and the unique parse errors.
fn print_stats(total: &Stats, show_stats: bool) {
    if show_stats {
        println!("== packets validated OK (count, by name)");
        for (id, n) in &total.ok {
            println!("  {} (id {id}): {n}", play_name(*id));
        }
        println!("== unsupported (not errors): packet, reason, count");
        for ((id, reason), n) in &total.unsupported {
            println!("  {} (id {id}): {reason} x{n}", play_name(*id));
        }
        let unv: Vec<String> = total.unvalidated.iter().map(|(id, n)| format!("{id}:{}x{n}", play_name(*id))).collect();
        println!("== unvalidated ids (not errors): {}", if unv.is_empty() { "none".into() } else { unv.join(", ") });
        println!("== unknown entity id warnings (not errors)");
        if total.unknown_entity.is_empty() {
            println!("  none");
        }
        for (id, n) in &total.unknown_entity {
            println!("  {}: {n}", play_name(*id));
        }
    }
    println!("== parse errors: total={} unique={}", total.error_count, total.errors.len());
    for (i, ((id, msg), (count, off))) in total.errors.iter().enumerate() {
        if i >= 20 {
            println!("  ... {} more unique errors", total.errors.len() - 20);
            break;
        }
        println!(
            "  {} (id {id}) x{count} first at body offset {off}: {msg}",
            play_name(*id)
        );
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Arc::new(parse_args()?);
    let opts = BotOpts {
        dump: cfg.dump,
        walk: cfg.walk,
        cmds: Vec::new(),
        cmd_delay: Duration::from_millis(cfg.cmd_delay_ms),
    };

    let mut handles = Vec::with_capacity(cfg.bots);
    for i in 0..cfg.bots {
        let name = format!("{}{}", cfg.prefix, i);
        let mut opts = opts.clone();
        if i == 0 {
            opts.cmds = cfg.cmds.clone();
            // RINE_CMD_FILE: extra commands, one per line (# comments)
            if let Ok(f) = std::env::var("RINE_CMD_FILE") {
                if let Ok(t) = std::fs::read_to_string(f) {
                    opts.cmds.extend(t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(|l| l.trim_start_matches('/').to_string()));
                }
            }
        }
        let addr = cfg.addr.clone();
        let seconds = cfg.seconds;
        handles.push(tokio::spawn(run_bot(addr, name, opts, seconds)));
        if i + 1 < cfg.bots {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    let mut reports = Vec::with_capacity(cfg.bots);
    for h in handles {
        reports.push(h.await.context("bot task panicked")?);
    }

    for r in &reports {
        r.print(cfg.dump);
    }

    let total_bots = reports.len();
    let reached_play = reports.iter().filter(|r| r.state == State::Play).count();
    let total_chunks: u64 = reports.iter().map(|r| r.chunks).sum();
    let total_errors: u64 = reports.iter().map(|r| r.parse_errors).sum();
    let rates: Vec<f64> = reports.iter().filter_map(|r| r.chunks_per_sec()).collect();
    let avg_rate = if rates.is_empty() { 0.0 } else { rates.iter().sum::<f64>() / rates.len() as f64 };
    println!(
        "== aggregate: bots={total_bots} reached_play={reached_play} total_chunks={total_chunks} total_errors={total_errors} avg_chunks_per_sec={avg_rate:.2} (mean per play bot)"
    );

    let mut merged = Stats::default();
    for r in &reports {
        merged.merge(&r.stats);
    }
    print_stats(&merged, cfg.stats);

    if cfg.expect_clean && total_errors > 0 {
        eprintln!("--expect-clean: {total_errors} parse error(s)");
        std::process::exit(1);
    }
    Ok(())
}
