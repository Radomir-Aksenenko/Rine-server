//! Headless Minecraft 26.3 (protocol 777) test client for verifying and load-testing the server.
//!
//! Usage: rine-testbot <host:port> [--bots N] [--name Prefix] [--dump] [--seconds S] [--walk]

mod bot;
mod chunk;
mod nbt;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use bot::{run_bot, BotOpts, State};

struct Cfg {
    addr: String,
    bots: usize,
    prefix: String,
    dump: bool,
    seconds: f64,
    walk: bool,
    cmd: Option<String>,
}

fn usage() {
    eprintln!("usage: rine-testbot <host:port> [--bots N] [--name Prefix] [--dump] [--seconds S] [--walk]");
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
        cmd: None,
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
            "--dump" => cfg.dump = true,
            "--walk" => cfg.walk = true,
            "--cmd" => cfg.cmd = Some(args.next().context("--cmd needs a value")?),
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

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Arc::new(parse_args()?);
    let opts = BotOpts { dump: cfg.dump, walk: cfg.walk, cmd: None };

    let mut handles = Vec::with_capacity(cfg.bots);
    for i in 0..cfg.bots {
        let name = format!("{}{}", cfg.prefix, i);
        let mut opts = opts.clone();
        if i == 0 { opts.cmd = cfg.cmd.clone(); }
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

    let total = reports.len();
    let reached_play = reports.iter().filter(|r| r.state == State::Play).count();
    let total_chunks: u64 = reports.iter().map(|r| r.chunks).sum();
    let total_errors: u64 = reports.iter().map(|r| r.parse_errors).sum();
    let rates: Vec<f64> = reports.iter().filter_map(|r| r.chunks_per_sec()).collect();
    let avg_rate = if rates.is_empty() { 0.0 } else { rates.iter().sum::<f64>() / rates.len() as f64 };
    println!(
        "== aggregate: bots={total} reached_play={reached_play} total_chunks={total_chunks} total_errors={total_errors} avg_chunks_per_sec={avg_rate:.2} (mean per play bot)"
    );
    Ok(())
}
