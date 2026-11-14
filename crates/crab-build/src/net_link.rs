//! Talking to the master over the network (`specs/ota`): the same line protocol as over USB, behind
//! a TCP port (4040) with a challenge-response login. The key is `SHA-256("crablogic" || password)`;
//! the board sends `auth <nonce hex>`, the client answers `auth <HMAC-SHA256(key, nonce) hex>`.
//! The key itself is written to the board only over USB (`netkey`).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::mcu_module::{read_line, ModuleUploadError};

/// Port of the master when the address has none.
pub const DEFAULT_PORT: u16 = 4040;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Same short read timeout as the serial port has: the readers retry it until their own limit.
const READ_TIMEOUT: Duration = Duration::from_millis(200);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Anything the upload code can talk to: a serial port or a TCP stream.
pub trait Link: Read + Write {}
impl<T: Read + Write + ?Sized> Link for T {}

#[derive(Debug, thiserror::Error)]
pub enum NetLinkError {
    #[error("у проекта нет пароля: заливка по сети требует пароль ([project] password)")]
    NoPassword,
    #[error("плата {host} не отвечает: {reason}")]
    Connect { host: String, reason: String },
    #[error("к плате уже подключён другой клиент")]
    Busy,
    #[error(
        "неверный пароль для заливки по сети (или ключ на плате не записан — \
         запишите его по USB: crablogic netkey)"
    )]
    BadPassword,
    #[error("плата ответила не по протоколу: {0}")]
    Protocol(String),
    #[error("мастер на плате не поддерживает заливку по сети — перезалейте мастер")]
    Unsupported,
    #[error("ошибка связи с платой: {0}")]
    Link(String),
}

impl From<ModuleUploadError> for NetLinkError {
    fn from(e: ModuleUploadError) -> Self {
        NetLinkError::Link(e.to_string())
    }
}

impl From<std::io::Error> for NetLinkError {
    fn from(e: std::io::Error) -> Self {
        NetLinkError::Link(e.to_string())
    }
}

/// Where the board is: a serial port or a network address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardLink {
    Serial(String),
    Tcp(String),
}

impl BoardLink {
    /// `COM3` and `/dev/ttyUSB0` are serial ports; `192.168.1.5`, `host:4040`, `tcp:host` are
    /// network addresses.
    pub fn parse(text: &str) -> BoardLink {
        let text = text.trim();
        if let Some(addr) = text.strip_prefix("tcp:") {
            return BoardLink::Tcp(addr.trim().to_owned());
        }
        if looks_like_address(text) {
            BoardLink::Tcp(text.to_owned())
        } else {
            BoardLink::Serial(text.to_owned())
        }
    }
}

/// A `tcp:` prefix, or a dot in the name (an IP address or a host name) and no path separators.
pub fn looks_like_address(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("tcp:") || (text.contains('.') && !text.contains(['/', '\\']))
}

/// The login key from the project password.
pub fn net_key(password: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"crablogic");
    hash.update(password.as_bytes());
    hash.finalize().into()
}

/// The answer to the challenge: `HMAC-SHA256(key, nonce)`.
pub fn auth_answer(key: &[u8; 32], nonce: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(nonce);
    mac.finalize().into_bytes().into()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok())
        .collect()
}

fn with_port(host: &str) -> String {
    let host = host.trim().strip_prefix("tcp:").unwrap_or(host.trim());
    // `host:port`, or a bare host / IPv4 (an IPv6 literal in brackets is `[::1]:port`).
    let has_port = match host.rsplit_once(':') {
        Some((head, tail)) => {
            !tail.is_empty()
                && tail.bytes().all(|b| b.is_ascii_digit())
                && (!head.contains(':') || head.ends_with(']'))
        }
        None => false,
    };
    if has_port {
        host.to_owned()
    } else {
        format!("{host}:{DEFAULT_PORT}")
    }
}

