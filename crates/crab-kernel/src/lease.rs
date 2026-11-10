//! Resource leases: a module owns the outputs it drives, and dropping a lease (unload, trap,
//! replacement without hand-over) puts the output into its safe state.
//!
//! The safe state comes from the block's `on_fault` (bits 9–10 of the pin number the emitter
//! passes): 0 low, 1 high, 2 hold. Levels are physical (an inverted output already accounts
//! for the inversion when the emitter computes the bits).
//!
//! At a module replacement the leases pass to the new version marked as inherited, so outputs do
//! not glitch. The new module makes one its own by writing that pin; what it still has not
//! written after its first full step is no longer driven and ends ([`Leases::release_inherited`]).

use alloc::vec::Vec;

/// How a module uses a pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinUse {
    /// Digital output (`gpio_write`).
    Out,
    /// PWM output (`pwm_write`).
    Pwm,
}

/// What an output does when its module is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fault {
    /// Off: level 0, PWM duty 0.
    #[default]
    Low,
    /// On: level 1, PWM full duty.
    High,
    /// Keep the last value.
    Hold,
}

impl Fault {
    /// The `on_fault` bits 9–10 of a pin number as the emitter passes it.
    pub fn from_pin_bits(pin: i32) -> Self {
        match pin >> 9 & 3 {
            1 => Fault::High,
            2 => Fault::Hold,
            _ => Fault::Low,
        }
    }
}

/// The board hardware as the kernel needs it: putting an output into its safe state.
///
/// Implementations are cheap handles (a zero-sized type on the board, a shared stub on the PC):
/// every [`Lease`] keeps a clone to act from its `Drop`.
pub trait Hal: Clone {
    /// Called once when a lease of `pin` ends and `fault` is not [`Fault::Hold`].
    fn safe(&self, pin: u8, use_: PinUse, fault: Fault);
}

/// A board without outputs to protect (the PC simulator records outputs in its stub instead).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHal;

impl Hal for NoHal {
    fn safe(&self, _: u8, _: PinUse, _: Fault) {}
}

/// One output owned by one module.
pub struct Lease<H: Hal> {
    pin: u8,
    use_: PinUse,
    fault: Fault,
    /// Came from the previous version of the module and not written by this one yet.
    inherited: bool,
    hal: H,
}

impl<H: Hal> Lease<H> {
    pub fn pin(&self) -> u8 {
        self.pin
    }

    pub fn use_(&self) -> PinUse {
        self.use_
    }

    pub fn fault(&self) -> Fault {
        self.fault
    }
}

impl<H: Hal> Drop for Lease<H> {
    fn drop(&mut self) {
        if self.fault != Fault::Hold {
            self.hal.safe(self.pin, self.use_, self.fault);
        }
    }
}

/// The leases of one module.
pub struct Leases<H: Hal> {
    hal: H,
    held: Vec<Lease<H>>,
}

impl<H: Hal> Leases<H> {
    pub fn new(hal: H) -> Self {
        Self {
            hal,
            held: Vec::new(),
        }
    }

    /// Takes (or updates) the lease of `pin` for this module.
    pub fn take(&mut self, pin: u8, use_: PinUse, fault: Fault) {
        if let Some(l) = self.held.iter_mut().find(|l| l.pin == pin) {
            l.use_ = use_;
            l.fault = fault;
            l.inherited = false;
            return;
        }
        self.held.push(Lease {
            pin,
            use_,
            fault,
            inherited: false,
            hal: self.hal.clone(),
        });
    }

    /// Whether this module holds `pin`.
    pub fn holds(&self, pin: u8) -> bool {
        self.held.iter().any(|l| l.pin == pin)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Lease<H>> {
        self.held.iter()
    }

    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Ends every lease: each output goes to its safe state.
    pub fn release(&mut self) {
        self.held.clear();
    }

    /// Hands every lease over to `other` (module replacement): no output changes. The leases
    /// stay inherited until `other` writes the pin.
    pub fn hand_over(&mut self, other: &mut Leases<H>) {
        other.release();
        other.held.append(&mut self.held);
        for l in &mut other.held {
            l.inherited = true;
        }
    }

    /// Whether some lease is still only inherited (cheap check for the kernel's step).
    pub fn has_inherited(&self) -> bool {
        self.held.iter().any(|l| l.inherited)
    }

    /// Ends the leases still only inherited (the new version does not drive those pins);
    /// returns how many. `Hold` ones just go, the pin stays as it is.
    pub fn release_inherited(&mut self) -> usize {
        let before = self.held.len();
        self.held.retain(|l| !l.inherited);
        before - self.held.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::rc::Rc;
    use core::cell::RefCell;

    #[derive(Clone, Default)]
    struct Rec(Rc<RefCell<Vec<(u8, PinUse, Fault)>>>);

    impl Hal for Rec {
        fn safe(&self, pin: u8, use_: PinUse, fault: Fault) {
            self.0.borrow_mut().push((pin, use_, fault));
        }
    }

    #[test]
    fn inherited_leases_end_unless_taken() {
        let hal = Rec::default();
        let mut old = Leases::new(hal.clone());
        let mut new = Leases::new(hal.clone());
        old.take(1, PinUse::Out, Fault::Low);
        old.take(2, PinUse::Pwm, Fault::High);
        old.take(3, PinUse::Out, Fault::Hold);
        old.take(4, PinUse::Out, Fault::Low);
        assert!(!old.has_inherited());

        old.hand_over(&mut new);
        assert!(old.is_empty() && new.has_inherited());
        assert!(hal.0.borrow().is_empty());

        new.take(4, PinUse::Out, Fault::Low);
        assert_eq!(new.release_inherited(), 3);
        assert!(!new.has_inherited());
        assert_eq!(
            *hal.0.borrow(),
            [(1, PinUse::Out, Fault::Low), (2, PinUse::Pwm, Fault::High)],
            "hold gives no safe call"
        );
        assert!(new.holds(4) && !new.holds(1) && !new.holds(3));
        assert_eq!(new.release_inherited(), 0);
    }
}
