//! How connections end, and how they fail to start: a program that
//! finished is not a dropped line (B103), a goodbye from the server is not
//! a reset, our own close is ours; refused ports, black holes and servers
//! that never speak SSH each fail as themselves, and a connect given up
//! leaves nothing behind.

mod support;

use std::time::Duration;

use moshpit_ssh::{ConnectionEnd, Event, Exit, PtyRequest, SshError, Target};
use support::*;
use tokio::io::AsyncReadExt;

fn through(relay: &Relay) -> Target {
    Target::new(USER, "127.0.0.1", relay.port)
}

async fn connect_to(target: &Target) -> moshpit_ssh::Ssh {
    moshpit_ssh::Ssh::connect(
        target,
        moshpit_ssh::Auth::key(fixture_key("ed25519")),
        std::sync::Arc::new(AcceptAny),
        &options(),
    )
    .await
    .unwrap()
}

/// B103's two cases side by side. `exit` in a plain SSH shell: the
/// terminal's channel closes with a status and the connection is still
/// there. A dropped line: the channel is lost, and so is the connection.
#[tokio::test]
async fn a_finished_shell_and_a_dropped_line_are_told_apart() {
    let host = Host::start(Bench::default()).await;

    let relay = Relay::start(host.addr).await;
    let ssh = connect_to(&through(&relay)).await;
    let mut term = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    within(5, read_until(&mut term, "$ ")).await;
    term.write(b"exit\r").await.unwrap();
    let (events, _) = within(5, drain(&mut term)).await;
    assert!(events.contains(&Event::Exited(Exit::Status(0))));
    assert_eq!(events.last(), Some(&Event::Closed));
    assert_eq!(ssh.end(), None, "the connection outlives the shell");

    let mut term = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    within(5, read_until(&mut term, "$ ")).await;
    relay.cut();
    let (events, _) = within(5, drain(&mut term)).await;
    assert_eq!(events.last(), Some(&Event::Lost));
    assert!(!events.iter().any(|e| matches!(e, Event::Exited(_))));
    let end = within(5, ssh.ended()).await;
    assert!(end.is_lost(), "{end:?}");
    assert!(ssh.is_closed());
}

#[tokio::test]
async fn a_goodbye_from_the_server_is_not_a_drop() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut term = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    let _ = ssh.exec("disconnect").await;
    match within(5, ssh.ended()).await {
        ConnectionEnd::ServerDisconnected { message, .. } => {
            assert_eq!(message, "the host is going down");
        }
        other => panic!("{other:?}"),
    }
    let (events, _) = within(5, drain(&mut term)).await;
    assert_eq!(events.last(), Some(&Event::Lost));
}

#[tokio::test]
async fn our_own_close_is_ours() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let mut cat = ssh.exec("cat").await.unwrap();
    within(5, ssh.close()).await;
    assert_eq!(within(5, ssh.ended()).await, ConnectionEnd::ClosedByUs);
    assert!(ssh.is_closed());
    let (events, _) = within(5, drain(&mut cat)).await;
    assert_eq!(events.last(), Some(&Event::Lost));
    assert!(matches!(
        ssh.run("echo late").await,
        Err(SshError::Closed { .. })
    ));
}

#[tokio::test]
async fn a_closed_port_is_unreachable() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let err = moshpit_ssh::Ssh::connect(
        &Target::new(USER, "127.0.0.1", port),
        moshpit_ssh::Auth::password("x"),
        std::sync::Arc::new(AcceptAny),
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(
        matches!(err, SshError::Unreachable { port: p, .. } if p == port),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_name_that_does_not_resolve_is_unreachable() {
    let err = moshpit_ssh::Ssh::connect(
        &Target::new(USER, "no-such-host.invalid", 22),
        moshpit_ssh::Auth::password("x"),
        std::sync::Arc::new(AcceptAny),
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, SshError::Unreachable { .. }), "{err:?}");
}

/// A port that accepts and never speaks SSH: the handshake runs out, and
/// the socket is let go — the session task doesn't sit on it forever.
#[tokio::test]
async fn a_silent_server_times_the_handshake_out_and_is_let_go() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        // Read what the client says until it hangs up.
        let mut sink = Vec::new();
        let _ = socket.read_to_end(&mut sink).await;
        sink
    });
    let options = moshpit_ssh::ConnectOptions {
        handshake_timeout: Duration::from_millis(300),
        ..options()
    };
    let err = moshpit_ssh::Ssh::connect(
        &Target::new(USER, "127.0.0.1", port),
        moshpit_ssh::Auth::password("x"),
        std::sync::Arc::new(AcceptAny),
        &options,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, SshError::HandshakeTimeout { .. }), "{err:?}");
    let said = within(5, accepted).await.unwrap();
    assert!(said.starts_with(b"SSH-2.0-"), "{said:?}");
}

