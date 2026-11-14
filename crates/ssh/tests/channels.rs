//! Commands and terminals: exit statuses and signals, stdin and its EOF,
//! output order, PTY sizes and the environment, many channels on one
//! connection and many connections at once.

mod support;

use std::time::Duration;

use bytes::Bytes;
use moshpit_ssh::{Event, Exit, PtyRequest, Sig, SshError, Stream};
use support::*;

#[tokio::test]
async fn a_command_reports_its_output_and_exit_status() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;

    let out = ssh.run("echo hello").await.unwrap();
    assert_eq!(out.stdout(), b"hello\n");
    assert_eq!(out.stderr(), b"");
    assert_eq!(out.exit, Some(Exit::Status(0)));
    assert!(out.closed && out.success());

    let out = ssh.run("fail 3").await.unwrap();
    assert_eq!(out.stderr(), b"nope\n");
    assert_eq!(out.status(), Some(3));
    assert!(!out.success());

    let out = ssh.run("no-such-thing").await.unwrap();
    assert_eq!(out.status(), Some(127));
    assert_eq!(out.stderr(), b"no-such-thing: command not found\n");
}

/// B138: the account's shell gets one word to hand sh, so a fish login
/// shell never parses the app's sh — `$?`, a loop, a backslash in quotes.
#[tokio::test]
async fn every_command_reaches_the_shell_as_sh_c_and_one_word() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let script = "herdr x; echo \"rc=$?\" 'a\\'";
    let _ = ssh.run(script).await.unwrap();
    let seen = host.seen();
    let request = seen.requests.last().unwrap();
    assert_eq!(request, r#"sh -c 'herdr x; echo "rc=$?" '\''a'\\''\'''"#);
    assert_eq!(seen.commands.last().unwrap(), script);
}

/// What `executeCommand(_, mergeStreams: true)` gave the app: both streams,
/// in arrival order.
#[tokio::test]
async fn merged_output_keeps_arrival_order() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let out = ssh.run("interleave").await.unwrap();
    assert_eq!(out.merged(), b"abcd");
    assert_eq!(out.stdout(), b"bd");
    assert_eq!(out.stderr(), b"ac");
    assert_eq!(
        out.chunks.iter().map(|(s, _)| *s).collect::<Vec<_>>(),
        [
            Stream::Stderr,
            Stream::Stdout,
            Stream::Stderr,
            Stream::Stdout
        ]
    );
}

/// A channel the host closed without saying how the program ended: no
/// status, and the caller can tell (Citadel took it for success).
#[tokio::test]
async fn a_command_without_an_exit_status_is_not_a_success() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let out = ssh.run("no-status").await.unwrap();
    assert_eq!(out.stdout(), b"partial");
    assert_eq!(out.exit, None);
    assert!(out.closed);
    assert!(!out.success());
}

#[tokio::test]
async fn large_output_arrives_whole_and_in_order() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let n = 3 * 1024 * 1024 + 17;
    let out = ssh.run(&format!("noise {n}")).await.unwrap();
    let expected: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
    assert_eq!(out.stdout().len(), n);
    assert!(out.stdout() == expected);
}

#[tokio::test]
async fn stdin_streams_through_and_its_eof_ends_the_command() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut cat = ssh.exec("cat").await.unwrap();
    cat.write(b"ping\n").await.unwrap();
    assert_eq!(cat.next().await, Some(Event::Stdout(Bytes::from("ping\n"))));
    cat.write(b"pong\n").await.unwrap();
    assert_eq!(cat.next().await, Some(Event::Stdout(Bytes::from("pong\n"))));
    cat.close_stdin().await.unwrap();
    assert_eq!(cat.next().await, Some(Event::Exited(Exit::Status(0))));
    assert_eq!(cat.next().await, Some(Event::Eof));
    assert_eq!(cat.next().await, Some(Event::Closed));
    assert_eq!(cat.next().await, None);
}

/// Writes go out whole and in the order they were made, while one task
/// pumps the output — as the terminal's reader does. (Output nobody reads
/// holds the whole connection up: russh delivers to a bounded queue.)
#[tokio::test]
async fn writes_keep_their_order() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let cat = ssh.exec("cat").await.unwrap();
    let (mut reader, writer) = cat.split();
    let pump = tokio::spawn(async move {
        let mut got = Vec::new();
        while let Some(event) = reader.next().await {
            if let Event::Stdout(bytes) = event {
                got.extend_from_slice(&bytes);
            }
        }
        got
    });
    let mut expected = Vec::new();
    for i in 0..200 {
        let line = format!("{i:04} {}\n", "x".repeat(i * 37 % 4000));
        expected.extend_from_slice(line.as_bytes());
        writer.write(line.as_bytes()).await.unwrap();
    }
    writer.close_stdin().await.unwrap();
    let got = within(10, pump).await.unwrap();
    assert!(got == expected);
}

