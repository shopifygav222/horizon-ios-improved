//! The guest's end: reach the host from the code (directly, or through a
//! relay), agree on the key, then offer the session on a loopback port the
//! game's client connects to like to any server.
use crate::{
    Code, PathMode,
    crypto::{self, Role},
    direct,
    invite::{Invite, Parsed},
    resolve_relay,
    session::{self, Secure},
    wire::{self, ErrorCode, FromRelay, ToRelay},
};
use std::{
    fmt, io,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::channel,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub struct GuestConfig {
    /// The code as pasted: a full invite, or a relay's short code.
    pub code: String,
    /// The relay to use for short codes (the player's setting); a full
    /// invite names its own.
    pub relay: Option<String>,
    /// The game's protocol number and version name (a relay refuses a
    /// guest whose protocol differs from the host's).
    pub protocol: i32,
    pub game: String,
    pub path: PathMode,
    /// Endpoints of the code that are the host's public tunnel (a shared world's lease names
    /// them): dialled only when no other endpoint answered within [`TUNNEL_HEAD_START`], and
    /// over them the two ends still race for a direct stream. Empty: every endpoint is dialled
    /// at once, as for any code.
    pub tunnels: Vec<SocketAddr>,
    /// STUN servers for this end's public address, offered to the host for that race.
    pub stun: Vec<String>,
}

/// How long a code's own endpoints are dialled alone before the host's public tunnel is too:
/// one that works answers well within it, and is faster to play over.
pub const TUNNEL_HEAD_START: Duration = Duration::from_millis(600);

/// Why joining failed, in words for the player.
#[derive(Debug)]
pub enum JoinError {
    BadCode(crate::invite::CodeError),
    /// A short code without a relay to resolve it.
    NeedsRelay,
    /// None of the code's endpoints answered, and no relay to fall back to.
    NotReachable,
    Unreachable(String),
    UnknownCode,
    Ended,
    VersionMismatch {
        host: Option<String>,
        mine: String,
    },
    HostUnreachable,
    Full,
    RateLimited,
    WrongCode,
    Failed(String),
}
impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadCode(e) => write!(f, "{e}"),
            Self::NeedsRelay => write!(
                f,
                "This short code works through your friend's relay: add the relay's address under Join a friend, or ask for the full code."
            ),
            Self::NotReachable => write!(
                f,
                "Couldn't reach your friend's game. Ask them to read the note under their code: they may need UPnP on their router, a forwarded port, a relay or a VPN."
            ),
            Self::Unreachable(e) => write!(f, "Couldn't reach the relay ({e})."),
            Self::UnknownCode => write!(
                f,
                "No world uses this code. Check it, or ask your friend for a new one."
            ),
            Self::Ended => write!(
                f,
                "This invite has ended: your friend stopped sharing or left the world."
            ),
            Self::VersionMismatch {
                host: Some(v),
                mine,
            } => {
                write!(f, "Your friend plays Minecraft {v}; you have {mine}.")
            }
            Self::VersionMismatch { host: None, mine } => {
                write!(
                    f,
                    "Your friend plays another version of Minecraft than {mine}."
                )
            }
            Self::HostUnreachable => write!(f, "Your friend's game didn't answer."),
            Self::Full => write!(
                f,
                "Your friend's world is full, or the invite service is busy. Try again in a moment."
            ),
            Self::RateLimited => {
                write!(f, "Too many attempts. Wait a few minutes, then try again.")
            }
            Self::WrongCode => write!(f, "The code doesn't match. Check the last four characters."),
            Self::Failed(e) => write!(f, "Couldn't connect to your friend ({e})."),
        }
    }
}
impl std::error::Error for JoinError {}

/// A settled session, ready to carry the game's connection.
pub struct Tunnel {
    secure: Secure,
    /// Whether the stream goes straight to the host (else through the relay or the tunnel).
    pub direct: bool,
    /// Whether it goes through the host's public tunnel.
    pub tunneled: bool,
}

/// Join with what the player pasted: a full invite is dialled directly
/// (every endpoint at once), then through the relay it names, if any; a
/// short code goes through the configured relay. Blocks for a few seconds.
pub fn join(config: &GuestConfig) -> Result<Tunnel, JoinError> {
    match crate::invite::parse_detailed(&config.code).map_err(JoinError::BadCode)? {
        Parsed::Short(code) => {
            let relay = config.relay.as_deref().ok_or(JoinError::NeedsRelay)?;
            join_relay(config, relay, &code)
        }
        Parsed::Long(invite) => {
            // A tunnel is no direct path: a guest that never goes direct still takes it.
            let tunnels = invite
                .endpoints
                .iter()
                .any(|e| config.tunnels.contains(e));
            if (config.path != PathMode::RelayOnly || tunnels)
                && !invite.endpoints.is_empty()
                && let Some(t) = join_direct(&invite, config)?
            {
                return Ok(t);
            }
            match &invite.relay {
                Some(r) if config.path != PathMode::DirectOnly => {
                    join_relay(config, &r.address, &r.code)
                }
                _ => Err(JoinError::NotReachable),
            }
        }
    }
}

