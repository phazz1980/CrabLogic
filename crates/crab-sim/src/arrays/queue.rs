//! Shift register, ring buffer and stack (`array_shift`, `array_ring`, `array_stack`). All three
//! act on rising edges of their flag inputs and remember the previous level themselves.

use crab_core::array_blocks::ArrayOp;
use crab_core::arrays::ElemType;

use super::{ArrayBehavior, ArrayState, Items, Noop, OpCtx};

/// Rising edge of `now` against the remembered level `prev` (which is updated).
fn rising(prev: &mut bool, now: f64) -> bool {
    let level = now != 0.0;
    let edge = level && !*prev;
    *prev = level;
    edge
}

/// Writes the `value` input (port 0) into element `i`; an index outside the array is skipped.
fn put(arr: &mut ArrayState, i: usize, ctx: &OpCtx) {
    if arr.is_text() {
        arr.set_text(i, &ctx.texts[0]);
    } else {
        arr.set_num(i, ctx.inputs[0]);
    }
}

fn flag(cond: bool) -> f64 {
    f64::from(u8::from(cond))
}

/// `array_shift`: in 0 = value, 1 = go.
#[derive(Default)]
struct Shift {
    prev_go: bool,
}

impl ArrayBehavior for Shift {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        _: &mut [f64],
        _: &mut [String],
    ) {
        if !rising(&mut self.prev_go, ctx.inputs[1]) || arr.is_empty() {
            return;
        }
        match &mut arr.items {
            Items::Num(v) => v.rotate_right(1),
            Items::Text(v) => v.rotate_right(1),
        }
        put(arr, 0, ctx);
    }
}

/// `array_ring`: in 0 = value, 1 = push; out 0 = head, 1 = count.
#[derive(Default)]
struct Ring {
    prev_push: bool,
    head: usize,
    count: usize,
}

impl ArrayBehavior for Ring {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        _: &mut [String],
    ) {
        let len = arr.len();
        if rising(&mut self.prev_push, ctx.inputs[1]) && len > 0 {
            put(arr, self.head, ctx);
            self.head = (self.head + 1) % len;
            self.count = (self.count + 1).min(len);
        }
        out[0] = self.head as f64;
        out[1] = self.count as f64;
    }
}

/// `array_stack`: in 0 = value, 1 = push, 2 = pop; out 0 = out, 1 = count, 2 = empty, 3 = full,
/// 4 = err.
#[derive(Default)]
struct Stack {
    prev_push: bool,
    prev_pop: bool,
    count: usize,
}

impl ArrayBehavior for Stack {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        texts_out: &mut [String],
    ) {
        let len = arr.len();
        let push = rising(&mut self.prev_push, ctx.inputs[1]);
        let pop = rising(&mut self.prev_pop, ctx.inputs[2]);
        let mut err = false;
        if push {
            if self.count < len {
                put(arr, self.count, ctx);
                self.count += 1;
            } else {
                err = true;
            }
        }
        if pop {
            if self.count > 0 {
                self.count -= 1;
            } else {
                err = true;
            }
        }
        if arr.is_text() {
            texts_out[0] = self
                .count
                .checked_sub(1)
                .map_or("", |top| arr.text(top))
                .to_owned();
        } else {
            out[0] = self.count.checked_sub(1).map_or(0.0, |top| arr.num(top));
        }
        out[1] = self.count as f64;
        out[2] = flag(self.count == 0);
        out[3] = flag(self.count >= len);
        out[4] = flag(err);
    }
}

