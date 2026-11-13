//! A way in for a host no network lets friends reach: the host connects *out* to a public
//! tunnel server (bore, <https://github.com/ekzhang/bore>; `bore.pub` is its free public one),
//! which gives it a port on the server's address and passes every connection to that port back
//! down connections the host itself opens.
//!
//! The tunnel carries the game's own end-to-end encrypted stream (the invite code's secret never
//! leaves the code), so the server sees addresses and sizes, nothing else. Its speed and whether
//! it is up are the server's: the game uses it only when no direct way exists, and says so.
//!
//! The protocol (bore 0.5): JSON messages ended by a zero byte on a control connection to port
//! 7835. The client says `{"Hello":0}` and hears `{"Hello":<port>}`; for each visitor the server
//! says `{"Connection":"<id>"}`, and the client opens a new connection, says `{"Accept":"<id>"}`
//! and from then on that connection is the visitor's bytes.
use crate::{Error, Result};
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const CONTROL_PORT: u16 = 7835;
pub const DEFAULT_SERVER: &str = "bore.pub";
/// The longest message the protocol allows.
const MAX_FRAME: usize = 256;
const CONNECT: Duration = Duration::from_secs(5);
/// The server says something (a heartbeat at least) far more often than this.
const SILENCE: Duration = Duration::from_secs(45);

/// An open tunnel. Dropping it closes it.
pub struct Tunnel {
    address: SocketAddr,
    control: TcpStream,
    alive: Arc<AtomicBool>,
}

fn failed(what: impl std::fmt::Display) -> Error {
    Error::Store(format!("the public tunnel: {what}"))
}

fn send(stream: &mut TcpStream, message: &serde_json::Value) -> io::Result<()> {
    let mut frame = serde_json::to_vec(message)?;
    frame.push(0);
    stream.write_all(&frame)
}

/// The next message, or None when the server closed the connection.
fn receive(reader: &mut BufReader<TcpStream>) -> io::Result<Option<serde_json::Value>> {
    let mut frame = Vec::new();
    let limit = MAX_FRAME as u64 + 1;
    let n = io::Read::take(&mut *reader, limit).read_until(0, &mut frame)?;
    if n == 0 {
        return Ok(None);
    }
    if frame.pop() != Some(0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a message without its end",
        ));
    }
    serde_json::from_slice(&frame)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Carries bytes both ways between two connections until either ends, then closes both.
fn pump(a: TcpStream, b: TcpStream) -> io::Result<()> {
    let (mut a_read, mut a_write) = (a.try_clone()?, a);
    let (mut b_read, mut b_write) = (b.try_clone()?, b);
    for s in [&a_write, &b_write] {
        s.set_read_timeout(None)?;
        let _ = s.set_nodelay(true);
    }
    let forward = std::thread::Builder::new()
        .name("shared-worlds-tunnel-in".into())
        .spawn(move || {
            let _ = io::copy(&mut a_read, &mut b_write);
            let _ = b_write.shutdown(Shutdown::Both);
            let _ = a_read.shutdown(Shutdown::Both);
        })?;
    let _ = io::copy(&mut b_read, &mut a_write);
    let _ = a_write.shutdown(Shutdown::Both);
    let _ = b_read.shutdown(Shutdown::Both);
    let _ = forward.join();
    Ok(())
}

/// One visitor: a new connection to the server that accepts it, joined to the local port.
fn accept(server: SocketAddr, local: SocketAddr, id: &str) -> io::Result<()> {
    let mut remote = TcpStream::connect_timeout(&server, CONNECT)?;
    send(&mut remote, &serde_json::json!({ "Accept": id }))?;
    let game = TcpStream::connect_timeout(&local, CONNECT)?;
    pump(remote, game)
}