#[tokio::test]
async fn a_signal_ends_a_command_and_says_which() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut sleeper = ssh.exec("sleep").await.unwrap();
    sleeper.signal(Sig::TERM).await.unwrap();
    let (events, _) = within(5, drain(&mut sleeper)).await;
    assert!(events.contains(&Event::Exited(Exit::Signal {
        signal: "TERM".into(),
        core_dumped: false,
        message: "killed".into(),
    })));
    assert_eq!(events.last(), Some(&Event::Closed));
    assert_eq!(host.seen().signals, vec!["TERM"]);
}

#[tokio::test]
async fn a_refused_command_is_an_error() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let err = ssh.exec("refuse").await.err().unwrap();
    assert!(matches!(err, SshError::Refused { .. }), "{err:?}");
    // The connection is fine.
    assert!(ssh.run("echo still here").await.unwrap().success());
}

#[tokio::test]
async fn many_channels_share_one_connection() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut runs = tokio::task::JoinSet::new();
    for i in 0..24 {
        let ssh = ssh.clone();
        runs.spawn(async move { (i, ssh.run(&format!("echo {i}")).await.unwrap()) });
    }
    let mut done = 0;
    while let Some(result) = runs.join_next().await {
        let (i, out) = result.unwrap();
        assert_eq!(out.stdout(), format!("{i}\n").into_bytes());
        done += 1;
    }
    assert_eq!(done, 24);
    assert_eq!(host.seen().connections, 1);
}

/// The mosh bootstrap and its `-CC` sidecar, or a reconnect racing the old
/// connection: separate connections, separate fates.
#[tokio::test]
async fn several_connections_live_and_end_independently() {
    let host = Host::start(Bench::default()).await;
    let (a, b) = tokio::join!(connect(&host), connect(&host));
    assert_eq!(host.seen().connections, 2);
    a.close().await;
    assert!(a.is_closed());
    assert!(!b.is_closed());
    assert!(b.run("echo b").await.unwrap().success());
    assert!(a.run("echo a").await.is_err());
}

#[tokio::test]
async fn a_terminal_gets_its_size_type_and_utf8_locale() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut request = PtyRequest::new(120, 40);
    request.command = Some("env LANG".into());
    let mut term = ssh.terminal(&request).await.unwrap();
    let (events, stdout) = within(5, drain(&mut term)).await;
    assert_eq!(stdout, b"en_US.UTF-8\n");
    assert_eq!(events.last(), Some(&Event::Closed));
    let seen = host.seen();
    assert_eq!(seen.ptys, vec![("xterm-256color".to_owned(), 120, 40)]);
    assert_eq!(
        seen.env,
        vec![("LANG".to_owned(), "en_US.UTF-8".to_owned())]
    );
}

#[tokio::test]
async fn a_terminal_follows_resizes() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut request = PtyRequest::new(80, 24);
    request.command = Some("watch-size".into());
    let mut term = ssh.terminal(&request).await.unwrap();
    term.resize(132, 50).await.unwrap();
    assert!(
        within(5, read_until(&mut term, "50 132"))
            .await
            .contains("50 132")
    );
    term.resize(40, 90).await.unwrap();
    assert!(
        within(5, read_until(&mut term, "90 40"))
            .await
            .contains("90 40")
    );
    assert_eq!(host.seen().window_changes, vec![(132, 50), (40, 90)]);
    term.signal(Sig::HUP).await.unwrap();
    let (events, _) = within(5, drain(&mut term)).await;
    assert_eq!(events.last(), Some(&Event::Closed));
}

/// A shell typed into, and left with `exit`: the host closes the channel
/// after the status, and the connection stays up for whatever comes next.
#[tokio::test]
async fn exit_in_a_shell_ends_the_terminal_cleanly() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut term = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    assert!(within(5, read_until(&mut term, "$ ")).await.ends_with("$ "));
    term.write(b"echo hi\r").await.unwrap();
    assert!(
        within(5, read_until(&mut term, "$ "))
            .await
            .contains("echo hi")
    );
    term.write(b"exit\r").await.unwrap();
    let (events, stdout) = within(5, drain(&mut term)).await;
    assert!(String::from_utf8_lossy(&stdout).contains("logout"));
    let tail: Vec<&Event> = events
        .iter()
        .filter(|e| !matches!(e, Event::Stdout(_)))
        .collect();
    assert_eq!(
        tail,
        [&Event::Exited(Exit::Status(0)), &Event::Eof, &Event::Closed]
    );
    assert!(!ssh.is_closed());
    assert_eq!(ssh.end(), None);
    assert!(ssh.run("echo after").await.unwrap().success());
    tokio::time::sleep(Duration::from_millis(50)).await;
}