/// Behaviour of `Shift`, `Ring` and `Stack`.
pub(super) fn behavior(op: ArrayOp, _ty: ElemType) -> Box<dyn ArrayBehavior> {
    match op {
        ArrayOp::Shift => Box::new(Shift::default()),
        ArrayOp::Ring => Box::new(Ring::default()),
        ArrayOp::Stack => Box::new(Stack::default()),
        _ => Box::new(Noop),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::arrays::ArrayVar;

    fn array(ty: ElemType, len: u32) -> ArrayState {
        ArrayState::new(&ArrayVar {
            name: "a".into(),
            ty,
            len,
            items: Vec::new(),
            description: String::new(),
        })
    }

    /// One scan of `b` with the given inputs; returns (numeric outputs, string outputs).
    fn scan(
        b: &mut dyn ArrayBehavior,
        arr: &mut ArrayState,
        inputs: &[f64],
        text: &str,
        n_out: usize,
    ) -> (Vec<f64>, Vec<String>) {
        let mut in_texts = vec![String::new(); inputs.len()];
        in_texts[0] = text.to_owned();
        let (mut out, mut texts_out) = (vec![0.0; n_out], vec![String::new(); n_out]);
        b.step(
            arr,
            None,
            &OpCtx {
                inputs,
                texts: &in_texts,
            },
            &mut out,
            &mut texts_out,
        );
        (out, texts_out)
    }

    fn nums(a: &ArrayState) -> Vec<f64> {
        (0..a.len()).map(|i| a.num(i)).collect()
    }

    fn strings(a: &ArrayState) -> Vec<String> {
        (0..a.len()).map(|i| a.text(i).to_owned()).collect()
    }

    #[test]
    fn shift_acts_on_rising_edges_only() {
        let mut arr = array(ElemType::Int, 4);
        let mut b = Shift::default();
        for (v, go) in [(1.0, 1.0), (2.0, 1.0), (2.0, 0.0), (3.0, 1.0)] {
            scan(&mut b, &mut arr, &[v, go], "", 0);
        }
        // The held `go` acted once (value 1); the new edge took value 3.
        assert_eq!(nums(&arr), vec![3.0, 1.0, 0.0, 0.0]);
        for v in [4.0, 5.0, 6.0] {
            scan(&mut b, &mut arr, &[v, 0.0], "", 0);
            scan(&mut b, &mut arr, &[v, 1.0], "", 0);
        }
        assert_eq!(nums(&arr), vec![6.0, 5.0, 4.0, 3.0]);
    }

    #[test]
    fn shift_stores_as_the_type_and_handles_empty() {
        let mut arr = array(ElemType::Byte, 2);
        let mut b = Shift::default();
        scan(&mut b, &mut arr, &[257.0, 1.0], "", 0);
        assert_eq!(nums(&arr), vec![1.0, 0.0]);
        let mut empty = array(ElemType::Int, 0);
        let mut b = Shift::default();
        scan(&mut b, &mut empty, &[1.0, 1.0], "", 0);
        assert!(empty.is_empty());
    }

    #[test]
    fn shift_of_strings() {
        let mut arr = array(ElemType::String, 3);
        let mut b = Shift::default();
        for s in ["a", "b", "c", "d"] {
            scan(&mut b, &mut arr, &[0.0, 0.0], s, 0);
            scan(&mut b, &mut arr, &[0.0, 1.0], s, 0);
        }
        assert_eq!(strings(&arr), vec!["d", "c", "b"]);
    }

    #[test]
    fn ring_wraps_and_overwrites_the_oldest() {
        let mut arr = array(ElemType::Int, 3);
        let mut b = Ring::default();
        let mut seen = Vec::new();
        for v in 1..=5 {
            scan(&mut b, &mut arr, &[f64::from(v), 0.0], "", 2);
            let (out, _) = scan(&mut b, &mut arr, &[f64::from(v), 1.0], "", 2);
            seen.push((out[0], out[1]));
        }
        assert_eq!(nums(&arr), vec![4.0, 5.0, 3.0]);
        assert_eq!(
            seen,
            vec![(1.0, 1.0), (2.0, 2.0), (0.0, 3.0), (1.0, 3.0), (2.0, 3.0)]
        );
        // A held push writes nothing more.
        let (held, _) = scan(&mut b, &mut arr, &[9.0, 1.0], "", 2);
        assert_eq!(nums(&arr), vec![4.0, 5.0, 3.0]);
        assert_eq!((held[0], held[1]), (2.0, 3.0));
    }

    #[test]
    fn ring_of_strings_and_empty_ring() {
        let mut arr = array(ElemType::String, 2);
        let mut b = Ring::default();
        for s in ["x", "y", "z"] {
            scan(&mut b, &mut arr, &[0.0, 0.0], s, 2);
            scan(&mut b, &mut arr, &[0.0, 1.0], s, 2);
        }
        assert_eq!(strings(&arr), vec!["z", "y"]);
        let mut empty = array(ElemType::Int, 0);
        let mut b = Ring::default();
        let (out, _) = scan(&mut b, &mut empty, &[1.0, 1.0], "", 2);
        assert_eq!((out[0], out[1]), (0.0, 0.0));
    }

    /// One stack scan; outputs (out, count, empty, full, err).
    fn st(b: &mut Stack, arr: &mut ArrayState, v: f64, push: bool, pop: bool) -> [f64; 5] {
        let (out, _) = scan(b, arr, &[v, flag(push), flag(pop)], "", 5);
        [out[0], out[1], out[2], out[3], out[4]]
    }

    #[test]
    fn stack_push_pop_full_empty_err() {
        let mut arr = array(ElemType::Int, 2);
        let mut b = Stack::default();
        let idle = |b: &mut Stack, arr: &mut ArrayState| st(b, arr, 0.0, false, false);
        assert_eq!(idle(&mut b, &mut arr), [0.0, 0.0, 1.0, 0.0, 0.0]);
        // Pop on an empty stack: err.
        assert_eq!(
            st(&mut b, &mut arr, 0.0, false, true),
            [0.0, 0.0, 1.0, 0.0, 1.0]
        );
        idle(&mut b, &mut arr);
        assert_eq!(
            st(&mut b, &mut arr, 7.0, true, false),
            [7.0, 1.0, 0.0, 0.0, 0.0]
        );
        // A held push does nothing; err is false.
        assert_eq!(
            st(&mut b, &mut arr, 8.0, true, false),
            [7.0, 1.0, 0.0, 0.0, 0.0]
        );
        idle(&mut b, &mut arr);
        assert_eq!(
            st(&mut b, &mut arr, 8.0, true, false),
            [8.0, 2.0, 0.0, 1.0, 0.0]
        );
        idle(&mut b, &mut arr);
        // Full: refused.
        assert_eq!(
            st(&mut b, &mut arr, 9.0, true, false),
            [8.0, 2.0, 0.0, 1.0, 1.0]
        );
        assert_eq!(nums(&arr), vec![7.0, 8.0]);
        idle(&mut b, &mut arr);
        assert_eq!(
            st(&mut b, &mut arr, 0.0, false, true),
            [7.0, 1.0, 0.0, 0.0, 0.0]
        );
        idle(&mut b, &mut arr);
        assert_eq!(
            st(&mut b, &mut arr, 0.0, false, true),
            [0.0, 0.0, 1.0, 0.0, 0.0]
        );
        // Popped elements stay in the array.
        assert_eq!(nums(&arr), vec![7.0, 8.0]);
    }

    #[test]
    fn stack_push_comes_before_pop_in_one_scan() {
        let mut arr = array(ElemType::Int, 2);
        let mut b = Stack::default();
        // Empty stack: push then pop leaves it empty, no error.
        assert_eq!(
            st(&mut b, &mut arr, 5.0, true, true),
            [0.0, 0.0, 1.0, 0.0, 0.0]
        );
        st(&mut b, &mut arr, 0.0, false, false);
        st(&mut b, &mut arr, 1.0, true, false);
        st(&mut b, &mut arr, 0.0, false, false);
        st(&mut b, &mut arr, 2.0, true, false);
        st(&mut b, &mut arr, 0.0, false, false);
        // Full stack: the push is refused (err), the pop is done.
        assert_eq!(
            st(&mut b, &mut arr, 3.0, true, true),
            [1.0, 1.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn stack_of_strings_and_zero_length() {
        let mut arr = array(ElemType::String, 2);
        let mut b = Stack::default();
        let (out, t) = scan(&mut b, &mut arr, &[0.0, 1.0, 0.0], "hi", 5);
        assert_eq!((t[0].as_str(), out[1]), ("hi", 1.0));
        scan(&mut b, &mut arr, &[0.0, 0.0, 0.0], "", 5);
        let (_, t) = scan(&mut b, &mut arr, &[0.0, 1.0, 0.0], "yo", 5);
        assert_eq!(t[0], "yo");
        scan(&mut b, &mut arr, &[0.0, 0.0, 0.0], "", 5);
        let (_, t) = scan(&mut b, &mut arr, &[0.0, 0.0, 1.0], "", 5);
        assert_eq!(t[0], "hi");
        scan(&mut b, &mut arr, &[0.0, 0.0, 0.0], "", 5);
        let (_, t) = scan(&mut b, &mut arr, &[0.0, 0.0, 1.0], "", 5);
        assert_eq!(t[0], "");

        let mut zero = array(ElemType::Int, 0);
        let mut b = Stack::default();
        assert_eq!(
            st(&mut b, &mut zero, 1.0, true, false),
            [0.0, 0.0, 1.0, 1.0, 1.0]
        );
    }
}
