//! Hosts in front of an echo server standing in for the integrated server,
//! guests, and (for the relay cases) a self-hosted relay, all on loopback
//! in one process. Port mapping and STUN are off here: they have their own
//! tests against mock routers and servers in horizon-friends.
use horizon_friends::{
    Code, Host, HostConfig, HostEvent, JoinError, PathMode,
    guest::{self, GuestConfig},
    invite::{self, Invite, Parsed},
    reach,
};
use horizon_relay::{Config, Relay, limits::Rates};
use std::{
    cell::RefCell,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const PROTOCOL: i32 = 777;

/// Echoes every connection back until it closes.
fn echo_server() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            std::thread::spawn(move || {
                let mut out = s.try_clone().unwrap();
                let _ = std::io::copy(&mut s, &mut out);
            });
        }
    });
    a
}

fn relay(config: Config) -> (Relay, String) {
    let (relay, address, _) = Relay::spawn_local(config).unwrap();
    (relay, address.to_string())
}

fn generous() -> Config {
    Config {
        rates: Rates {
            join_per_min: 600.0,
            join_burst: 100.0,
            ..Rates::default()
        },
        ..Config::default()
    }
}

fn start(relay: Option<&str>, path: PathMode) -> Host {
    Host::start(HostConfig {
        lan: echo_server(),
        protocol: PROTOCOL,
        game: "26.3".into(),
        relay: relay.map(str::to_owned),
        path,
        port: 0,
        reach: reach::Options {
            map_port: false,
            stun: Vec::new(),
            loopback: true,
        },
        tunnel: None,
    })
    .unwrap()
}

thread_local! {
    /// Events polled but not yet waited for.
    static BACKLOG: RefCell<Vec<HostEvent>> = const { RefCell::new(Vec::new()) };
}

/// The first event (in order) that `f` picks; the others stay queued.
fn wait_event<T>(host: &Host, mut f: impl FnMut(&HostEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let found = BACKLOG.with_borrow_mut(|backlog| {
            backlog.extend(host.poll());
            let i = backlog.iter().position(|e| f(e).is_some())?;
            f(&backlog.remove(i))
        });
        if let Some(t) = found {
            return t;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no such host event");
}

/// The invite once ready (and, with a relay, once it holds the short code).
fn invite_of(host: &Host, with_relay: bool) -> (String, Option<Code>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let s = host.state();
        if let Some(code) = s.invite
            && (!with_relay || s.short.is_some())
        {
            return (code, s.short);
        }
        assert!(Instant::now() < deadline, "no code");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn join(code: &str, relay: Option<&str>, path: PathMode) -> Result<guest::Tunnel, JoinError> {
    join_with(code, relay, Vec::new(), path)
}

fn join_with(
    code: &str,
    relay: Option<&str>,
    tunnels: Vec<SocketAddr>,
    path: PathMode,
) -> Result<guest::Tunnel, JoinError> {
    guest::join(&GuestConfig {
        code: code.to_owned(),
        relay: relay.map(str::to_owned),
        protocol: PROTOCOL,
        game: "26.3".into(),
        path,
        tunnels,
        stun: Vec::new(),
    })
}

/// A guest that knows which of the code's endpoints are the host's public tunnel.
fn join_via(
    code: &str,
    tunnels: Vec<SocketAddr>,
    path: PathMode,
) -> Result<guest::Tunnel, JoinError> {
    join_with(code, None, tunnels, path)
}

/// A stand-in for the public tunnel (bore's server and the host's end of it): a loopback port
/// that carries every connection to the host's direct port, counted.
fn tunneled_host(path: PathMode) -> (Host, Arc<AtomicUsize>) {
    let carried = Arc::new(AtomicUsize::new(0));
    let count = carried.clone();
    let open: horizon_friends::host::Tunnel = Arc::new(move |port: u16| {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let address = listener.local_addr().ok()?;
        let count = count.clone();
        std::thread::spawn(move || {
            for visitor in listener.incoming() {
                let Ok(visitor) = visitor else { continue };
                count.fetch_add(1, Ordering::SeqCst);
                let Ok(host) = TcpStream::connect(("127.0.0.1", port)) else {
                    continue;
                };
                for (mut from, mut to) in [
                    (visitor.try_clone().unwrap(), host.try_clone().unwrap()),
                    (host, visitor),
                ] {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from, &mut to);
                        let _ = to.shutdown(std::net::Shutdown::Both);
                        let _ = from.shutdown(std::net::Shutdown::Both);
                    });
                }
            }
        });
        Some((address, Box::new(()) as Box<dyn std::any::Any + Send>))
    });
    let host = Host::start(HostConfig {
        lan: echo_server(),
        protocol: PROTOCOL,
        game: "26.3".into(),
        relay: None,
        path,
        port: 0,
        reach: reach::Options {
            map_port: false,
            stun: Vec::new(),
            loopback: true,
        },
        tunnel: Some(open),
    })
    .unwrap();
    (host, carried)
}