#[tokio::test]
async fn a_server_that_is_not_ssh_fails_the_handshake() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        use tokio::io::AsyncWriteExt;
        let _ = socket
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
    });
    let err = moshpit_ssh::Ssh::connect(
        &Target::new(USER, "127.0.0.1", port),
        moshpit_ssh::Auth::password("x"),
        std::sync::Arc::new(AcceptAny),
        &options(),
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(err, SshError::Handshake { .. }), "{err:?}");
}

/// Never a reconnect underneath the caller: a lost connection stays lost.
#[tokio::test]
async fn a_lost_connection_stays_lost() {
    let host = Host::start(Bench::default()).await;
    let relay = Relay::start(host.addr).await;
    let ssh = connect_to(&through(&relay)).await;
    relay.cut();
    assert!(within(5, ssh.ended()).await.is_lost());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(host.seen().connections, 1);
    assert!(ssh.run("echo again").await.is_err());
}

/// A half-open connection: the host never answers the request. Citadel gave
/// channel creation 15 s; here the whole request has a budget.
#[tokio::test]
async fn a_request_the_host_never_answers_times_out() {
    let host = Host::start(Bench::default()).await;
    let options = moshpit_ssh::ConnectOptions {
        channel_timeout: Duration::from_millis(300),
        ..options()
    };
    let ssh = moshpit_ssh::Ssh::connect(
        &host.target(),
        moshpit_ssh::Auth::key(fixture_key("ed25519")),
        std::sync::Arc::new(AcceptAny),
        &options,
    )
    .await
    .unwrap();
    let err = within(5, ssh.exec("hang")).await.err().unwrap();
    assert!(matches!(err, SshError::Unresponsive { .. }), "{err:?}");
    // Not the connection's fault: it still runs commands.
    assert!(ssh.run("echo fine").await.unwrap().success());
}

/// Input stuck behind a dead line (the window never reopens, the socket
/// buffers are full) gives up when the line is declared dead, rather than
/// waiting forever for a window nobody will open.
#[tokio::test]
async fn a_write_stuck_on_a_dead_line_gives_up_when_it_dies() {
    let host = Host::start(Bench::default()).await;
    let relay = Relay::start(host.addr).await;
    let ssh = connect_to(&through(&relay)).await;
    let cat = ssh.exec("cat").await.unwrap();
    let (mut reader, writer) = cat.split();
    tokio::spawn(async move { while reader.next().await.is_some() {} });
    relay.stall();
    let typing = tokio::spawn(async move {
        let chunk = vec![b'x'; 64 * 1024];
        loop {
            writer.write(&chunk).await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), SshError>(())
    });
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(!typing.is_finished(), "the writes should be stuck by now");
    relay.cut();
    let result = within(5, typing).await.unwrap();
    assert!(matches!(result, Err(SshError::Closed { .. })), "{result:?}");
    assert!(within(5, ssh.ended()).await.is_lost());
}

/// The debug fault hooks break the link the way a network does: a cut is a
/// drop (the terminal's channel lost, the connection lost), never a program
/// that finished; a stall says nothing at all, and a command on it never
/// answers.
#[tokio::test]
async fn injected_faults_are_drops_not_ends() {
    let host = Host::start(Bench::default()).await;

    let ssh = connect(&host).await;
    let mut term = ssh.terminal(&PtyRequest::new(80, 24)).await.unwrap();
    within(5, read_until(&mut term, "$ ")).await;
    ssh.inject_fault(moshpit_ssh::LinkFault::Cut);
    let (events, _) = within(5, drain(&mut term)).await;
    assert_eq!(events.last(), Some(&Event::Lost));
    assert!(!events.iter().any(|e| matches!(e, Event::Exited(_))));
    assert!(within(5, ssh.ended()).await.is_lost());

    let ssh = connect(&host).await;
    ssh.inject_fault(moshpit_ssh::LinkFault::Stall);
    let answer = tokio::time::timeout(Duration::from_millis(600), ssh.run("true")).await;
    assert!(
        answer.is_err(),
        "a stalled link answers nothing: {answer:?}"
    );
    assert_eq!(ssh.end(), None, "and says nothing either");
    // Given up on, it goes like any other connection.
    ssh.inject_fault(moshpit_ssh::LinkFault::Cut);
    assert!(within(5, ssh.ended()).await.is_lost());
}

