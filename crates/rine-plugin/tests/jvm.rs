//! End-to-end test: starts a real JVM and loads the bundled hello example plugin.
//! If no JVM can be found, the test prints a skip message and passes.

use std::path::Path;
use std::time::Duration;

use rine_plugin::{Action, Event, PluginHost, SyncKind};

fn collect_actions(host: &PluginHost, ticks: usize) -> Vec<Action> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        out.extend(host.tick());
        std::thread::sleep(Duration::from_millis(50));
    }
    out
}

#[test]
fn hello_plugin_end_to_end() {
    let hello_jar = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin-api/examples/hello.jar");
    assert!(
        hello_jar.is_file(),
        "missing {} - run plugin-api/build.ps1 first",
        hello_jar.display()
    );

    let dir = std::env::temp_dir().join(format!("rine-plugin-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    std::fs::copy(&hello_jar, dir.join("hello.jar")).expect("copy hello.jar");

    let host = match PluginHost::start(&dir) {
        Ok(Some(host)) => host,
        Ok(None) => {
            eprintln!("SKIP: no JVM found (set JAVA_HOME to a JDK); jvm test not run");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        Err(e) => panic!("PluginHost::start failed: {e:?}"),
    };

    // Join: the plugin broadcasts a greeting containing the name.
    host.post(Event::Join {
        name: "Tester".into(),
        uuid: "00000000-0000-0000-0000-000000000001".into(),
    });
    let actions = collect_actions(&host, 10);
    assert!(
        actions.iter().any(|a| matches!(a, Action::Broadcast(t) if t.contains("Tester"))),
        "expected a Broadcast containing the name, got {actions:?}"
    );

    // Command: /hello replies to the sender.
    host.post(Event::Command {
        name: "Tester".into(),
        line: "/hello".into(),
    });
    let actions = collect_actions(&host, 10);
    assert!(
        actions.iter().any(|a| matches!(
            a,
            Action::Message { player, text } if player == "Tester" && text.contains("Привет")
        )),
        "expected a Message reply to /hello, got {actions:?}"
    );

    // Sync check: the plugin cancels diamond ore, and only diamond ore.
    assert!(host.wants_sync(SyncKind::BlockBreak), "plugin should register a BlockBreak listener");
    let diamond = Event::BlockBreak {
        name: "Tester".into(),
        x: 1,
        y: 2,
        z: 3,
        block: "minecraft:diamond_ore".into(),
    };
    assert!(host.sync_cancel(&diamond), "diamond ore break should be cancelled");

    let stone = Event::BlockBreak {
        name: "Tester".into(),
        x: 1,
        y: 2,
        z: 3,
        block: "minecraft:stone".into(),
    };
    assert!(!host.sync_cancel(&stone), "stone break should not be cancelled");

    // Commands registered by the plugin.
    assert!(
        host.commands().iter().any(|c| c == "hello"),
        "commands() should contain hello, got {:?}",
        host.commands()
    );

    host.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