fn tunnel_of(host: &Host) -> SocketAddr {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(t) = host.state().tunnel {
            return t;
        }
        assert!(Instant::now() < deadline, "no tunnel");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// How the host says the next session is carried: (direct, through the tunnel).
fn carried_as(host: &Host) -> (bool, bool) {
    wait_event(host, |e| match e {
        HostEvent::Connected { direct, tunnel, .. } => Some((*direct, *tunnel)),
        _ => None,
    })
}

#[test]
fn a_direct_endpoint_wins_over_the_hosts_public_tunnel() {
    let (host, carried) = tunneled_host(PathMode::Auto);
    let (code, _) = invite_of(&host, false);
    let tunnel = tunnel_of(&host);
    // The tunnel comes first in the code, the host's own addresses after it.
    let invite = long(&code);
    assert_eq!(invite.endpoints[0], tunnel);
    assert!(invite.endpoints[1..].iter().any(|e| e.ip().is_loopback()));
    let joined = join_via(&code, vec![tunnel], PathMode::Auto).unwrap();
    assert!(joined.direct && !joined.tunneled);
    assert_eq!(carried_as(&host), (true, false));
    round_trip(joined, &payload(300_000));
    // The tunnel was never dialled: the direct endpoint answered within the head start.
    std::thread::sleep(guest::TUNNEL_HEAD_START + Duration::from_millis(300));
    assert_eq!(carried.load(Ordering::SeqCst), 0);
    // A guest that doesn't know which endpoint is the tunnel (an older game) still joins.
    let joined = join_via(&code, Vec::new(), PathMode::Auto).unwrap();
    assert!(joined.direct);
    round_trip(joined, b"older guest");
}

#[test]
fn through_the_public_tunnel_the_two_ends_still_connect_directly() {
    let (host, carried) = tunneled_host(PathMode::Auto);
    let (code, _) = invite_of(&host, false);
    let tunnel = tunnel_of(&host);
    // Only the tunnel answers from where this guest is.
    let mut invite = long(&code);
    invite.endpoints = vec![tunnel];
    let started = Instant::now();
    let joined = join_via(&invite.encode(), vec![tunnel], PathMode::Auto).unwrap();
    // The two found a direct stream over it, and the game goes there.
    assert!(joined.direct && !joined.tunneled);
    assert_eq!(carried_as(&host), (true, false));
    assert_eq!(carried.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < Duration::from_secs(8), "{:?}", started.elapsed());
    round_trip(joined, &payload(1 << 20));
}

#[test]
fn a_guest_that_never_goes_direct_plays_through_the_public_tunnel() {
    let (host, carried) = tunneled_host(PathMode::Auto);
    let (code, _) = invite_of(&host, false);
    let tunnel = tunnel_of(&host);
    let joined = join_via(&code, vec![tunnel], PathMode::RelayOnly).unwrap();
    assert!(!joined.direct && joined.tunneled);
    assert_eq!(carried_as(&host), (false, true));
    assert_eq!(carried.load(Ordering::SeqCst), 1);
    round_trip(joined, &payload(1 << 20));
    // Without the tunnel in the code, it has nowhere to go.
    let mut invite = long(&code);
    invite.endpoints.retain(|e| *e != tunnel);
    assert!(matches!(
        join_via(&invite.encode(), vec![tunnel], PathMode::RelayOnly),
        Err(JoinError::NotReachable)
    ));
}

#[test]
fn a_wrong_secret_through_the_tunnel_is_refused() {
    let (host, _) = tunneled_host(PathMode::Auto);
    let (code, _) = invite_of(&host, false);
    let tunnel = tunnel_of(&host);
    let mut invite = long(&code);
    invite.endpoints = vec![tunnel];
    let mut other = long(&invite_of(&start(None, PathMode::Auto), false).0);
    other.endpoints = vec![tunnel];
    assert!(join_via(&other.encode(), vec![tunnel], PathMode::Auto).is_err());
    assert!(wait_event(&host, |e| match e {
        HostEvent::WrongCode { count } => Some(*count >= 1),
        _ => None,
    }));
    // The right code still joins after it.
    let joined = join_via(&invite.encode(), vec![tunnel], PathMode::Auto).unwrap();
    round_trip(joined, b"after a wrong code");
}

#[test]
fn tunnel_marks_ride_along_in_an_address_and_come_back_off_it() {
    let tunnels: Vec<SocketAddr> = vec![
        "203.0.113.9:4000".parse().unwrap(),
        "[2001:db8::1]:4001".parse().unwrap(),
    ];
    let marked = horizon_friends::with_tunnels("0B33K-Q405Q", &tunnels);
    assert_eq!(
        horizon_friends::split_tunnels(&marked),
        ("0B33K-Q405Q", tunnels)
    );
    assert_eq!(horizon_friends::with_tunnels("0B33K-Q405Q", &[]), "0B33K-Q405Q");
    assert_eq!(
        horizon_friends::split_tunnels("0B33K-Q405Q"),
        ("0B33K-Q405Q", Vec::new())
    );
    assert_eq!(
        horizon_friends::split_tunnels("0B33K~nonsense,203.0.113.9:4000"),
        ("0B33K", vec![tunnels[0]])
    );
}

/// What the game's client does: connect to the loopback port, send, read
/// the echo back.
fn round_trip(tunnel: guest::Tunnel, payload: &[u8]) {
    let (port, done) = tunnel.serve_loopback().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut w = s.try_clone().unwrap();
    let send = payload.to_vec();
    let writer = std::thread::spawn(move || w.write_all(&send).unwrap());
    let mut got = vec![0u8; payload.len()];
    s.read_exact(&mut got).unwrap();
    writer.join().unwrap();
    assert!(got == payload, "echo differs");
    drop(s);
    done.join().unwrap();
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn long(code: &str) -> Invite {
    match invite::parse(code) {
        Some(Parsed::Long(i)) => i,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_code_alone_connects_without_any_server() {
    let host = start(None, PathMode::Auto);
    let (code, short) = invite_of(&host, false);
    assert!(short.is_none());
    let invite = long(&code);
    assert!(invite.relay.is_none());
    assert!(
        invite.endpoints.iter().any(|e| e.ip().is_loopback()),
        "{invite:?}"
    );
    let tunnel = join(&code, None, PathMode::Auto).unwrap();
    assert!(tunnel.direct);
    round_trip(tunnel, &payload(1 << 20));
    assert!(wait_event(&host, |e| match e {
        HostEvent::Connected { direct, .. } => Some(*direct),
        _ => None,
    }));
    wait_event(&host, |e| {
        matches!(e, HostEvent::Disconnected { .. }).then_some(())
    });
    assert_eq!(host.sessions(), 0);
    // Two guests at once.
    let a = join(&code, None, PathMode::Auto).unwrap();
    let b = join(&code, None, PathMode::Auto).unwrap();
    let ta = std::thread::spawn(move || round_trip(a, &payload(50_000)));
    round_trip(b, &payload(70_000));
    ta.join().unwrap();
    // Without a relay, a relay-only guest has nowhere to go.
    assert!(matches!(
        join(&code, None, PathMode::RelayOnly),
        Err(JoinError::NotReachable)
    ));
}

#[test]
fn a_short_code_through_a_self_hosted_relay() {
    let (relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (code, short) = invite_of(&host, true);
    let short = short.unwrap().to_string();
    // The full code names the relay too.
    assert_eq!(long(&code).relay.unwrap().address, address);
    // A short code needs the relay's address.
    assert!(matches!(
        join(&short, None, PathMode::Auto),
        Err(JoinError::NeedsRelay)
    ));
    let tunnel = join(&short, Some(&address), PathMode::RelayOnly).unwrap();
    assert!(!tunnel.direct);
    let data = payload(1 << 20);
    round_trip(tunnel, &data);
    let relayed = relay.counts().relayed_bytes;
    assert!(relayed > 2 * data.len() as u64, "{relayed}");
    // The relay's own direct race (both ends on loopback): direct.
    let tunnel = join(&short, Some(&address), PathMode::Auto).unwrap();
    assert!(tunnel.direct);
    round_trip(tunnel, b"raced");
}

#[test]
fn unreachable_endpoints_fall_back_to_the_relay_in_the_code() {
    let (relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (code, _) = invite_of(&host, true);
    // The same invite with an endpoint nobody answers on.
    let mut invite = long(&code);
    let dead = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    invite.endpoints = vec![dead];
    let started = Instant::now();
    assert!(matches!(
        join(&invite.encode(), None, PathMode::DirectOnly),
        Err(JoinError::NotReachable)
    ));
    assert!(started.elapsed() < Duration::from_secs(6));
    let tunnel = join(&invite.encode(), None, PathMode::Auto).unwrap();
    round_trip(tunnel, b"fallback");
    assert!(relay.counts().relayed_bytes > 0);
    // Without a relay in the code: the host's advice is the way out.
    invite.relay = None;
    assert!(matches!(
        join(&invite.encode(), None, PathMode::Auto),
        Err(JoinError::NotReachable)
    ));
}

#[test]
fn wrong_secrets_fail() {
    let (_relay, address) = relay(generous());
    let host = start(Some(&address), PathMode::Auto);
    let (code, short) = invite_of(&host, true);
    // A full code with another secret (re-encoded, so the checksum passes).
    let mut forged = long(&code);
    forged.secret[0] ^= 1;
    assert!(matches!(
        join(&forged.encode(), None, PathMode::DirectOnly),
        Err(JoinError::WrongCode)
    ));
    assert_eq!(
        wait_event(&host, |e| match e {
            HostEvent::WrongCode { count } => Some(*count),
            _ => None,
        }),
        1
    );
    // Short codes: five wrong secrets retire it for a new one.
    let short = short.unwrap();
    let wrong = Code {
        nameplate: short.nameplate.clone(),
        secret: if short.secret == "0000" {
            "0001".into()
        } else {
            "0000".into()
        },
    };
    for _ in 0..horizon_friends::host::MAX_WRONG {
        let e = join(&wrong.to_string(), Some(&address), PathMode::Auto)
            .err()
            .unwrap();
        assert!(matches!(e, JoinError::WrongCode), "{e}");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let fresh = loop {
        if let Some(s) = host.state().short
            && s.nameplate != short.nameplate
        {
            break s;
        }
        assert!(Instant::now() < deadline, "no new short code");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(matches!(
        join(&short.to_string(), Some(&address), PathMode::Auto),
        Err(JoinError::Ended)
    ));
    round_trip(
        join(&fresh.to_string(), Some(&address), PathMode::Auto).unwrap(),
        b"fresh",
    );
    // The full code follows the new short code; its secret stays.
    let (code2, _) = invite_of(&host, true);
    assert_eq!(long(&code2).relay.unwrap().code, fresh);
    assert_eq!(long(&code2).secret, long(&code).secret);
}

#[test]
fn bad_codes_unknown_codes_versions_and_ended_invites() {
    assert!(matches!(
        join("hello", None, PathMode::Auto),
        Err(JoinError::BadCode(_))
    ));
    let (relay, address) = relay(generous());
    let mut host = start(Some(&address), PathMode::RelayOnly);
    let (_, short) = invite_of(&host, true);
    let short = short.unwrap();
    let other = guest::join(&GuestConfig {
        code: short.to_string(),
        relay: Some(address.clone()),
        protocol: PROTOCOL - 1,
        game: "26.2".into(),
        path: PathMode::Auto,
        tunnels: Vec::new(),
        stun: Vec::new(),
    });
    match other {
        Err(e @ JoinError::VersionMismatch { .. }) => {
            assert_eq!(
                e.to_string(),
                "Your friend plays Minecraft 26.3; you have 26.2."
            )
        }
        other => panic!("{:?}", other.err()),
    }
    host.stop();
    assert!(matches!(
        join(&short.to_string(), Some(&address), PathMode::Auto),
        Err(JoinError::Ended)
    ));
    // A nameplate nobody holds; five of them block this address for a while.
    let free = (0..)
        .map(|i| horizon_friends::code::nameplate(i * 7919))
        .find(|n| *n != short.nameplate)
        .unwrap();
    let unknown = format!("{free}0000");
    for _ in 0..5 {
        assert!(matches!(
            join(&unknown, Some(&address), PathMode::Auto),
            Err(JoinError::UnknownCode)
        ));
    }
    assert!(matches!(
        join(&short.to_string(), Some(&address), PathMode::Auto),
        Err(JoinError::RateLimited)
    ));
    let counts = relay.counts();
    assert_eq!(counts.unknown, 5);
    assert!(counts.rate_limited >= 1);
    assert_eq!(counts.hosts, 0);
}

#[test]
fn a_host_that_never_attaches_is_unreachable() {
    use horizon_friends::wire::{self, FromRelay, ToRelay};
    let (_relay, address) = relay(Config {
        attach_wait: Duration::from_millis(500),
        ..generous()
    });
    let mut control = TcpStream::connect(&address).unwrap();
    wire::send(
        &mut control,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: PROTOCOL,
            game: "26.3".into(),
        },
    )
    .unwrap();
    let FromRelay::Registered {
        nameplate, token, ..
    } = wire::recv(&mut control).unwrap()
    else {
        panic!()
    };
    let code = format!("{nameplate}0000");
    assert!(matches!(
        join(&code, Some(&address), PathMode::Auto),
        Err(JoinError::HostUnreachable)
    ));
    // The relay announced the session; attaching late fails.
    let FromRelay::Join { session } = wire::recv(&mut control).unwrap() else {
        panic!()
    };
    let mut late = TcpStream::connect(&address).unwrap();
    wire::send(
        &mut late,
        &ToRelay::Attach {
            v: wire::VERSION,
            session,
            token,
        },
    )
    .unwrap();
    assert!(matches!(
        wire::recv::<_, FromRelay>(&mut late).unwrap(),
        FromRelay::Error { .. }
    ));
}

#[test]
fn relayed_bandwidth_is_capped() {
    let rate = 200_000.0;
    let (_relay, address) = relay(Config {
        session_rate: rate,
        ..generous()
    });
    let host = start(Some(&address), PathMode::RelayOnly);
    let (_, short) = invite_of(&host, true);
    let tunnel = join(
        &short.unwrap().to_string(),
        Some(&address),
        PathMode::RelayOnly,
    )
    .unwrap();
    let started = Instant::now();
    round_trip(tunnel, &payload(600_000));
    // The first 100 kB pass at once, the rest at 200 kB/s.
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn health_and_whoami_answer_http() {
    let (_relay, address) = relay(Config::default());
    let get = |path: &str| {
        let mut s = TcpStream::connect(&address).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: relay\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };
    let health = get("/health");
    assert!(health.starts_with("HTTP/1.1 200 OK\r\n"), "{health}");
    assert!(health.ends_with("{\"ok\":true,\"hosts\":0,\"bridges\":0,\"registrations\":0,\"joins\":0,\"unknown\":0,\"rate_limited\":0,\"relayed_bytes\":0}"));
    assert!(get("/whoami").contains("\"address\":\"127.0.0.1:"));
    assert!(get("/").starts_with("HTTP/1.1 404"));
}

/// A first line trickled a byte at a time still has to arrive within
/// `first_line` in all; past `max_connections`, connections close unanswered.
#[test]
fn slow_first_lines_and_too_many_connections_are_closed() {
    let (_relay, address) = relay(Config {
        first_line: Duration::from_secs(1),
        max_connections: 2,
        ..Config::default()
    });
    let started = Instant::now();
    let mut slow = TcpStream::connect(&address).unwrap();
    let held = TcpStream::connect(&address).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let mut over = TcpStream::connect(&address).unwrap();
    over.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(over.read(&mut [0u8; 1]).unwrap_or(0), 0);
    while slow.write_all(b"{").is_ok() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let closed = started.elapsed();
    assert!(
        closed < Duration::from_secs(3),
        "the slow first line held on for {closed:?}"
    );
    drop(held);
}
