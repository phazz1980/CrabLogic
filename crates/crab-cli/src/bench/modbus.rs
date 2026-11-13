//! The PC side of the Modbus TCP stand: a minimal client (function 03 only, `std::net` only) that
//! reads holding registers over and over and times every answer.

use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Registers read by one request.
pub const READ_COUNT: u16 = 10;
/// Requests in one run.
pub const REQUESTS: usize = 500;
/// Unit id of the slave block (its default address).
pub const UNIT: u8 = 1;
/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(1);
/// This many failures in a row mean the board is not answering: the run stops.
const GIVE_UP_AFTER: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModbusError {
    /// The frame ended before its own length said.
    Short,
    /// The answer is not for this request (transaction, protocol or unit differ).
    Mismatch,
    /// The slave answered with an exception code.
    Exception(u8),
    BadFrame(&'static str),
    Io(String),
}

impl fmt::Display for ModbusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short => f.write_str("короткий ответ"),
            Self::Mismatch => f.write_str("ответ не на этот запрос"),
            Self::Exception(code) => write!(f, "исключение Modbus {code:#04x}"),
            Self::BadFrame(why) => write!(f, "неверный ответ: {why}"),
            Self::Io(e) => write!(f, "ошибка связи: {e}"),
        }
    }
}

/// MBAP header and the PDU of function 03: transaction, protocol 0, length 6, unit, 03, start,
/// quantity.
pub fn encode_read_request(tid: u16, unit: u8, start: u16, count: u16) -> [u8; 12] {
    let mut f = [0u8; 12];
    f[0..2].copy_from_slice(&tid.to_be_bytes());
    // Protocol id 0.
    f[4..6].copy_from_slice(&6u16.to_be_bytes());
    f[6] = unit;
    f[7] = 3;
    f[8..10].copy_from_slice(&start.to_be_bytes());
    f[10..12].copy_from_slice(&count.to_be_bytes());
    f
}

/// Registers from a whole answer frame (MBAP header included).
pub fn parse_read_response(
    frame: &[u8],
    tid: u16,
    unit: u8,
    count: u16,
) -> Result<Vec<u16>, ModbusError> {
    // Header (7) + function + at least one more byte (exception code or byte count).
    if frame.len() < 9 {
        return Err(ModbusError::Short);
    }
    if u16::from_be_bytes([frame[0], frame[1]]) != tid
        || u16::from_be_bytes([frame[2], frame[3]]) != 0
        || frame[6] != unit
    {
        return Err(ModbusError::Mismatch);
    }
    match frame[7] {
        0x83 => Err(ModbusError::Exception(frame[8])),
        3 => {
            let bytes = usize::from(frame[8]);
            if bytes != usize::from(count) * 2 {
                return Err(ModbusError::BadFrame("не то число байт"));
            }
            if frame.len() < 9 + bytes {
                return Err(ModbusError::Short);
            }
            let data = &frame[9..9 + bytes];
            Ok((0..bytes / 2)
                .map(|i| u16::from_be_bytes([data[2 * i], data[2 * i + 1]]))
                .collect())
        }
        _ => Err(ModbusError::BadFrame("не та функция")),
    }
}

/// One whole frame from the stream: the header says how much follows.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Vec<u8>, ModbusError> {
    let io = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ModbusError::Short
        } else {
            ModbusError::Io(e.kind().to_string())
        }
    };
    let mut frame = vec![0u8; 7];
    r.read_exact(&mut frame).map_err(io)?;
    let length = usize::from(u16::from_be_bytes([frame[4], frame[5]]));
    // Length counts the unit id (already read) and the PDU.
    if !(2..=254).contains(&length) {
        return Err(ModbusError::BadFrame("длина кадра"));
    }
    frame.resize(6 + length, 0);
    r.read_exact(&mut frame[7..]).map_err(io)?;
    Ok(frame)
}

/// Mean, 95th percentile (nearest rank) and maximum of the answer times, milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatencyStats {
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

pub fn latency_stats(samples_ms: &[f64]) -> Option<LatencyStats> {
    if samples_ms.is_empty() {
        return None;
    }
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (sorted.len() * 95).div_ceil(100);
    Some(LatencyStats {
        mean_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p95_ms: sorted[rank.clamp(1, sorted.len()) - 1],
        max_ms: sorted[sorted.len() - 1],
    })
}

