//! The third face of a block: the WASM instructions it adds to a sheet module (next to
//! `behavior` for the simulation and `emit_cpp` for the sketch). The module builder in
//! `crab-codegen` implements [`WasmEmit`]; a block only says what to compute.
//!
//! Values are 32 bit as in the numeric model: `bool`/`int` are `i32`, `float` is `f32`.

pub use wasm_encoder::Instruction;

/// Type of a value on the WASM stack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WTy {
    I,
    F,
}

/// What the module builder offers a block while it writes its code.
pub trait WasmEmit {
    /// Type of input `i` (the connected signal, or the default's).
    fn input_ty(&self, i: usize) -> WTy;
    /// Pushes input `i`, converted to `want`.
    fn input_as(&mut self, i: usize, want: WTy);
    /// Appends one instruction.
    fn op(&mut self, i: Instruction<'static>);
    fn c_i(&mut self, x: i32);
    fn c_f(&mut self, x: f32);
    /// Pushes input `i` as an `i32` flag (`1` when it is not zero).
    fn flag(&mut self, i: usize);
    /// Parameter `i` as a number known when the module is built; `Err` when it comes from a
    /// variable or an input.
    fn const_param(&self, i: usize) -> Result<f64, String>;
    /// A global that keeps its value between steps (starts at zero); returns its index for
    /// `GlobalGet`/`GlobalSet`.
    fn state(&mut self, ty: WTy) -> u32;
    /// Takes the value of type `from` from the stack as output `i` (dropped when the output is
    /// not connected anywhere).
    fn set_out(&mut self, i: usize, from: WTy);
    /// A global like `state`, starting at the 32-bit pattern `bits`.
    fn state_init(&mut self, ty: WTy, bits: i32) -> u32;
    /// Text parameter `i` as it is when the module is built.
    fn text_param(&self, i: usize) -> Result<String, String>;
    /// Runs `then` only when the `i32` on the stack is not zero.
    fn when(&mut self, then: &mut dyn FnMut(&mut dyn WasmEmit));
    /// `if (i32 on the stack is not zero) { then } else { other }` with nothing left on the stack.
    fn branch(
        &mut self,
        then: &mut dyn FnMut(&mut dyn WasmEmit),
        other: &mut dyn FnMut(&mut dyn WasmEmit),
    );
    /// Pushes the scan time in milliseconds (an `i32` that wraps).
    fn now(&mut self);
    /// Calls the master's `math(op, a, b)` with the float locals `a` and `b` and leaves the
    /// `f32` result on the stack: `op` 0 sin, 1 cos, 2 tan, 3 pow.
    fn math(&mut self, op: i32, a: u32, b: u32);
    /// The key the block saves its value under: the same on every build of the project.
    fn block_key(&self) -> i32;
    /// Whether output `i` is connected anywhere.
    fn output_connected(&self, i: usize) -> bool;
    /// Pushes the current value of output `i` (it must be connected).
    fn out_get(&mut self, i: usize);
    /// Calls the master's `retain(op, key, value)`: `op` 0 reads (`value` is the default), 1
    /// writes. `value` pushes an `i32`; the answer (an `i32`) is left on the stack.
    fn retain(&mut self, op: i32, key: i32, value: &mut dyn FnMut(&mut dyn WasmEmit));
    /// Pushes parameter `i` converted to `want`: the constant of the block, or the value of the
    /// variable or input the parameter is bound to.
    fn param_as(&mut self, i: usize, want: WTy);
    /// A scratch local of the block; it lives until the end of the block's code.
    fn tmp(&mut self, ty: WTy) -> u32;
    /// `if (cond on the stack) then else`, both branches leave a value of `ty`.
    fn choose(
        &mut self,
        ty: WTy,
        then: &mut dyn FnMut(&mut dyn WasmEmit),
        other: &mut dyn FnMut(&mut dyn WasmEmit),
    );
}

/// A block's code. `Ok(Some(ty))`: the value of output 0 is left on the stack; `Ok(None)`: the
/// block stored its outputs itself with `set_out`. `Err` is the reason the block cannot be
/// written for a module.
pub type WasmFn = fn(&mut dyn WasmEmit) -> Result<Option<WTy>, String>;

/// The module code of the built-in block `kind`, if it has any.
#[must_use]
pub fn get(kind: &str) -> Option<WasmFn> {
    Some(match kind {
        "abs" => abs,
        "sq" => sq,
        "sqrt" => sqrt,
        "number_change" => number_change,
        "scale_table" => scale_table,
        "random" => random,
        "pid" => pid,
        "dms" => dms,
        "decoder" => decoder,
        "encoder" => encoder,
        "sin" => |e| trig(e, 0),
        "cos" => |e| trig(e, 1),
        "tan" => |e| trig(e, 2),
        "pow" => pow,
        "flash_store" => flash_store,
        "cycle_report" => |_| Ok(None),
        "bit_and" => |e| bit_op(e, Instruction::I32And),
        "bit_or" => |e| bit_op(e, Instruction::I32Or),
        "bit_xor" => |e| bit_op(e, Instruction::I32Xor),
        "bit_not" => bit_not,
        "bit_read" => bit_read,
        "bit_write" => bit_write,
        "bit_change" => bit_change,
        "bits_to_byte" => bits_to_byte,
        "byte_to_bits" => byte_to_bits,
        "bytes_to_word" => bytes_to_word,
        "word_to_bytes" => word_to_bytes,
        "bytes_to_dword" => bytes_to_dword,
        "dword_to_bytes" => dword_to_bytes,
        _ => return None,
    })
}

fn abs(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let ty = e.input_ty(0);
    e.input_as(0, ty);
    match ty {
        WTy::F => e.op(Instruction::F32Abs),
        WTy::I => {
            // (x ^ (x >> 31)) - (x >> 31), wrapping like the sketch's integer code.
            let (x, s) = (e.tmp(WTy::I), e.tmp(WTy::I));
            e.op(Instruction::LocalTee(x));
            e.c_i(31);
            e.op(Instruction::I32ShrS);
            e.op(Instruction::LocalSet(s));
            e.op(Instruction::LocalGet(x));
            e.op(Instruction::LocalGet(s));
            e.op(Instruction::I32Xor);
            e.op(Instruction::LocalGet(s));
            e.op(Instruction::I32Sub);
        }
    }
    Ok(Some(ty))
}

fn sq(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let ty = e.input_ty(0);
    let x = e.tmp(ty);
    e.input_as(0, ty);
    e.op(Instruction::LocalTee(x));
    e.op(Instruction::LocalGet(x));
    e.op(if ty == WTy::F {
        Instruction::F32Mul
    } else {
        Instruction::I32Mul
    });
    Ok(Some(ty))
}

/// `x > 0 ? sqrt(x) : 0`, always `float`.
fn sqrt(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let x = e.tmp(WTy::F);
    e.input_as(0, WTy::F);
    e.op(Instruction::LocalTee(x));
    e.c_f(0.0);
    e.op(Instruction::F32Gt);
    e.choose(
        WTy::F,
        &mut |e| {
            e.op(Instruction::LocalGet(x));
            e.op(Instruction::F32Sqrt);
        },
        &mut |e| e.c_f(0.0),
    );
    Ok(Some(WTy::F))
}

fn bit_op(e: &mut dyn WasmEmit, op: Instruction<'static>) -> Result<Option<WTy>, String> {
    e.input_as(0, WTy::I);
    e.input_as(1, WTy::I);
    e.op(op);
    Ok(Some(WTy::I))
}

fn bit_not(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    e.input_as(0, WTy::I);
    e.c_i(-1);
    e.op(Instruction::I32Xor);
    Ok(Some(WTy::I))
}

/// Bit `bit` of input 0 as `0`/`1` (the shift count wraps at 32, as in the sketch).
fn read_bit(e: &mut dyn WasmEmit) -> Result<(), String> {
    e.input_as(0, WTy::I);
    e.param_as(0, WTy::I);
    e.op(Instruction::I32ShrS);
    e.c_i(1);
    e.op(Instruction::I32And);
    Ok(())
}

fn bit_read(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    read_bit(e)?;
    Ok(Some(WTy::I))
}

fn bit_write(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let (x, mask) = (e.tmp(WTy::I), e.tmp(WTy::I));
    e.c_i(1);
    e.param_as(0, WTy::I);
    e.op(Instruction::I32Shl);
    e.op(Instruction::LocalSet(mask));
    e.input_as(0, WTy::I);
    e.op(Instruction::LocalSet(x));
    e.flag(1);
    e.choose(
        WTy::I,
        &mut |e| {
            e.op(Instruction::LocalGet(x));
            e.op(Instruction::LocalGet(mask));
            e.op(Instruction::I32Or);
        },
        &mut |e| {
            e.op(Instruction::LocalGet(x));
            e.op(Instruction::LocalGet(mask));
            e.c_i(-1);
            e.op(Instruction::I32Xor);
            e.op(Instruction::I32And);
        },
    );
    Ok(Some(WTy::I))
}

/// `1` for one scan when the bit differs from the previous scan (not on the first one).
fn bit_change(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let (prev, init) = (e.state(WTy::I), e.state(WTy::I));
    let now = e.tmp(WTy::I);
    read_bit(e)?;
    e.op(Instruction::LocalSet(now));
    e.op(Instruction::GlobalGet(init));
    e.op(Instruction::LocalGet(now));
    e.op(Instruction::GlobalGet(prev));
    e.op(Instruction::I32Ne);
    e.op(Instruction::I32And);
    e.op(Instruction::LocalGet(now));
    e.op(Instruction::GlobalSet(prev));
    e.c_i(1);
    e.op(Instruction::GlobalSet(init));
    Ok(Some(WTy::I))
}

fn bits_to_byte(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    e.c_i(0);
    for i in 0..8 {
        e.flag(i);
        e.c_i(1 << i);
        e.op(Instruction::I32Mul);
        e.op(Instruction::I32Or);
    }
    Ok(Some(WTy::I))
}

fn byte_to_bits(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let x = e.tmp(WTy::I);
    e.input_as(0, WTy::I);
    e.op(Instruction::LocalSet(x));
    for i in 0..8 {
        e.op(Instruction::LocalGet(x));
        e.c_i(i as i32);
        e.op(Instruction::I32ShrS);
        e.c_i(1);
        e.op(Instruction::I32And);
        e.set_out(i, WTy::I);
    }
    Ok(None)
}

/// Input `i` cut to a byte.
fn byte_of(e: &mut dyn WasmEmit, i: usize) {
    e.input_as(i, WTy::I);
    e.c_i(0xFF);
    e.op(Instruction::I32And);
}

fn bytes_to_word(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    byte_of(e, 1);
    e.c_i(8);
    e.op(Instruction::I32Shl);
    byte_of(e, 0);
    e.op(Instruction::I32Or);
    Ok(Some(WTy::I))
}

fn word_to_bytes(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let x = e.tmp(WTy::I);
    e.input_as(0, WTy::I);
    e.op(Instruction::LocalSet(x));
    for (i, shift) in [(0, 0), (1, 8)] {
        e.op(Instruction::LocalGet(x));
        e.c_i(shift);
        e.op(Instruction::I32ShrS);
        e.c_i(0xFF);
        e.op(Instruction::I32And);
        e.set_out(i, WTy::I);
    }
    Ok(None)
}

fn bytes_to_dword(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    e.c_i(0);
    for i in 0..4 {
        byte_of(e, i);
        e.c_i(8 * i as i32);
        e.op(Instruction::I32Shl);
        e.op(Instruction::I32Or);
    }
    Ok(Some(WTy::I))
}

fn dword_to_bytes(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let x = e.tmp(WTy::I);
    e.input_as(0, WTy::I);
    e.op(Instruction::LocalSet(x));
    for i in 0..4 {
        e.op(Instruction::LocalGet(x));
        e.c_i(8 * i as i32);
        e.op(Instruction::I32ShrU);
        e.c_i(0xFF);
        e.op(Instruction::I32And);
        e.set_out(i, WTy::I);
    }
    Ok(None)
}

/// `1` when the input moved by more than the dead band since the value last reported; the first
/// scan only remembers the input.
fn number_change(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let (prev, init) = (e.state(WTy::F), e.state(WTy::I));
    let (x, hit, band) = (e.tmp(WTy::F), e.tmp(WTy::I), e.tmp(WTy::F));
    e.param_as(0, WTy::F);
    e.op(Instruction::LocalSet(band));
    e.input_as(0, WTy::F);
    e.op(Instruction::LocalSet(x));
    // hit = init && |x - prev| > band
    e.op(Instruction::GlobalGet(init));
    e.op(Instruction::LocalGet(x));
    e.op(Instruction::GlobalGet(prev));
    e.op(Instruction::F32Sub);
    e.op(Instruction::F32Abs);
    e.op(Instruction::LocalGet(band));
    e.op(Instruction::F32Gt);
    e.op(Instruction::I32And);
    e.op(Instruction::LocalSet(hit));
    // remember the input on the first scan and after every report
    e.op(Instruction::GlobalGet(init));
    e.op(Instruction::I32Eqz);
    e.op(Instruction::LocalGet(hit));
    e.op(Instruction::I32Or);
    e.when(&mut |e| {
        e.op(Instruction::LocalGet(x));
        e.op(Instruction::GlobalSet(prev));
    });
    e.c_i(1);
    e.op(Instruction::GlobalSet(init));
    e.op(Instruction::LocalGet(hit));
    Ok(Some(WTy::I))
}

/// Piecewise linear curve through the points of the text parameter, flat outside them.
fn scale_table(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let points = crate::more::parse_points(&e.text_param(0)?);
    if points.len() < 2 {
        return Err("в таблице нужно не меньше двух точек «x:y»".into());
    }
    let v = e.tmp(WTy::F);
    e.input_as(0, WTy::F);
    e.op(Instruction::LocalSet(v));
    let last = points.len() - 1;
    let (first, end) = (points[0], points[last]);
    e.op(Instruction::LocalGet(v));
    e.c_f(first.0);
    e.op(Instruction::F32Le);
    e.choose(WTy::F, &mut |e| e.c_f(first.1), &mut |e| {
        e.op(Instruction::LocalGet(v));
        e.c_f(end.0);
        e.op(Instruction::F32Ge);
        e.choose(WTy::F, &mut |e| e.c_f(end.1), &mut |e| {
            segments(e, v, &points, 1)
        });
    });
    Ok(Some(WTy::F))
}

/// The segment that ends at the first point at or after `v`, starting the search at point `k`.
fn segments(e: &mut dyn WasmEmit, v: u32, points: &[(f32, f32)], k: usize) {
    let (a, b) = (points[k - 1], points[k]);
    let interpolate = |e: &mut dyn WasmEmit| {
        e.c_f(a.1);
        e.c_f(b.1 - a.1);
        e.op(Instruction::LocalGet(v));
        e.c_f(a.0);
        e.op(Instruction::F32Sub);
        e.op(Instruction::F32Mul);
        e.c_f(b.0 - a.0);
        e.op(Instruction::F32Div);
        e.op(Instruction::F32Add);
    };
    if k == points.len() - 1 {
        interpolate(e);
        return;
    }
    e.op(Instruction::LocalGet(v));
    e.c_f(b.0);
    e.op(Instruction::F32Le);
    e.choose(WTy::F, &mut |e| interpolate(e), &mut |e| {
        segments(e, v, points, k + 1)
    });
}

/// A new number between `min` and `max` on every rising edge of the input; the generator is the
/// same xorshift as in the simulation.
fn random(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let state = e.state_init(WTy::I, crate::RANDOM_SEED as i32);
    let prev = e.state(WTy::I);
    let (en, lo, hi) = (e.tmp(WTy::I), e.tmp(WTy::F), e.tmp(WTy::F));
    e.param_as(0, WTy::F);
    e.op(Instruction::LocalSet(lo));
    e.param_as(1, WTy::F);
    e.op(Instruction::LocalSet(hi));
    e.flag(0);
    e.op(Instruction::LocalSet(en));
    e.op(Instruction::LocalGet(en));
    e.op(Instruction::GlobalGet(prev));
    e.op(Instruction::I32Eqz);
    e.op(Instruction::I32And);
    e.when(&mut |e| {
        for (op, by) in [
            (Instruction::I32Shl, 13),
            (Instruction::I32ShrU, 17),
            (Instruction::I32Shl, 5),
        ] {
            e.op(Instruction::GlobalGet(state));
            e.op(Instruction::GlobalGet(state));
            e.c_i(by);
            e.op(op);
            e.op(Instruction::I32Xor);
            e.op(Instruction::GlobalSet(state));
        }
        e.op(Instruction::LocalGet(lo));
        e.op(Instruction::LocalGet(hi));
        e.op(Instruction::LocalGet(lo));
        e.op(Instruction::F32Sub);
        e.op(Instruction::GlobalGet(state));
        e.c_i(10_001);
        e.op(Instruction::I32RemU);
        e.op(Instruction::F32ConvertI32U);
        e.c_f(10_000.0);
        e.op(Instruction::F32Div);
        e.op(Instruction::F32Mul);
        e.op(Instruction::F32Add);
        e.set_out(0, WTy::F);
    });
    e.op(Instruction::LocalGet(en));
    e.op(Instruction::GlobalSet(prev));
    Ok(None)
}

/// Arduino `constrain` of the float in local `x`: no panic when the limits are swapped.
fn constrain(e: &mut dyn WasmEmit, x: u32, lo: u32, hi: u32) {
    e.op(Instruction::LocalGet(x));
    e.op(Instruction::LocalGet(lo));
    e.op(Instruction::F32Lt);
    e.choose(WTy::F, &mut |e| e.op(Instruction::LocalGet(lo)), &mut |e| {
        e.op(Instruction::LocalGet(x));
        e.op(Instruction::LocalGet(hi));
        e.op(Instruction::F32Gt);
        e.choose(WTy::F, &mut |e| e.op(Instruction::LocalGet(hi)), &mut |e| {
            e.op(Instruction::LocalGet(x))
        });
    });
}

/// The PID of the sketch in 32 bit floats: integral limited to the output range, derivative on
/// the error, time from the scan clock.
fn pid(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let (kp, ki, kd) = (e.tmp(WTy::F), e.tmp(WTy::F), e.tmp(WTy::F));
    let (lo, hi) = (e.tmp(WTy::F), e.tmp(WTy::F));
    for (i, local) in [kp, ki, kd, lo, hi].into_iter().enumerate() {
        e.param_as(i, WTy::F);
        e.op(Instruction::LocalSet(local));
    }
    let (integral, prev_err) = (e.state(WTy::F), e.state(WTy::F));
    let (last, running) = (e.state(WTy::I), e.state(WTy::I));
    let (err, dt, tmp) = (e.tmp(WTy::F), e.tmp(WTy::F), e.tmp(WTy::F));
    e.flag(2);
    e.branch(
        &mut |e| {
            e.input_as(0, WTy::F);
            e.input_as(1, WTy::F);
            e.op(Instruction::F32Sub);
            e.op(Instruction::LocalSet(err));
            e.op(Instruction::GlobalGet(running));
            e.branch(
                &mut |e| {
                    e.now();
                    e.op(Instruction::GlobalGet(last));
                    e.op(Instruction::I32Sub);
                    e.op(Instruction::F32ConvertI32U);
                    e.c_f(1000.0);
                    e.op(Instruction::F32Div);
                    e.op(Instruction::LocalSet(dt));
                    e.op(Instruction::LocalGet(dt));
                    e.c_f(0.0);
                    e.op(Instruction::F32Gt);
                    e.when(&mut |e| {
                        e.op(Instruction::GlobalGet(integral));
                        e.op(Instruction::LocalGet(err));
                        e.op(Instruction::LocalGet(dt));
                        e.op(Instruction::F32Mul);
                        e.op(Instruction::LocalGet(ki));
                        e.op(Instruction::F32Mul);
                        e.op(Instruction::F32Add);
                        e.op(Instruction::LocalSet(tmp));
                        constrain(e, tmp, lo, hi);
                        e.op(Instruction::GlobalSet(integral));
                        e.op(Instruction::LocalGet(kp));
                        e.op(Instruction::LocalGet(err));
                        e.op(Instruction::F32Mul);
                        e.op(Instruction::GlobalGet(integral));
                        e.op(Instruction::F32Add);
                        e.op(Instruction::LocalGet(kd));
                        e.op(Instruction::LocalGet(err));
                        e.op(Instruction::GlobalGet(prev_err));
                        e.op(Instruction::F32Sub);
                        e.op(Instruction::LocalGet(dt));
                        e.op(Instruction::F32Div);
                        e.op(Instruction::F32Mul);
                        e.op(Instruction::F32Add);
                        e.op(Instruction::LocalSet(tmp));
                        constrain(e, tmp, lo, hi);
                        e.set_out(0, WTy::F);
                        e.op(Instruction::LocalGet(err));
                        e.op(Instruction::GlobalSet(prev_err));
                        e.now();
                        e.op(Instruction::GlobalSet(last));
                    });
                },
                &mut |e| {
                    e.c_i(1);
                    e.op(Instruction::GlobalSet(running));
                    e.now();
                    e.op(Instruction::GlobalSet(last));
                    e.op(Instruction::LocalGet(err));
                    e.op(Instruction::GlobalSet(prev_err));
                    e.op(Instruction::LocalGet(kp));
                    e.op(Instruction::LocalGet(err));
                    e.op(Instruction::F32Mul);
                    e.op(Instruction::LocalSet(tmp));
                    constrain(e, tmp, lo, hi);
                    e.set_out(0, WTy::F);
                },
            );
        },
        &mut |e| {
            e.c_i(0);
            e.op(Instruction::GlobalSet(running));
            e.c_f(0.0);
            e.op(Instruction::GlobalSet(integral));
            e.op(Instruction::LocalGet(lo));
            e.set_out(0, WTy::F);
        },
    );
    Ok(None)
}

/// Number of outputs the `count` parameter turns on (at most 16).
fn count_param(e: &dyn WasmEmit) -> Result<usize, String> {
    Ok((e.const_param(0)? as i64).clamp(0, 16) as usize)
}

/// Input 1 goes to the output chosen by input 0, the others get zero.
fn dms(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let count = count_param(e)?;
    let ty = e.input_ty(1);
    let (sel, x) = (e.tmp(WTy::I), e.tmp(ty));
    e.input_as(0, WTy::I);
    e.op(Instruction::LocalSet(sel));
    e.input_as(1, ty);
    e.op(Instruction::LocalSet(x));
    for i in 0..count {
        e.op(Instruction::LocalGet(x));
        if ty == WTy::F {
            e.c_f(0.0);
        } else {
            e.c_i(0);
        }
        e.op(Instruction::LocalGet(sel));
        e.c_i(i as i32);
        e.op(Instruction::I32Eq);
        e.op(Instruction::Select);
        e.set_out(i, ty);
    }
    Ok(None)
}

/// Output `o<n>` is on while the input equals `n`.
fn decoder(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let count = count_param(e)?;
    let x = e.tmp(WTy::I);
    e.input_as(0, WTy::I);
    e.op(Instruction::LocalSet(x));
    for i in 0..count {
        e.op(Instruction::LocalGet(x));
        e.c_i(i as i32);
        e.op(Instruction::I32Eq);
        e.set_out(i, WTy::I);
    }
    Ok(None)
}

/// The lowest set input gives `out`; `any` tells that some input is set.
fn encoder(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let count = count_param(e)?;
    let acc = e.tmp(WTy::I);
    e.c_i(0);
    for i in (0..count).rev() {
        e.op(Instruction::LocalSet(acc));
        e.c_i(i as i32);
        e.op(Instruction::LocalGet(acc));
        e.flag(i);
        e.op(Instruction::Select);
    }
    e.set_out(0, WTy::I);
    e.c_i(0);
    for i in 0..count {
        e.flag(i);
        e.op(Instruction::I32Or);
    }
    e.set_out(1, WTy::I);
    Ok(None)
}

/// `sinf`, `cosf` or `tanf` of the input, in degrees when the unit parameter is 0.
fn trig(e: &mut dyn WasmEmit, op: i32) -> Result<Option<WTy>, String> {
    let x = e.tmp(WTy::F);
    e.input_as(0, WTy::F);
    e.op(Instruction::LocalSet(x));
    // Unit 0 is degrees.
    e.param_as(0, WTy::I);
    e.op(Instruction::I32Eqz);
    e.choose(
        WTy::F,
        &mut |e| {
            e.op(Instruction::LocalGet(x));
            e.c_f(0.017_453_292);
            e.op(Instruction::F32Mul);
        },
        &mut |e| e.op(Instruction::LocalGet(x)),
    );
    e.op(Instruction::LocalSet(x));
    e.math(op, x, x);
    Ok(Some(WTy::F))
}

/// `powf(a, b)`.
fn pow(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    let (a, b) = (e.tmp(WTy::F), e.tmp(WTy::F));
    e.input_as(0, WTy::F);
    e.op(Instruction::LocalSet(a));
    e.input_as(1, WTy::F);
    e.op(Instruction::LocalSet(b));
    e.math(3, a, b);
    Ok(Some(WTy::F))
}

/// A value kept in flash: read at start, written on the rising edge of `save` when it differs.
fn flash_store(e: &mut dyn WasmEmit) -> Result<Option<WTy>, String> {
    if !e.output_connected(0) {
        return Ok(None);
    }
    let initial = (e.const_param(0)? as f32).to_bits() as i32;
    let key = e.block_key();
    let (started, prev) = (e.state(WTy::I), e.state(WTy::I));
    e.op(Instruction::GlobalGet(started));
    e.op(Instruction::I32Eqz);
    e.when(&mut |e| {
        e.retain(0, key, &mut |e| e.c_i(initial));
        e.op(Instruction::F32ReinterpretI32);
        e.set_out(0, WTy::F);
        e.c_i(1);
        e.op(Instruction::GlobalSet(started));
    });
    e.flag(1);
    e.op(Instruction::GlobalGet(prev));
    e.op(Instruction::I32Eqz);
    e.op(Instruction::I32And);
    e.input_as(0, WTy::F);
    e.out_get(0);
    e.op(Instruction::F32Ne);
    e.op(Instruction::I32And);
    e.when(&mut |e| {
        e.input_as(0, WTy::F);
        e.set_out(0, WTy::F);
        e.retain(1, key, &mut |e| {
            e.input_as(0, WTy::F);
            e.op(Instruction::I32ReinterpretF32);
        });
        e.op(Instruction::Drop);
    });
    e.flag(1);
    e.op(Instruction::GlobalSet(prev));
    Ok(None)
}