/// B276: the two things a watcher of a quiet connection needs. When the
/// host last said anything moves with every answer and stops on a dead
/// link; a ping is answered by the host itself on a live one, and on a
/// stalled one it is `false` once its time is up, not a hang.
#[tokio::test]
async fn a_ping_and_the_last_word_tell_a_live_link_from_a_dead_one() {
    let host = Host::start(Bench::default()).await;
    let relay = Relay::start(host.addr).await;
    let ssh = connect_to(&through(&relay)).await;

    let before = ssh.last_heard();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(within(5, ssh.ping(Duration::from_secs(2))).await);
    let pinged = ssh.last_heard();
    assert!(pinged > before, "the ping's answer is a word from the host");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(ssh.run("echo hi").await.unwrap().success());
    assert!(ssh.last_heard() > pinged, "so is a command's");

    relay.stall();
    let stalled = ssh.last_heard();
    let asked = std::time::Instant::now();
    assert!(!ssh.ping(Duration::from_millis(400)).await);
    assert!(asked.elapsed() < Duration::from_secs(2));
    assert_eq!(ssh.last_heard(), stalled, "a dead link says nothing");
    assert_eq!(
        ssh.end(),
        None,
        "and a ping that went unanswered ends nothing"
    );

    relay.resume();
    assert!(within(5, ssh.ping(Duration::from_secs(2))).await);
    assert!(ssh.last_heard() > stalled);

    relay.cut();
    assert!(within(5, ssh.ended()).await.is_lost());
    assert!(!within(5, ssh.ping(Duration::from_secs(2))).await);
}

/// The host's count of open session channels, once it settles at `want`
/// (within 5 s), else what it last was.
async fn open_channels_settle(host: &Host, want: usize) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let open = host.seen.lock().unwrap().open_channels;
        if open == want || std::time::Instant::now() > deadline {
            return open;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// B720: a command the host refuses gives its channel back. The refusal
/// returned and left the channel open on the host, a session against its
/// MaxSessions until the connection ended (B612's class).
#[tokio::test]
async fn a_refused_command_gives_its_channel_back() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    assert!(ssh.run("refuse").await.is_err());
    assert_eq!(open_channels_settle(&host, 0).await, 0, "the channel was left open");
}

/// B720: a process dropped without a close — a pump given up, herdr's push
/// abandoned — closes its channel. Dropped, it only stopped being read.
#[tokio::test]
async fn a_dropped_process_closes_its_channel() {
    let host = Host::start(Bench::default()).await;
    let ssh = connect(&host).await;
    let process = ssh.exec("cat").await.expect("cat runs");
    assert_eq!(open_channels_settle(&host, 1).await, 1);
    drop(process);
    assert_eq!(open_channels_settle(&host, 0).await, 0, "the channel was left open");
}

/// B720: a read given up while its channel is still being opened (B243's
/// cancel, a timeout) closes the channel once the host's yes arrives. The
/// yes went to nobody and the channel stayed open on both sides.
#[tokio::test]
async fn an_open_given_up_midway_is_closed_when_it_lands() {
    let host = Host::start(Bench {
        open_delay: Some(std::time::Duration::from_millis(300)),
        ..Default::default()
    })
    .await;
    let ssh = connect(&host).await;
    let cancel = tokio_util::sync::CancellationToken::new();
    let given_up = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        given_up.cancel();
    });
    assert!(matches!(ssh.run_until("echo hi", &cancel).await, Ok(None)));
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    assert_eq!(open_channels_settle(&host, 0).await, 0, "the late channel was left open");
}
