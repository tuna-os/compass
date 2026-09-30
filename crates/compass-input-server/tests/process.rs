//! The built `compass-input-server`, driven over its stdin and stdout the way
//! the engine drives it. No call here injects anything.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use compass_core::input_server::wire::{Call, ExpansionMode, ServerMessage};
use compass_core::input_server::{MessageBuffer, frame};
use serde_json::json;

#[test]
fn the_server_answers_calls_and_exits_when_stdin_closes() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_compass-input-server"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the server starts");

    let mut stdin = child.stdin.take().unwrap();
    let calls = [
        Call::GetCapabilities,
        Call::CreateSnippet {
            trigger: ";sig".into(),
            mode: ExpansionMode::Word,
        },
        Call::ResetContext,
    ];
    let mut bytes = Vec::new();
    for (id, call) in calls.iter().enumerate() {
        bytes.extend(frame(&call.encode(i32::try_from(id).unwrap() + 1)));
    }
    bytes.extend(frame(
        br#"{"jsonrpc":"2.0","method":"Snippet/nope","id":9,"params":{}}"#,
    ));
    stdin.write_all(&bytes).unwrap();
    stdin.flush().unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let mut buffer = MessageBuffer::new();
    let mut replies = Vec::new();
    let mut chunk = [0u8; 4096];
    while replies.len() < 4 {
        let read = stdout.read(&mut chunk).unwrap();
        if read == 0 {
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            let _ = child.wait();
            if stderr.contains("could not compile the default keymap") {
                eprintln!("SKIPPED: no xkb keymap data here: {stderr}");
                return;
            }
            panic!("the server closed stdout early: {stderr}");
        }
        for message in buffer.push(&chunk[..read]) {
            replies.push(ServerMessage::decode(&message).unwrap());
        }
    }

    let ServerMessage::Reply {
        id: 1,
        result: Ok(caps),
    } = &replies[0]
    else {
        panic!("{:?}", replies[0]);
    };
    let injection = caps["injection"].as_bool().expect("a bool");
    eprintln!("the server reports injection={injection} on this machine");
    assert_eq!(
        replies[1],
        ServerMessage::Reply {
            id: 2,
            result: Ok(json!({"ok": false}))
        }
    );
    assert_eq!(
        replies[2],
        ServerMessage::Reply {
            id: 3,
            result: Ok(serde_json::Value::Null)
        }
    );
    assert!(
        matches!(
            &replies[3],
            ServerMessage::Reply {
                id: 9,
                result: Err(_)
            }
        ),
        "an unknown method is answered with an error: {:?}",
        replies[3]
    );

    drop(stdin);
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "exited with {status}");
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the server did not exit after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