/// Dial every endpoint of the code (the host's public tunnel only after the others had
/// [`TUNNEL_HEAD_START`]); the first that answers as a Horizon host gets the handshake, the
/// others are dropped before it.
fn join_direct(invite: &Invite, config: &GuestConfig) -> Result<Option<Tunnel>, JoinError> {
    let (tx, rx) = channel::<(TcpStream, SocketAddr, bool)>();
    let tunnels: Vec<SocketAddr> = invite
        .endpoints
        .iter()
        .copied()
        .filter(|e| config.tunnels.contains(e))
        .collect();
    let race = config.path != PathMode::RelayOnly;
    // The tunnel waits only while something else is being dialled.
    let head_start = if !tunnels.is_empty() && race && invite.endpoints.len() > tunnels.len() {
        TUNNEL_HEAD_START
    } else {
        Duration::ZERO
    };
    let deadline = Instant::now() + Duration::from_secs(5) + head_start;
    let mut session = [0u8; 8];
    crate::fill_random(&mut session).map_err(|e| JoinError::Failed(e.to_string()))?;
    let session = u64::from_le_bytes(session);
    let won = Arc::new(AtomicBool::new(false));
    // This end's public address, looked up meanwhile, for a direct race over the tunnel.
    let stun = (!tunnels.is_empty() && race && !config.stun.is_empty())
        .then(|| {
            let servers = config.stun.clone();
            std::thread::Builder::new()
                .name("horizon-friends-stun".into())
                .spawn(move || crate::stun::public_ip(&servers, Duration::from_secs(2)))
                .ok()
        })
        .flatten();
    let started = Instant::now();
    for &endpoint in &invite.endpoints {
        let tunnel = tunnels.contains(&endpoint);
        if !tunnel && !race {
            continue;
        }
        let (tx, won) = (tx.clone(), won.clone());
        let _ = std::thread::Builder::new()
            .name("horizon-friends-dial".into())
            .spawn(move || {
                if tunnel {
                    while started.elapsed() < head_start && !won.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    if won.load(Ordering::Relaxed) {
                        return;
                    }
                }
                let Ok(mut s) = TcpStream::connect_timeout(&endpoint, Duration::from_secs(4))
                else {
                    return;
                };
                let hello = if tunnel {
                    direct::tunnel_hello(&mut s, session)
                } else {
                    direct::code_hello(&mut s, session)
                };
                if hello.is_ok() {
                    let _ = tx.send((s, endpoint, tunnel));
                }
            });
    }
    drop(tx);
    let left = deadline.saturating_duration_since(Instant::now());
    let received = rx.recv_timeout(left);
    won.store(true, Ordering::Relaxed);
    let Ok((mut stream, endpoint, tunnel)) = received else {
        return Ok(None);
    };
    let context = if tunnel { "tunnel" } else { "direct" };
    let key = crypto::exchange(&mut stream, &invite.secret_text(), context, session)
        .map_err(|e| JoinError::Failed(e.to_string()))?;
    let secure = session::secure(stream, &key, Role::Guest).map_err(|e| {
        if crypto::is_wrong_code(&e) {
            JoinError::WrongCode
        } else {
            JoinError::Failed(e.to_string())
        }
    })?;
    if !tunnel {
        return Ok(Some(Tunnel {
            secure,
            direct: true,
            tunneled: false,
        }));
    }
    // Through the tunnel: race for a direct stream over it, from a port of this end's own.
    let (listeners, port) = if race {
        match direct::listen(0) {
            Ok((l, p)) => (l, Some(p)),
            Err(_) => (Vec::new(), None),
        }
    } else {
        (Vec::new(), None)
    };
    let public = stun.and_then(|t| t.join().ok()).flatten();
    let candidates = port.map_or_else(Vec::new, |p| {
        direct::candidates(p, Some(endpoint), public.map(|ip| SocketAddr::new(ip, p)))
    });
    let settled = session::settle(
        secure,
        &key,
        Role::Guest,
        config.path,
        session,
        port,
        candidates,
        Box::new(move |tx, deadline, stop| direct::accept(listeners, session, deadline, stop, tx)),
    )
    .map_err(|e| JoinError::Failed(e.to_string()))?;
    Ok(Some(Tunnel {
        secure: settled.secure,
        direct: settled.direct,
        tunneled: !settled.direct,
    }))
}