struct Client {
    addr: SocketAddr,
    unit: u8,
    tid: u16,
    stream: Option<TcpStream>,
}

impl Client {
    fn connect(&mut self) -> Result<&mut TcpStream, ModbusError> {
        if self.stream.is_none() {
            let io = |e: std::io::Error| ModbusError::Io(e.kind().to_string());
            let s = TcpStream::connect_timeout(&self.addr, TIMEOUT).map_err(io)?;
            s.set_read_timeout(Some(TIMEOUT)).map_err(io)?;
            s.set_write_timeout(Some(TIMEOUT)).map_err(io)?;
            let _ = s.set_nodelay(true);
            self.stream = Some(s);
        }
        Ok(self.stream.as_mut().expect("connected above"))
    }

    /// One request; the connection is dropped on any failure and made again by the next one.
    fn read_holding(&mut self, start: u16, count: u16) -> Result<Vec<u16>, ModbusError> {
        self.tid = self.tid.wrapping_add(1);
        let (tid, unit) = (self.tid, self.unit);
        let result = (|| {
            let stream = self.connect()?;
            stream
                .write_all(&encode_read_request(tid, unit, start, count))
                .map_err(|e| ModbusError::Io(e.kind().to_string()))?;
            let frame = read_frame(stream)?;
            parse_read_response(&frame, tid, unit, count)
        })();
        if result.is_err() {
            self.stream = None;
        }
        result
    }
}

/// What a polling run measured.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Poll {
    /// Answer times of the good requests, milliseconds.
    pub samples_ms: Vec<f64>,
    pub errors: usize,
    pub first_error: Option<String>,
    /// Requests made (good and bad) and the time they took.
    pub requests: usize,
    pub elapsed: Duration,
    /// Stopped early: the board stopped answering.
    pub gave_up: bool,
}

