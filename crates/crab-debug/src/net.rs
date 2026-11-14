//! The master over the network (`specs/ota`): the same line protocol as over USB, behind a TCP port
//! (4040) with a challenge-response login. The key is `SHA-256("crablogic" || password)`; the board
//! sends `auth <nonce hex>`, the client answers `auth <HMAC-SHA256(key, nonce) hex>`. The key itself
//! is written to the board only over USB (`netkey`).

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use crate::transport::{Connector, Transport};

/// Port of the master when the address has none.
pub const DEFAULT_PORT: u16 = 4040;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Read timeout of the debug link: the same as the serial port has.
const READ_TIMEOUT: Duration = Duration::from_millis(50);
/// How long a write of the debug link may stall before the link is declared lost.
const WRITE_DEADLINE: Duration = Duration::from_millis(500);
/// Write timeout of a stream for the upload, where a big chunk may take longer.
const UPLOAD_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the login waits for a line of the board.
const LINE_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("у проекта нет пароля: работа по сети требует пароль ([project] password)")]
    NoPassword,
    #[error("плата {host} не отвечает: {reason}")]
    Connect { host: String, reason: String },
    #[error("к плате уже подключён другой клиент")]
    Busy,
    #[error("плата занята: с ней работают по USB")]
    BusyUsb,
    #[error(
        "неверный пароль для работы по сети (или ключ на плате не записан — \
         запишите его по USB: crablogic netkey)"
    )]
    BadPassword,
    #[error("плата ответила не по протоколу: {0}")]
    Protocol(String),
    #[error("ошибка связи с платой: {0}")]
    Io(String),
}

impl From<io::Error> for LoginError {
    fn from(e: io::Error) -> Self {
        LoginError::Io(e.to_string())
    }
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

/// Lower-case hex of `bytes`.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes of a hex string; `None` for an odd length or a non-hex character.
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok())
        .collect()
}