/// Through a relay with its short code (its own direct race, then the
/// relay's byte stream).
fn join_relay(config: &GuestConfig, relay_url: &str, code: &Code) -> Result<Tunnel, JoinError> {
    let relays = resolve_relay(relay_url).map_err(|e| JoinError::Unreachable(e.to_string()))?;
    let direct_allowed = config.path != PathMode::RelayOnly;
    let (listeners, port) = if direct_allowed {
        match direct::listen(0) {
            Ok((l, p)) => (l, Some(p)),
            Err(_) => (Vec::new(), None),
        }
    } else {
        (Vec::new(), None)
    };
    let mut last = io::Error::other("no address");
    let mut stream = None;
    for relay in &relays {
        match direct::connect_from(port, *relay, Duration::from_secs(5)) {
            Ok(s) => {
                stream = Some((s, *relay));
                break;
            }
            Err(e) => last = e,
        }
    }
    let (mut stream, relay) = stream.ok_or_else(|| JoinError::Unreachable(last.to_string()))?;
    let failed = |e: io::Error| JoinError::Failed(e.to_string());
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(failed)?;
    let _ = stream.set_nodelay(true);
    wire::send(
        &mut stream,
        &ToRelay::Join {
            v: wire::VERSION,
            nameplate: code.nameplate.clone(),
            protocol: config.protocol,
            game: config.game.clone(),
        },
    )
    .map_err(failed)?;
    let (session, observed) = match wire::recv::<_, FromRelay>(&mut stream).map_err(failed)? {
        FromRelay::Bridged { session, observed } => (session, observed),
        FromRelay::Error { code, game } => {
            return Err(match code {
                ErrorCode::UnknownCode => JoinError::UnknownCode,
                ErrorCode::Ended => JoinError::Ended,
                ErrorCode::VersionMismatch => JoinError::VersionMismatch {
                    host: game,
                    mine: config.game.clone(),
                },
                ErrorCode::HostUnreachable => JoinError::HostUnreachable,
                ErrorCode::Full => JoinError::Full,
                ErrorCode::RateLimited => JoinError::RateLimited,
                ErrorCode::BadRequest => JoinError::Failed("the relay refused the request".into()),
            });
        }
        other => return Err(JoinError::Failed(format!("unexpected answer {other:?}"))),
    };
    let key = crypto::exchange(&mut stream, &code.secret, &code.nameplate, session)
        .map_err(|_| JoinError::HostUnreachable)?;
    let relayed = session::secure(stream, &key, Role::Guest).map_err(|e| {
        if crypto::is_wrong_code(&e) {
            JoinError::WrongCode
        } else {
            JoinError::HostUnreachable
        }
    })?;
    let observed = observed.and_then(|o| o.parse::<SocketAddr>().ok());
    let candidates = port.map_or_else(Vec::new, |p| direct::candidates(p, Some(relay), observed));
    let settled = session::settle(
        relayed,
        &key,
        Role::Guest,
        config.path,
        session,
        port,
        candidates,
        Box::new(move |tx, deadline, stop| direct::accept(listeners, session, deadline, stop, tx)),
    )
    .map_err(|e| JoinError::Failed(e.to_string()))?;
    Ok(Tunnel {
        secure: settled.secure,
        direct: settled.direct,
        tunneled: false,
    })
}

impl Tunnel {
    /// Offer the session on `127.0.0.1:port` for one connection (the game's
    /// client); the returned thread ends when that connection does.
    pub fn serve_loopback(self) -> io::Result<(u16, JoinHandle<()>)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let thread = std::thread::Builder::new()
            .name("horizon-friends-guest".into())
            .spawn(move || {
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                let plain = loop {
                    match listener.accept() {
                        Ok((s, from)) if from.ip().is_loopback() => break Some(s),
                        Ok(_) => {}
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() > deadline {
                                break None;
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break None,
                    }
                };
                drop(listener);
                let (reader, writer) = self.secure;
                let Some(plain) = plain else {
                    let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
                    return;
                };
                let _ = plain.set_nonblocking(false);
                if let Ok(t) = crate::pump(plain, reader, writer) {
                    let _ = t.join();
                }
            })?;
        Ok((port, thread))
    }
}
