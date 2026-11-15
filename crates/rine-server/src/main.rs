//! Rine: Minecraft Java Edition 26.3 server.

mod config;
mod game;
mod net;
mod out;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let t0 = std::time::Instant::now();
    rine_data::init();
    let config = config::Config::load_or_create("rine.toml");
    out::THRESHOLD.store(config.compression_threshold, Ordering::Relaxed);
    tracing::info!("data loaded in {:?}", t0.elapsed());

    let (new_tx, new_rx) = crossbeam_channel::unbounded();
    let (wake_tx, wake_rx) = crossbeam_channel::bounded::<()>(1);
    let shared = Arc::new(net::Shared {
        config: config.clone(),
        online: AtomicUsize::new(0),
        new_players: new_tx,
        wake: wake_tx,
        config_frames: net::build_config_frames(config.compression_threshold),
    });

    let running = Arc::new(AtomicBool::new(true));
    {
        let r = running.clone();
        let _ = ctrlc_hook(move || r.store(false, Ordering::Relaxed));
    }
    let mut game = game::Game::new(shared.clone(), new_rx, wake_rx, running.clone());
    let (console_tx, console_rx) = crossbeam_channel::unbounded::<String>();
    game.console_rx = Some(console_rx);
    tracing::info!("spawn at {:?}", game.spawn);
    let game_thread = std::thread::Builder::new().name("game".into()).spawn(move || game.run())?;

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().thread_name("net").build()?;
    rt.spawn(async move {
        if let Err(e) = net::listen(shared).await {
            tracing::error!("listener: {e}");
            std::process::exit(1);
        }
    });
    // console: every line goes to the command dispatcher at op level 4
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match std::io::stdin().read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let l = line.trim();
                    if !l.is_empty() && console_tx.send(l.to_string()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let _ = game_thread.join();
    rt.shutdown_timeout(std::time::Duration::from_secs(1));
    Ok(())
}

fn ctrlc_hook(f: impl Fn() + Send + 'static) -> std::io::Result<()> {
    // minimal Ctrl+C: tokio signal on a helper runtime thread
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let _ = tokio::signal::ctrl_c().await;
        });
        f();
    });
    Ok(())
}