impl Tunnel {
    /// Opens a tunnel on `server` (a host name; its control port is [`CONTROL_PORT`]) to the
    /// port `local` on this machine.
    pub fn open(server: &str, local: SocketAddr) -> Result<Self> {
        let mut addresses: Vec<SocketAddr> = (server, CONTROL_PORT)
            .to_socket_addrs()
            .map_err(|e| failed(format!("{server} wasn't found: {e}")))?
            .collect();
        // IPv4 first: every guest can dial it.
        addresses.sort_by_key(|a| a.is_ipv6());
        let mut last = failed(format!("{server} has no address"));
        for address in addresses {
            match Self::open_at(address, local) {
                Ok(tunnel) => return Ok(tunnel),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// As [`Self::open`], with the server's control address given.
    pub fn open_at(server: SocketAddr, local: SocketAddr) -> Result<Self> {
        let mut control = TcpStream::connect_timeout(&server, CONNECT).map_err(failed)?;
        control.set_read_timeout(Some(CONNECT)).map_err(failed)?;
        let _ = control.set_nodelay(true);
        send(&mut control, &serde_json::json!({ "Hello": 0 })).map_err(failed)?;
        let mut reader = BufReader::new(control.try_clone().map_err(failed)?);
        let hello = receive(&mut reader).map_err(failed)?;
        let port = match hello
            .as_ref()
            .and_then(|m| m.as_object())
            .and_then(|m| m.iter().next())
        {
            Some((kind, port)) if kind == "Hello" => port
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
                .filter(|p| *p != 0)
                .ok_or_else(|| failed("it gave no port"))?,
            Some((kind, message)) if kind == "Error" => return Err(failed(message)),
            Some((kind, _)) if kind == "Challenge" => return Err(failed("it asks for a password")),
            _ => return Err(failed("it didn't answer as a tunnel server")),
        };
        control.set_read_timeout(Some(SILENCE)).map_err(failed)?;
        let alive = Arc::new(AtomicBool::new(true));
        let running = alive.clone();
        std::thread::Builder::new()
            .name("shared-worlds-tunnel".into())
            .spawn(move || {
                while running.load(Ordering::SeqCst) {
                    let Ok(Some(message)) = receive(&mut reader) else {
                        break;
                    };
                    let Some(id) = message.get("Connection").and_then(|id| id.as_str()) else {
                        // A heartbeat, or something newer than this client: nothing to do.
                        continue;
                    };
                    let id = id.to_owned();
                    let spawned = std::thread::Builder::new()
                        .name("shared-worlds-tunnel-guest".into())
                        .spawn(move || {
                            if let Err(e) = accept(server, local, &id) {
                                log::warn!("Shared worlds: a tunnelled connection ended: {e}");
                            }
                        });
                    if spawned.is_err() {
                        break;
                    }
                }
                running.store(false, Ordering::SeqCst);
            })
            .map_err(failed)?;
        Ok(Self {
            address: SocketAddr::new(server.ip(), port),
            control,
            alive,
        })
    }

    /// Where friends reach the host: the server's address and the port it gave.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// False once the server closed the tunnel or went quiet: open a new one.
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.control.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpListener, time::Instant};

    /// A tunnel server as bore's: one control port; a public port per client; each visitor is
    /// announced on the control connection and joined to the connection that accepts it.
    fn server(answer: &'static str) -> SocketAddr {
        let control = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = control.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut client, _) = control.accept().unwrap();
            let mut reader = BufReader::new(client.try_clone().unwrap());
            assert_eq!(
                receive(&mut reader).unwrap().unwrap(),
                serde_json::json!({ "Hello": 0 })
            );
            let public = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = public.local_addr().unwrap().port();
            match answer {
                "hello" => send(&mut client, &serde_json::json!({ "Hello": port })).unwrap(),
                "error" => {
                    return send(&mut client, &serde_json::json!({ "Error": "port in use" }))
                        .unwrap();
                }
                _ => return send(&mut client, &serde_json::json!({ "Challenge": "abc" })).unwrap(),
            }
            send(&mut client, &serde_json::json!("Heartbeat")).unwrap();
            for (n, visitor) in public.incoming().take(2).enumerate() {
                let visitor = visitor.unwrap();
                send(
                    &mut client,
                    &serde_json::json!({ "Connection": format!("id-{n}") }),
                )
                .unwrap();
                let (accepting, _) = control.accept().unwrap();
                let mut reader = BufReader::new(accepting.try_clone().unwrap());
                assert_eq!(
                    receive(&mut reader).unwrap().unwrap(),
                    serde_json::json!({ "Accept": format!("id-{n}") })
                );
                std::thread::spawn(move || pump(visitor, accepting));
            }
            // The server goes away: the control connection closes.
        });
        address
    }

    /// A local "game": echoes what it is sent, upper-cased.
    fn game() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 64];
                    while let Ok(n) = stream.read(&mut buffer) {
                        if n == 0 || stream.write_all(&buffer[..n].to_ascii_uppercase()).is_err() {
                            break;
                        }
                    }
                });
            }
        });
        address
    }

    #[test]
    fn visitors_reach_the_local_port_through_the_tunnel() {
        let tunnel = Tunnel::open_at(server("hello"), game()).unwrap();
        assert!(tunnel.alive());
        assert!(tunnel.address().ip().is_loopback());
        for word in [&b"first visitor"[..], b"second"] {
            let mut visitor = TcpStream::connect(tunnel.address()).unwrap();
            visitor
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            visitor.write_all(word).unwrap();
            let mut answer = vec![0u8; word.len()];
            visitor.read_exact(&mut answer).unwrap();
            assert_eq!(answer, word.to_ascii_uppercase());
        }
        // The server left after two visitors: the tunnel notices.
        let started = Instant::now();
        while tunnel.alive() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!tunnel.alive());
    }

    #[test]
    fn a_server_that_refuses_says_why() {
        let error = Tunnel::open_at(server("error"), game())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("port in use"), "{error}");
        let error = Tunnel::open_at(server("challenge"), game())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("password"), "{error}");
        // Nothing listens there.
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(Tunnel::open_at(closed, game()).is_err());
    }

    #[test]
    fn an_overlong_or_unfinished_message_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            client.write_all(&[b'x'; MAX_FRAME + 50]).unwrap();
        });
        let stream = TcpStream::connect(address).unwrap();
        let mut reader = BufReader::new(stream);
        assert!(receive(&mut reader).is_err());
    }
}
