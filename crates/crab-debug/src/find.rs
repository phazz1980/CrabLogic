//! Search of CrabLogic boards in the local network (`specs/net-find`).
//!
//! One UDP broadcast request per IPv4 adapter of this computer, replies collected on a socket
//! bound to each adapter's address. The wire format lives in `crab-netfind`.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::thread;
use std::time::{Duration, Instant};

use crab_netfind::{decode_reply, request, Board, PORT};

/// A board that answered the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Address the reply came from.
    pub ip: Ipv4Addr,
    pub board: Board,
}

/// Longest packet read from a socket; anything longer than a reply is rejected by the decoder.
const BUF_LEN: usize = 512;
/// Read timeout of one socket; the sockets are polled in turn until the search time is over.
const POLL: Duration = Duration::from_millis(20);

/// Searches the boards of the local network for `timeout`. Sends a request on every IPv4 adapter
/// with a broadcast address (loopback skipped) and returns the boards sorted by address, one
/// entry per board id. Returns an empty list if the adapters cannot be listed.
pub fn find_boards(timeout: Duration) -> Vec<Found> {
    find_with(&adapter_targets(), timeout)
}

/// Pairs of (local address to bind, where to send the request) for every usable IPv4 adapter.
fn adapter_targets() -> Vec<(Ipv4Addr, SocketAddr)> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut targets: Vec<(Ipv4Addr, SocketAddr)> = Vec::new();
    for iface in interfaces {
        if iface.is_loopback() {
            continue;
        }
        let if_addrs::IfAddr::V4(v4) = iface.addr else {
            continue;
        };
        if v4.ip.is_loopback() || targets.iter().any(|(ip, _)| *ip == v4.ip) {
            continue;
        }
        if let Some(broadcast) = v4.broadcast {
            targets.push((v4.ip, SocketAddr::new(IpAddr::V4(broadcast), PORT)));
        }
    }
    targets
}

/// The search over explicit targets: binds each local address, sends the request to its
/// destination and collects the replies until `timeout`.
pub(crate) fn find_with(targets: &[(Ipv4Addr, SocketAddr)], timeout: Duration) -> Vec<Found> {
    let mut sockets: Vec<UdpSocket> = Vec::new();
    for (local, dest) in targets {
        let Ok(socket) = UdpSocket::bind(SocketAddr::new(IpAddr::V4(*local), 0)) else {
            continue;
        };
        if socket.set_broadcast(true).is_err()
            || socket.set_read_timeout(Some(POLL)).is_err()
            || socket.send_to(&request(), dest).is_err()
        {
            continue;
        }
        sockets.push(socket);
    }

    let mut found: Vec<Found> = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; BUF_LEN];
    while !sockets.is_empty() && Instant::now() < deadline {
        let mut i = 0;
        while i < sockets.len() {
            match sockets[i].recv_from(&mut buf) {
                Ok((len, from)) => {
                    if let (IpAddr::V4(ip), Some(board)) = (from.ip(), decode_reply(&buf[..len])) {
                        if !found.iter().any(|f| f.board.id == board.id) {
                            found.push(Found { ip, board });
                        }
                    }
                    i += 1;
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    i += 1;
                }
                // Windows reports an earlier unreachable-port error here (ICMP) and the socket
                // still gets the replies that come later. A socket that keeps failing is paced by
                // the sleep, so the loop does not spin.
                Err(_) => {
                    thread::sleep(POLL);
                    i += 1;
                }
            }
        }
    }

    found.sort_by_key(|f| f.ip);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_netfind::is_request;

    const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;

    fn board(id: &str, name: &str) -> Board {
        Board {
            id: id.into(),
            chip: "esp32".into(),
            master: "2.0.1".into(),
            hash: Some("0badf00d".into()),
            has_key: true,
            port: 4040,
            name: name.into(),
        }
    }

    /// A fake board on 127.0.0.1: answers every search request with `reply` and, before it, sends
    /// `garbage`. Runs until no request arrives for about a second.
    fn responder(reply: Board, garbage: &'static [u8]) -> SocketAddr {
        let socket = UdpSocket::bind((LOOPBACK, 0)).expect("bind responder");
        socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("timeout");
        let addr = socket.local_addr().expect("addr");
        thread::spawn(move || {
            let mut buf = [0u8; BUF_LEN];
            let mut out = Vec::new();
            while let Ok((len, from)) = socket.recv_from(&mut buf) {
                if !is_request(&buf[..len]) {
                    continue;
                }
                let _ = socket.send_to(garbage, from);
                out.clear();
                if crab_netfind::encode_reply(&reply, &mut out) {
                    let _ = socket.send_to(&out, from);
                }
            }
        });
        addr
    }

    /// An address where nothing listens: a socket bound and dropped.
    fn silent_address() -> SocketAddr {
        let socket = UdpSocket::bind((LOOPBACK, 0)).expect("bind");
        socket.local_addr().expect("addr")
    }

    #[test]
    fn finds_board_with_its_address_and_skips_garbage() {
        let dest = responder(board("a1b2c3d4e5f6", "Pump"), b"CRABHERE\tgarbage");
        let found = find_with(&[(LOOPBACK, dest)], Duration::from_millis(800));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ip, LOOPBACK);
        assert_eq!(found[0].board, board("a1b2c3d4e5f6", "Pump"));
    }

    #[test]
    fn two_adapters_reaching_one_board_give_one_entry() {
        let dest = responder(board("a1b2c3d4e5f6", "Pump"), b"\x00\x01junk");
        let targets = [(LOOPBACK, dest), (LOOPBACK, dest)];
        let found = find_with(&targets, Duration::from_millis(800));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].board.id, "a1b2c3d4e5f6");
    }

    #[test]
    fn two_boards_are_both_listed_sorted_by_address() {
        let first = responder(board("000000000001", "A"), b"");
        let second = responder(board("000000000002", "B"), b"");
        let found = find_with(
            &[(LOOPBACK, second), (LOOPBACK, first)],
            Duration::from_millis(800),
        );
        let ids: Vec<&str> = found.iter().map(|f| f.board.id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"000000000001"));
        assert!(ids.contains(&"000000000002"));
        assert!(found.windows(2).all(|w| w[0].ip <= w[1].ip));
    }

    #[test]
    fn no_responder_gives_empty_list_within_timeout() {
        let timeout = Duration::from_millis(300);
        let started = Instant::now();
        let found = find_with(&[(LOOPBACK, silent_address())], timeout);
        let took = started.elapsed();
        assert!(found.is_empty());
        assert!(took >= timeout);
        assert!(took < timeout + Duration::from_millis(500), "took {took:?}");
    }

    #[test]
    fn no_targets_give_empty_list() {
        assert!(find_with(&[], Duration::from_millis(50)).is_empty());
    }
}
