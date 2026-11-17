//! Centre and slaves exchange frames through a simulated air: delay, loss, damaged bytes.

use lora_e220::frame::Ty;
use lora_e220::master::{Config, Master, Peer};
use lora_e220::slave::Slave;
use lora_e220::{Layout, Radio};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dest {
    Centre,
    Slave(u8),
}

struct Air {
    now: u32,
    rng: u32,
    delay: u32,
    loss_up: u32,
    loss_down: u32,
    corrupt: u32,
    /// Replies are lost until this time.
    replies_blocked_until: u32,
    frames: Vec<(u32, Dest, Vec<u8>)>,
}

impl Air {
    fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            now: 0,
            rng: 12345,
            delay: 0,
            loss_up: 0,
            loss_down: 0,
            corrupt: 0,
            replies_blocked_until: 0,
            frames: Vec::new(),
        }))
    }

    fn roll(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.rng >> 8) % 100
    }
}

struct End {
    air: Rc<RefCell<Air>>,
    me: Dest,
}

impl Radio for End {
    fn send(&mut self, frame: &[u8]) -> bool {
        let mut a = self.air.borrow_mut();
        let (dest, loss) = if self.me == Dest::Centre {
            (Dest::Slave((frame[0] >> 3) & 15), a.loss_down)
        } else {
            (Dest::Centre, a.loss_up)
        };
        let blocked = dest == Dest::Centre && a.now < a.replies_blocked_until;
        if blocked || a.roll() < loss {
            return true;
        }
        let mut bytes = frame.to_vec();
        if a.roll() < a.corrupt {
            let at = a.roll() as usize % bytes.len();
            bytes[at] ^= 1 << (a.roll() % 8);
        }
        let at = a.now + a.delay;
        a.frames.push((at, dest, bytes));
        true
    }

    fn recv(&mut self, buf: &mut [u8]) -> Option<usize> {
        let mut a = self.air.borrow_mut();
        let now = a.now;
        let i = a
            .frames
            .iter()
            .position(|(t, d, _)| *t <= now && *d == self.me)?;
        let (_, _, bytes) = a.frames.remove(i);
        buf[..bytes.len()].copy_from_slice(&bytes);
        Some(bytes.len())
    }
}

const REQ: [Ty; 3] = [Ty::Bool, Ty::Int, Ty::Float];
const REP: [Ty; 2] = [Ty::Int, Ty::Bool];

struct Net {
    air: Rc<RefCell<Air>>,
    master: Master,
    slaves: Vec<Slave>,
    centre: End,
    ends: Vec<End>,
    /// Slaves that do not run (switched off).
    off: Vec<bool>,
    centre_off: bool,
    now: u32,
}

impl Net {
    fn new(nodes: usize, cfg: Config) -> Self {
        let air = Air::new();
        let req = Layout::new(&REQ).unwrap();
        let rep = Layout::new(&REP).unwrap();
        let mut master = Master::new(cfg);
        let mut slaves = Vec::new();
        let mut ends = Vec::new();
        for n in 0..nodes {
            master.add(Peer::new(n as u8, req, rep)).unwrap();
            slaves.push(Slave::new(n as u8, req, rep, cfg.offline_ms));
            ends.push(End {
                air: air.clone(),
                me: Dest::Slave(n as u8),
            });
        }
        Net {
            centre: End {
                air: air.clone(),
                me: Dest::Centre,
            },
            air,
            master,
            slaves,
            ends,
            off: vec![false; nodes],
            centre_off: false,
            now: 0,
        }
    }

    fn run(&mut self, ms: u32) {
        for _ in 0..ms {
            self.now += 1;
            self.air.borrow_mut().now = self.now;
            if !self.centre_off {
                self.master.step(self.now, &mut self.centre);
            }
            for i in 0..self.slaves.len() {
                if !self.off[i] {
                    self.slaves[i].step(self.now, &mut self.ends[i]);
                }
            }
        }
    }

    fn command(&mut self, node: usize, value: u32) {
        self.master.peer_mut(node).unwrap().tx[1] = value;
    }
}