/// Connects to the master at `host` and logs in with the project password. The stream behaves like
/// the serial port: reads time out after 200 ms (`WouldBlock` or `TimedOut`, both retried by the
/// upload code).
pub fn connect(host: &str, password: &str) -> Result<TcpStream, NetLinkError> {
    if password.is_empty() {
        return Err(NetLinkError::NoPassword);
    }
    let addr = with_port(host);
    let fail = |reason: String| NetLinkError::Connect {
        host: host.to_owned(),
        reason,
    };
    let addrs = addr.to_socket_addrs().map_err(|e| fail(e.to_string()))?;
    let mut last = String::from("адрес не найден");
    let mut stream = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last = e.to_string(),
        }
    }
    let mut stream = stream.ok_or_else(|| fail(last))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    login(&mut stream, password).map_err(|e| match e {
        NetLinkError::Link(reason) => fail(reason),
        other => other,
    })?;
    Ok(stream)
}

/// The login on an opened link.
fn login<P: Read + Write>(port: &mut P, password: &str) -> Result<(), NetLinkError> {
    let line = read_line(port)?;
    if line == "err busy" {
        return Err(NetLinkError::Busy);
    }
    let nonce = line
        .strip_prefix("auth ")
        .and_then(|hex| from_hex(hex.trim()))
        .filter(|n| !n.is_empty())
        .ok_or_else(|| NetLinkError::Protocol(line.clone()))?;
    let answer = auth_answer(&net_key(password), &nonce);
    port.write_all(format!("auth {}\n", to_hex(&answer)).as_bytes())?;
    port.flush()?;
    match read_line(port)?.as_str() {
        "ok" => Ok(()),
        "err auth" => Err(NetLinkError::BadPassword),
        "err busy" => Err(NetLinkError::Busy),
        other => Err(NetLinkError::Protocol(other.to_owned())),
    }
}