/// `host` with the default port when it has none; a `tcp:` prefix is dropped.
pub fn with_port(host: &str) -> String {
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

fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

/// Reads one non-empty line (without the line ending); read timeouts are retried for ~3 s.
fn read_line<P: Read>(port: &mut P) -> Result<String, LoginError> {
    let deadline = Instant::now() + LINE_WAIT;
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match port.read(&mut byte) {
            Ok(0) => return Err(LoginError::Io("плата закрыла соединение".into())),
            Ok(_) => match byte[0] {
                b'\n' => {
                    let text = String::from_utf8_lossy(&line).trim().to_owned();
                    if !text.is_empty() {
                        return Ok(text);
                    }
                    line.clear();
                }
                b => line.push(b),
            },
            Err(e) if is_timeout(&e) => {
                if Instant::now() >= deadline {
                    return Err(LoginError::Io("плата не ответила".into()));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// The login on an opened link.
pub fn login<P: Read + Write>(port: &mut P, password: &str) -> Result<(), LoginError> {
    if password.is_empty() {
        return Err(LoginError::NoPassword);
    }
    let line = read_line(port)?;
    match line.as_str() {
        "err busy" => return Err(LoginError::Busy),
        "err busy usb" => return Err(LoginError::BusyUsb),
        _ => {}
    }
    let nonce = line
        .strip_prefix("auth ")
        .and_then(|hex| from_hex(hex.trim()))
        .filter(|n| !n.is_empty())
        .ok_or_else(|| LoginError::Protocol(line.clone()))?;
    let answer = auth_answer(&net_key(password), &nonce);
    port.write_all(format!("auth {}\n", to_hex(&answer)).as_bytes())?;
    port.flush()?;
    match read_line(port)?.as_str() {
        "ok" => Ok(()),
        "err auth" => Err(LoginError::BadPassword),
        "err busy" => Err(LoginError::Busy),
        "err busy usb" => Err(LoginError::BusyUsb),
        other => Err(LoginError::Protocol(other.to_owned())),
    }
}

/// Connects to the master at `host` and logs in with the project password. Reads on the stream
/// time out after `read_timeout` (`WouldBlock` or `TimedOut`).
pub fn connect(
    host: &str,
    password: &str,
    read_timeout: Duration,
) -> Result<TcpStream, LoginError> {
    if password.is_empty() {
        return Err(LoginError::NoPassword);
    }
    let fail = |reason: String| LoginError::Connect {
        host: host.to_owned(),
        reason,
    };
    let addrs = with_port(host)
        .to_socket_addrs()
        .map_err(|e| fail(e.to_string()))?;
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
    let setup = |s: &TcpStream| -> io::Result<()> {
        s.set_nodelay(true)?;
        s.set_read_timeout(Some(read_timeout))?;
        s.set_write_timeout(Some(UPLOAD_WRITE_TIMEOUT))
    };
    setup(&stream).map_err(|e| fail(e.to_string()))?;
    login(&mut stream, password).map_err(|e| match e {
        LoginError::Io(reason) => fail(reason),
        other => other,
    })?;
    Ok(stream)
}

struct TcpTransport(TcpStream);

impl Transport for TcpTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            // A closed socket is a lost link, not silence: the session reconnects.
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "плата закрыла соединение",
            )),
            Ok(n) => Ok(n),
            Err(e) if is_timeout(&e) => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn write_all(&mut self, mut data: &[u8]) -> io::Result<()> {
        let deadline = Instant::now() + WRITE_DEADLINE;
        while !data.is_empty() {
            match self.0.write(data) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => data = &data[n..],
                Err(e) if is_timeout(&e) => {
                    // A half-sent line would break the framing: the link is reopened instead.
                    if Instant::now() >= deadline {
                        return Err(io::ErrorKind::TimedOut.into());
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Connector for the master at `host` (`192.168.1.5`, `host:4040`, `tcp:host`): TCP with the login
/// by the project password. A failed login is an error of the connector with the Russian reason;
/// the session shows it as its status and tries again.
pub fn tcp(host: &str, password: &str) -> Connector {
    let (host, password) = (host.to_owned(), password.to_owned());
    Box::new(move || {
        let stream =
            connect(&host, &password, READ_TIMEOUT).map_err(|e| io::Error::other(e.to_string()))?;
        stream.set_write_timeout(Some(WRITE_DEADLINE))?;
        Ok(Box::new(TcpTransport(stream)) as Box<dyn Transport>)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    const NONCE: [u8; 16] = *b"0123456789abcdef";
    const T: Duration = Duration::from_millis(50);

    fn listener() -> (TcpListener, String) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        (l, addr)
    }

    /// A fake master: sends the challenge, checks the answer against `password`, then runs `after`.
    fn fake_master(
        password: &'static str,
        after: impl FnOnce(TcpStream) + Send + 'static,
    ) -> String {
        let (l, addr) = listener();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
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

    /// A master that says one line instead of the challenge and waits for the client to leave.
    fn one_line(line: &'static str) -> String {
        let (l, addr) = listener();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            write!(s, "{line}\r\n").unwrap();
            let mut sink = [0u8; 64];
            let _ = s.read(&mut sink);
        });
        addr
    }

    #[test]
    fn key_and_answer() {
        let mut h = Sha256::new();
        h.update(b"crablogicsecret");
        let want: [u8; 32] = h.finalize().into();
        assert_eq!(net_key("secret"), want);
        let key = net_key("secret");
        assert_eq!(auth_answer(&key, &NONCE), auth_answer(&key, &NONCE));
        assert_ne!(
            auth_answer(&key, &NONCE),
            auth_answer(&net_key("x"), &NONCE)
        );
        assert_eq!(from_hex("00ff10").unwrap(), vec![0, 255, 16]);
        assert!(from_hex("0").is_none());
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
    fn good_login() {
        let addr = fake_master("secret", |_| {});
        assert!(connect(&addr, "secret", T).is_ok());
    }

    #[test]
    fn wrong_password() {
        let addr = fake_master("secret", |_| {});
        assert!(matches!(
            connect(&addr, "wrong", T),
            Err(LoginError::BadPassword)
        ));
    }

    #[test]
    fn busy_variants() {
        let addr = one_line("err busy");
        assert!(matches!(connect(&addr, "x", T), Err(LoginError::Busy)));
        let addr = one_line("err busy usb");
        let e = connect(&addr, "x", T).unwrap_err();
        assert!(matches!(e, LoginError::BusyUsb), "{e}");
        assert_eq!(e.to_string(), "плата занята: с ней работают по USB");
        // As the answer to `auth`.
        let (l, addr) = listener();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            write!(s, "auth {}\r\n", to_hex(&NONCE)).unwrap();
            let mut line = String::new();
            BufReader::new(s.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            s.write_all(b"err busy usb\r\n").unwrap();
        });
        assert!(matches!(connect(&addr, "x", T), Err(LoginError::BusyUsb)));
    }

    #[test]
    fn empty_password_and_dead_host() {
        assert!(matches!(
            connect("127.0.0.1:1", "", T),
            Err(LoginError::NoPassword)
        ));
        let (l, addr) = listener();
        drop(l);
        let e = connect(&addr, "x", T).unwrap_err();
        assert!(e.to_string().starts_with("плата "), "{e}");
    }

    #[test]
    fn tcp_transport_reads_and_is_silent() {
        let addr = fake_master("secret", |mut s| {
            s.write_all(b"hello\n").unwrap();
            // Silent, but open.
            std::thread::sleep(Duration::from_millis(800));
        });
        let mut t = tcp(&addr, "secret")().unwrap();
        let mut buf = [0u8; 32];
        let mut got = Vec::new();
        let end = Instant::now() + Duration::from_secs(2);
        while got.len() < 6 && Instant::now() < end {
            let n = t.read(&mut buf).unwrap();
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(got, b"hello\n");
        assert_eq!(t.read(&mut buf).unwrap(), 0);
        t.write_all(b"info\n").unwrap();
    }

    #[test]
    fn closed_connection_is_an_error() {
        let addr = fake_master("secret", drop);
        let mut t = tcp(&addr, "secret")().unwrap();
        let mut buf = [0u8; 8];
        let end = Instant::now() + Duration::from_secs(2);
        loop {
            match t.read(&mut buf) {
                Ok(0) if Instant::now() < end => {}
                Ok(n) => panic!("unexpected result after close: {n}"),
                Err(_) => break,
            }
        }
    }

    #[test]
    fn failed_login_is_the_connector_error() {
        let addr = fake_master("secret", |_| {});
        let e = tcp(&addr, "wrong")().err().unwrap();
        assert!(e.to_string().contains("неверный пароль"), "{e}");
        let e = tcp(&addr, "")().err().unwrap();
        assert!(e.to_string().contains("нет пароля"), "{e}");
    }
}
