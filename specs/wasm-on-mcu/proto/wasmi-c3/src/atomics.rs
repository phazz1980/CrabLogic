//! `__sync_*` functions for LLVM `forced-atomics` on a single-core RV32IMC without the A extension:
//! each operation runs with machine interrupts off.

use core::ptr::{read_volatile, write_volatile};

#[inline(always)]
fn critical<R>(f: impl FnOnce() -> R) -> R {
    let old: usize;
    // Clears MIE (bit 3 of mstatus) and returns the previous value.
    unsafe { core::arch::asm!("csrrci {0}, mstatus, 8", out(reg) old) };
    let r = f();
    if old & 8 != 0 {
        unsafe { core::arch::asm!("csrsi mstatus, 8") };
    }
    r
}

macro_rules! rmw {
    ($name:ident, $t:ty, |$old:ident, $v:ident| $new:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(p: *mut $t, $v: $t) -> $t {
            critical(|| unsafe {
                let $old = read_volatile(p);
                write_volatile(p, $new);
                $old
            })
        }
    };
}

macro_rules! ops {
    ($t:ty, $s:ty, $cas:ident, $xchg:ident, $add:ident, $sub:ident, $and:ident, $or:ident,
     $xor:ident, $nand:ident, $max:ident, $min:ident, $umax:ident, $umin:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $cas(p: *mut $t, expected: $t, new: $t) -> $t {
            critical(|| unsafe {
                let old = read_volatile(p);
                if old == expected {
                    write_volatile(p, new);
                }
                old
            })
        }
        rmw!($xchg, $t, |_o, v| v);
        rmw!($add, $t, |o, v| o.wrapping_add(v));
        rmw!($sub, $t, |o, v| o.wrapping_sub(v));
        rmw!($and, $t, |o, v| o & v);
        rmw!($or, $t, |o, v| o | v);
        rmw!($xor, $t, |o, v| o ^ v);
        rmw!($nand, $t, |o, v| !(o & v));
        rmw!($max, $t, |o, v| (o as $s).max(v as $s) as $t);
        rmw!($min, $t, |o, v| (o as $s).min(v as $s) as $t);
        rmw!($umax, $t, |o, v| o.max(v));
        rmw!($umin, $t, |o, v| o.min(v));
    };
}

ops!(
    u8,
    i8,
    __sync_val_compare_and_swap_1,
    __sync_lock_test_and_set_1,
    __sync_fetch_and_add_1,
    __sync_fetch_and_sub_1,
    __sync_fetch_and_and_1,
    __sync_fetch_and_or_1,
    __sync_fetch_and_xor_1,
    __sync_fetch_and_nand_1,
    __sync_fetch_and_max_1,
    __sync_fetch_and_min_1,
    __sync_fetch_and_umax_1,
    __sync_fetch_and_umin_1
);
ops!(
    u16,
    i16,
    __sync_val_compare_and_swap_2,
    __sync_lock_test_and_set_2,
    __sync_fetch_and_add_2,
    __sync_fetch_and_sub_2,
    __sync_fetch_and_and_2,
    __sync_fetch_and_or_2,
    __sync_fetch_and_xor_2,
    __sync_fetch_and_nand_2,
    __sync_fetch_and_max_2,
    __sync_fetch_and_min_2,
    __sync_fetch_and_umax_2,
    __sync_fetch_and_umin_2
);
ops!(
    u32,
    i32,
    __sync_val_compare_and_swap_4,
    __sync_lock_test_and_set_4,
    __sync_fetch_and_add_4,
    __sync_fetch_and_sub_4,
    __sync_fetch_and_and_4,
    __sync_fetch_and_or_4,
    __sync_fetch_and_xor_4,
    __sync_fetch_and_nand_4,
    __sync_fetch_and_max_4,
    __sync_fetch_and_min_4,
    __sync_fetch_and_umax_4,
    __sync_fetch_and_umin_4
);