/// Writes the network key of `password` to the board over USB (`netkey <hex>`), or removes it
/// (`None`: `netkey -`).
pub fn set_net_key<P: Read + Write>(
    port: &mut P,
    password: Option<&str>,
) -> Result<(), NetLinkError> {
    let command = match password {
        Some("") => return Err(NetLinkError::NoPassword),
        Some(p) => format!("netkey {}\n", to_hex(&net_key(p))),
        None => "netkey -\n".to_owned(),
    };
    port.write_all(command.as_bytes())?;
    port.flush()?;
    // The board may print log lines before it answers.
    for _ in 0..16 {
        let line = read_line(port)?;
        if line == "ok" {
            return Ok(());
        }
        if line.starts_with("err") {
            return Err(NetLinkError::Unsupported);
        }
    }
    Err(NetLinkError::Protocol("нет ответа на netkey".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcu_module::upload_module;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    const NONCE: [u8; 16] = *b"0123456789abcdef";

    /// A fake master: sends the challenge, checks the answer against `password`, then runs `after`.
    fn fake_master(
        password: &'static str,
        after: impl FnOnce(TcpStream) + Send + 'static,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            write!(s, "auth {}\r\n", to_hex(&NONCE)).unwrap();
            let mut line = String::new();
            BufReader::new(s.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            let want = format!("auth {}", to_hex(&auth_answer(&net_key(password), &NONCE)));
            if line.trim() == want {
                s.write_all(b"ok\r\n").unwrap();
                after(s);
            } else {
                s.write_all(b"err auth\r\n").unwrap();
            }
        });
        addr
    }

    #[test]
    fn key_is_sha256_of_prefix_and_password() {
        let mut h = Sha256::new();
        h.update(b"crablogicsecret");
        let want: [u8; 32] = h.finalize().into();
        assert_eq!(net_key("secret"), want);
        assert_ne!(net_key("a"), net_key("b"));
    }

    #[test]
    fn auth_answer_is_hmac_of_the_nonce() {
        let key = net_key("secret");
        let a = auth_answer(&key, &NONCE);
        assert_eq!(a, auth_answer(&key, &NONCE));
        assert_ne!(a, auth_answer(&key, b"fedcba9876543210"));
        assert_ne!(a, auth_answer(&net_key("other"), &NONCE));
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(from_hex("00ff10").unwrap(), vec![0, 255, 16]);
        assert!(from_hex("0").is_none());
        assert!(from_hex("zz").is_none());
    }

    #[test]
    fn addresses_get_the_default_port() {
        assert_eq!(with_port("1.2.3.4"), "1.2.3.4:4040");
        assert_eq!(with_port("1.2.3.4:5000"), "1.2.3.4:5000");
        assert_eq!(with_port("tcp:board.local"), "board.local:4040");
        assert_eq!(with_port("[::1]"), "[::1]:4040");
        assert_eq!(with_port("[::1]:7"), "[::1]:7");
    }

    #[test]
    fn link_parsing() {
        assert_eq!(BoardLink::parse("COM3"), BoardLink::Serial("COM3".into()));
        assert_eq!(
            BoardLink::parse("192.168.1.5"),
            BoardLink::Tcp("192.168.1.5".into())
        );
        assert_eq!(
            BoardLink::parse("tcp:board"),
            BoardLink::Tcp("board".into())
        );
        assert_eq!(
            BoardLink::parse("/dev/ttyUSB0"),
            BoardLink::Serial("/dev/ttyUSB0".into())
        );
    }

    #[test]
    fn handshake_succeeds() {
        let addr = fake_master("secret", |_| {});
        assert!(connect(&addr, "secret").is_ok());
    }

    #[test]
    fn wrong_password_is_reported() {
        let addr = fake_master("secret", |_| {});
        assert!(matches!(
            connect(&addr, "wrong"),
            Err(NetLinkError::BadPassword)
        ));
    }

    #[test]
    fn busy_board_is_reported() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.write_all(b"err busy\r\n").unwrap();
        });
        assert!(matches!(connect(&addr, "x"), Err(NetLinkError::Busy)));
    }

    #[test]
    fn empty_password_and_dead_host() {
        assert!(matches!(
            connect("127.0.0.1:1", ""),
            Err(NetLinkError::NoPassword)
        ));
        let free = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = free.local_addr().unwrap().to_string();
        drop(free);
        let e = connect(&addr, "x").unwrap_err();
        assert!(e.to_string().starts_with("плата "), "{e}");
    }

    #[test]
    fn module_goes_up_over_tcp() {
        // An empty module with a `crab.mem` section.
        const WASM: &[u8] = b"\0asm\x01\0\0\0\0\x0d\x08crab.mem\0\x10\0\0";
        let addr = fake_master("secret", |mut s| {
            // Header line, then the master takes 8 bytes at a time and acks each chunk.
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut header = String::new();
            reader.read_line(&mut header).unwrap();
            assert!(header.starts_with("put m1 "), "{header}");
            s.write_all(b"ready 8\r\n").unwrap();
            let mut got = 0;
            let total = WASM.len();
            while got < total {
                let n = (total - got).min(8);
                let mut buf = vec![0u8; n];
                reader.read_exact(&mut buf).unwrap();
                got += n;
                write!(s, "ack {got}\r\n").unwrap();
            }
            s.write_all(b"swap_us=5\r\nok\r\n").unwrap();
        });
        let mut stream = connect(&addr, "secret").unwrap();
        let done = upload_module(&mut stream, "m1", WASM, &[]).unwrap();
        assert_eq!(done.swap_us, Some(5));
    }

    struct Fake {
        out: std::collections::VecDeque<u8>,
        written: Vec<u8>,
        reply: &'static str,
    }

    impl Fake {
        fn new(reply: &'static str) -> Self {
            Fake {
                out: Default::default(),
                written: Vec::new(),
                reply,
            }
        }
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.out.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Err(std::io::ErrorKind::TimedOut.into()),
            }
        }
    }

    impl Write for Fake {
        fn write(&mut self, d: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(d);
            self.out.extend(self.reply.bytes());
            Ok(d.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn netkey_command_waits_for_ok() {
        let mut ok = Fake::new("ok\r\n");
        set_net_key(&mut ok, Some("secret")).unwrap();
        assert_eq!(
            String::from_utf8(ok.written).unwrap(),
            format!("netkey {}\n", to_hex(&net_key("secret")))
        );
        let mut rm = Fake::new("ok\r\n");
        set_net_key(&mut rm, None).unwrap();
        assert_eq!(rm.written, b"netkey -\n");
        let mut old = Fake::new("err unknown command\r\n");
        assert!(matches!(
            set_net_key(&mut old, Some("x")),
            Err(NetLinkError::Unsupported)
        ));
        assert!(matches!(
            set_net_key(&mut Fake::new("ok\n"), Some("")),
            Err(NetLinkError::NoPassword)
        ));
    }
}