#[test]
fn values_travel_both_ways() {
    let mut net = Net::new(2, Config::default());
    net.master.peer_mut(0).unwrap().tx[..3].copy_from_slice(&[
        1,
        (-300i32) as u32,
        2.5f32.to_bits(),
    ]);
    net.slaves[0].tx = [77, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    net.run(3000);
    assert_eq!(
        net.slaves[0].rx[..3],
        [1, (-300i32) as u32, 2.5f32.to_bits()]
    );
    assert_eq!(net.master.peer(0).unwrap().rx[..2], [77, 1]);
    assert!(net.master.peer(0).unwrap().online);
    assert!(net.master.peer(1).unwrap().online);
    assert!(net.slaves[0].linked(net.now));
}

#[test]
fn a_command_reaches_the_slave_within_three_seconds_with_eight_slaves() {
    let mut net = Net::new(8, Config::default());
    net.air.borrow_mut().delay = 100;
    net.run(10_000);
    net.command(7, 4242);
    let started = net.now;
    while net.slaves[7].rx[1] != 4242 {
        net.run(10);
        assert!(net.now - started <= 3000, "the command is late");
    }
}

#[test]
fn a_command_goes_out_before_the_round_pause_ends() {
    let mut net = Net::new(2, Config::default());
    net.run(5000);
    net.command(1, 9);
    let started = net.now;
    while net.slaves[1].rx[1] != 9 {
        net.run(1);
        assert!(net.now - started < 500, "the command waited for the round");
    }
}

#[test]
fn lost_frames_are_repeated_and_the_command_is_applied_once() {
    let mut net = Net::new(1, Config::default());
    net.command(0, 5);
    // Every reply is lost for the first second: the centre repeats the same request.
    net.air.borrow_mut().replies_blocked_until = 1000;
    // Until the first reply is in: later full images are other exchanges.
    net.run(2000);
    assert_eq!(net.slaves[0].rx[1], 5);
    assert_eq!(net.slaves[0].applied, 1);
    assert!(net.slaves[0].stats.repeats >= 1);
    assert!(net.master.peer(0).unwrap().stats.repeats >= 1);
}

#[test]
fn thirty_percent_loss_still_delivers() {
    // Short frames: a reply is expected within 0.3 s, the slaves are allowed longer silence.
    let cfg = Config {
        reply_timeout_ms: 300,
        offline_ms: 10_000,
        ..Config::default()
    };
    let mut net = Net::new(3, cfg);
    {
        let mut a = net.air.borrow_mut();
        a.loss_up = 30;
        a.loss_down = 30;
    }
    for round in 1..=6u32 {
        for n in 0..3 {
            net.command(n, round * 10 + n as u32);
        }
        net.run(6000);
        for n in 0..3 {
            assert_eq!(net.slaves[n].rx[1], round * 10 + n as u32, "round {round}");
        }
    }
}

#[test]
fn damaged_frames_are_dropped_and_counted() {
    let mut net = Net::new(1, Config::default());
    net.air.borrow_mut().corrupt = 50;
    for v in 1..=20u32 {
        net.command(0, v);
        net.slaves[0].tx[0] = v + 1000;
        net.run(3000);
        // Only values that were really sent may show up on the other side.
        assert!(net.slaves[0].rx[1] <= v);
        assert!(net.master.peer(0).unwrap().rx[0] <= v + 1000);
    }
    assert_eq!(net.slaves[0].rx[1], 20);
    let bad = net.slaves[0].stats.bad_crc + net.master.peer(0).unwrap().stats.bad_crc;
    assert!(bad > 0);
}

#[test]
fn offline_is_set_and_cleared() {
    let cfg = Config {
        offline_ms: 3000,
        ..Config::default()
    };
    let mut net = Net::new(2, cfg);
    net.run(3000);
    assert!(net.master.peer(1).unwrap().online);
    net.off[1] = true;
    net.run(4000);
    assert!(!net.master.peer(1).unwrap().online);
    assert!(
        net.master.peer(0).unwrap().online,
        "the other slave is not affected"
    );
    net.off[1] = false;
    net.run(3000);
    assert!(net.master.peer(1).unwrap().online);
    // The slave notices the silence of the centre and keeps the last commands.
    net.command(0, 31);
    net.run(2000);
    net.centre_off = true;
    net.run(4000);
    assert!(!net.slaves[0].linked(net.now));
    assert_eq!(net.slaves[0].rx[1], 31);
    net.centre_off = false;
    net.run(2000);
    assert!(net.slaves[0].linked(net.now));
}

#[test]
fn other_types_never_get_online() {
    let cfg = Config::default();
    let mut net = Net::new(1, cfg);
    let req = Layout::new(&[Ty::Int, Ty::Int, Ty::Int]).unwrap();
    let rep = Layout::new(&REP).unwrap();
    net.slaves[0] = Slave::new(0, req, rep, cfg.offline_ms);
    net.run(5000);
    assert!(!net.master.peer(0).unwrap().online);
    assert!(net.slaves[0].stats.bad_crc > 0);
}

#[test]
fn the_table_has_room_for_eight_unique_slaves() {
    let req = Layout::new(&REQ).unwrap();
    let rep = Layout::new(&REP).unwrap();
    let mut m = Master::new(Config::default());
    for n in 0..8 {
        assert!(m.add(Peer::new(n, req, rep)).is_some());
    }
    assert!(m.add(Peer::new(9, req, rep)).is_none());
    let mut m = Master::new(Config::default());
    assert!(m.add(Peer::new(3, req, rep)).is_some());
    assert!(m.add(Peer::new(3, req, rep)).is_none());
}