/// `requests` sequential reads. After them the loop goes on (uncounted, so the board stays busy)
/// until `stop` is set: the caller measures the sheet's step time meanwhile.
pub fn poll(addr: SocketAddr, requests: usize, stop: &AtomicBool) -> Poll {
    let mut client = Client {
        addr,
        unit: UNIT,
        tid: 0,
        stream: None,
    };
    let mut p = Poll::default();
    let started = Instant::now();
    let mut in_a_row = 0;
    while p.requests < requests {
        let at = Instant::now();
        match client.read_holding(0, READ_COUNT) {
            Ok(_) => {
                p.samples_ms.push(at.elapsed().as_secs_f64() * 1000.0);
                in_a_row = 0;
            }
            Err(e) => {
                p.errors += 1;
                in_a_row += 1;
                p.first_error.get_or_insert_with(|| e.to_string());
                if in_a_row >= GIVE_UP_AFTER {
                    p.gave_up = true;
                    break;
                }
            }
        }
        p.requests += 1;
    }
    p.elapsed = started.elapsed();
    while !p.gave_up && !stop.load(Ordering::Relaxed) {
        let _ = client.read_holding(0, READ_COUNT);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Answer of a slave, unit 1, transaction 7: 3 registers 0x0001 0x1234 0xFFFF.
    const ANSWER: [u8; 15] = [
        0x00, 0x07, 0x00, 0x00, 0x00, 0x09, 0x01, 0x03, 0x06, 0x00, 0x01, 0x12, 0x34, 0xFF, 0xFF,
    ];

    #[test]
    fn request_is_a_standard_mbap_read() {
        assert_eq!(
            encode_read_request(0x0102, 1, 0, 10),
            [0x01, 0x02, 0, 0, 0, 6, 1, 3, 0, 0, 0, 10]
        );
        assert_eq!(
            encode_read_request(1, 17, 0x0100, 2),
            [0, 1, 0, 0, 0, 6, 17, 3, 1, 0, 0, 2]
        );
    }

    #[test]
    fn response_gives_the_registers() {
        assert_eq!(
            parse_read_response(&ANSWER, 7, 1, 3).unwrap(),
            [0x0001, 0x1234, 0xFFFF]
        );
        // The wrong transaction, unit or count are refused.
        assert_eq!(
            parse_read_response(&ANSWER, 8, 1, 3),
            Err(ModbusError::Mismatch)
        );
        assert_eq!(
            parse_read_response(&ANSWER, 7, 2, 3),
            Err(ModbusError::Mismatch)
        );
        assert!(matches!(
            parse_read_response(&ANSWER, 7, 1, 4),
            Err(ModbusError::BadFrame(_))
        ));
    }

    #[test]
    fn exception_response_is_reported_with_its_code() {
        let frame = [0x00, 0x07, 0x00, 0x00, 0x00, 0x03, 0x01, 0x83, 0x02];
        assert_eq!(
            parse_read_response(&frame, 7, 1, 3),
            Err(ModbusError::Exception(2))
        );
        let text = ModbusError::Exception(2).to_string();
        assert!(text.contains("0x02"), "{text}");
    }

    #[test]
    fn short_frames_are_errors_not_panics() {
        assert_eq!(parse_read_response(&[], 7, 1, 3), Err(ModbusError::Short));
        assert_eq!(
            parse_read_response(&ANSWER[..8], 7, 1, 3),
            Err(ModbusError::Short)
        );
        // The header promises 6 data bytes, 4 came.
        assert_eq!(
            parse_read_response(&ANSWER[..13], 7, 1, 3),
            Err(ModbusError::Short)
        );
        assert_eq!(
            read_frame(&mut Cursor::new(&ANSWER[..5])),
            Err(ModbusError::Short)
        );
        assert_eq!(
            read_frame(&mut Cursor::new(&ANSWER[..12])),
            Err(ModbusError::Short)
        );
    }

    #[test]
    fn frame_is_read_by_its_length_field() {
        // Bytes after the frame belong to the next answer.
        let mut two = ANSWER.to_vec();
        two.extend_from_slice(&[9, 9, 9]);
        let mut cursor = Cursor::new(two);
        assert_eq!(read_frame(&mut cursor).unwrap(), ANSWER);
        assert_eq!(cursor.position(), ANSWER.len() as u64);
        // An absurd length is refused before anything is allocated.
        let bad = [0, 1, 0, 0, 0xFF, 0xFF, 1, 3];
        assert!(matches!(
            read_frame(&mut Cursor::new(bad)),
            Err(ModbusError::BadFrame(_))
        ));
    }

    #[test]
    fn latency_stats_on_a_fixed_list() {
        // 1..=20 ms: mean 10.5, nearest-rank 95 % is the 19th value, maximum 20.
        let samples: Vec<f64> = (1..=20).map(f64::from).collect();
        let s = latency_stats(&samples).unwrap();
        assert!((s.mean_ms - 10.5).abs() < 1e-9);
        assert_eq!(s.p95_ms, 19.0);
        assert_eq!(s.max_ms, 20.0);
        // Order does not matter.
        let mut shuffled = samples;
        shuffled.reverse();
        assert_eq!(latency_stats(&shuffled), Some(s));
        let one = latency_stats(&[4.0]).unwrap();
        assert_eq!((one.mean_ms, one.p95_ms, one.max_ms), (4.0, 4.0, 4.0));
        assert_eq!(latency_stats(&[]), None);
    }

    #[test]
    fn client_polls_a_local_slave() {
        // A slave on the loopback: answers every read with the same ten registers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = [0u8; 12];
            while s.read_exact(&mut req).is_ok() {
                let mut a = vec![req[0], req[1], 0, 0, 0, 23, req[6], 3, 20];
                a.extend((0..10u16).flat_map(|r| r.to_be_bytes()));
                if s.write_all(&a).is_err() {
                    break;
                }
            }
        });
        let stop = AtomicBool::new(true);
        let p = poll(addr, 25, &stop);
        assert_eq!((p.requests, p.errors, p.samples_ms.len()), (25, 0, 25));
        assert!(!p.gave_up);
        drop(server);
    }

    #[test]
    fn polling_nobody_gives_up() {
        // Nothing listens on a port that was just free.
        let addr = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap()
        };
        let stop = AtomicBool::new(true);
        let p = poll(addr, REQUESTS, &stop);
        assert!(p.gave_up);
        assert_eq!(p.errors, GIVE_UP_AFTER);
        assert!(p.first_error.is_some());
    }
}
