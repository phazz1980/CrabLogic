//! The `crab.config` blob of a modem block: role and the partners with the types of their values.
//! The generator writes it (host side, [`encode`]), the driver reads it ([`decode`], no allocation).
//!
//! ```text
//! version(1) | centre(1: 1 centre, 0 slave) | peers(1) | peers × ( node(1) | n_tx(1) | n_rx(1) | tx types | rx types )
//! ```
//! Type bytes: 0 bool, 1 int, 2 float. "tx" is what this side sends to the partner (the commands
//! on the centre, the answers on a slave), "rx" what it receives. The extra cells of the driver
//! follow the same order: the inputs are the `tx` cells of every partner, the outputs are the
//! `rx` cells of every partner followed by one flag cell (`online` / `linked`).

use crate::frame::{Ty, MAX_VALUES};
use crate::master::MAX_PEERS;
use crate::Layout;

pub const VERSION: u8 = 1;

/// One partner as the driver sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub node: u8,
    pub tx: Layout,
    pub rx: Layout,
}

/// The whole configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub centre: bool,
    peers: [Option<Peer>; MAX_PEERS],
}

impl Config {
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.iter().flatten()
    }

    /// Extra input cells: the values sent to the partners.
    pub fn n_in(&self) -> usize {
        self.peers().map(|p| p.tx.len()).sum()
    }

    /// Extra output cells: the values received from the partners and a flag for each.
    pub fn n_out(&self) -> usize {
        self.peers().map(|p| p.rx.len() + 1).sum()
    }
}

fn ty_of(b: u8) -> Option<Ty> {
    match b {
        0 => Some(Ty::Bool),
        1 => Some(Ty::Int),
        2 => Some(Ty::Float),
        _ => None,
    }
}

/// Reads a blob; `None` for anything that is not a valid configuration.
pub fn decode(blob: &[u8]) -> Option<Config> {
    let [version, centre, n, rest @ ..] = blob else {
        return None;
    };
    let n = usize::from(*n);
    if *version != VERSION || *centre > 1 || n == 0 || n > MAX_PEERS || (*centre == 0 && n != 1) {
        return None;
    }
    let mut cfg = Config {
        centre: *centre == 1,
        peers: [None; MAX_PEERS],
    };
    let mut at = rest;
    for slot in cfg.peers.iter_mut().take(n) {
        let [node, ntx, nrx, tail @ ..] = at else {
            return None;
        };
        let (ntx, nrx) = (usize::from(*ntx), usize::from(*nrx));
        if ntx > MAX_VALUES || nrx > MAX_VALUES || tail.len() < ntx + nrx {
            return None;
        }
        let mut tx = [Ty::Bool; MAX_VALUES];
        let mut rx = [Ty::Bool; MAX_VALUES];
        let (tx_bytes, rx_bytes) = tail.get(..ntx + nrx)?.split_at_checked(ntx)?;
        for (t, b) in tx.iter_mut().zip(tx_bytes) {
            *t = ty_of(*b)?;
        }
        for (t, b) in rx.iter_mut().zip(rx_bytes) {
            *t = ty_of(*b)?;
        }
        *slot = Some(Peer {
            node: *node,
            tx: Layout::new(tx.get(..ntx)?)?,
            rx: Layout::new(rx.get(..nrx)?)?,
        });
        at = tail.get(ntx + nrx..)?;
    }
    // No two partners share an address.
    let dup = cfg
        .peers()
        .enumerate()
        .any(|(i, a)| cfg.peers().skip(i + 1).any(|b| b.node == a.node));
    (at.is_empty() && !dup).then_some(cfg)
}

/// Writes a blob (host side). `peers`: node, types sent, types received.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(centre: bool, peers: &[(u8, Vec<Ty>, Vec<Ty>)]) -> Vec<u8> {
    let code = |t: &Ty| match t {
        Ty::Bool => 0u8,
        Ty::Int => 1,
        Ty::Float => 2,
    };
    let mut out = vec![VERSION, u8::from(centre), peers.len() as u8];
    for (node, tx, rx) in peers {
        out.extend([*node, tx.len() as u8, rx.len() as u8]);
        out.extend(tx.iter().map(code));
        out.extend(rx.iter().map(code));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(s: &str) -> Vec<Ty> {
        s.chars()
            .map(|c| match c {
                'b' => Ty::Bool,
                'i' => Ty::Int,
                _ => Ty::Float,
            })
            .collect()
    }

    #[test]
    fn roundtrip() {
        let blob = encode(
            true,
            &[(0, types("bif"), types("ii")), (3, types(""), types("b"))],
        );
        let cfg = decode(&blob).unwrap();
        assert!(cfg.centre);
        let peers: Vec<_> = cfg.peers().collect();
        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].node, 0);
        assert_eq!(peers[0].tx.types(), &types("bif")[..]);
        assert_eq!(peers[0].rx.types(), &types("ii")[..]);
        assert_eq!(peers[1].tx.len(), 0);
        assert_eq!(cfg.n_in(), 3);
        assert_eq!(cfg.n_out(), (2 + 1) + (1 + 1));
    }

    #[test]
    fn bad_blobs_are_refused() {
        let good = encode(false, &[(1, types("i"), types("b"))]);
        assert!(decode(&good).is_some());
        assert!(decode(&[]).is_none());
        assert!(decode(&good[..good.len() - 1]).is_none(), "cut");
        let mut long = good.clone();
        long.push(0);
        assert!(decode(&long).is_none(), "trailing bytes");
        let mut bad_ty = good.clone();
        *bad_ty.last_mut().unwrap() = 9;
        assert!(decode(&bad_ty).is_none());
        let mut other = good.clone();
        other[0] = 2;
        assert!(decode(&other).is_none(), "version");
        // A slave has exactly one partner; the centre at most eight, all different.
        assert!(decode(&encode(false, &[(0, vec![], vec![]), (1, vec![], vec![])])).is_none());
        assert!(decode(&encode(true, &[(2, vec![], vec![]), (2, vec![], vec![])])).is_none());
        assert!(decode(&encode(true, &[])).is_none());
    }
}
