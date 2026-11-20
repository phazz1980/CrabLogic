//! Fill, copy, sum, minimum, maximum and search.

use crab_core::array_blocks::ArrayOp;
use crab_core::arrays::ElemType;

use super::{ArrayBehavior, ArrayState, Items, Noop, OpCtx};

/// Behaviour of `Fill`, `Copy`, `Sum`, `Min`, `Max` and `Find`.
pub(super) fn behavior(op: ArrayOp, ty: ElemType) -> Box<dyn ArrayBehavior> {
    match op {
        ArrayOp::Fill => Box::new(Fill),
        ArrayOp::Copy => Box::new(Copy { prev_go: false }),
        ArrayOp::Sum => Box::new(Sum),
        ArrayOp::Min => Box::new(Extreme { max: false }),
        ArrayOp::Max => Box::new(Extreme { max: true }),
        ArrayOp::Find => Box::new(Find {
            // One-element scratch array of the same type: the searched value goes through
            // `set_num`, so it is compared in the form the elements are stored in.
            probe: ArrayState {
                ty,
                items: Items::Num(vec![0.0]),
            },
        }),
        _ => Box::new(Noop),
    }
}

/// `array_fill`: in 0 = value, 1 = go. While `go` is true every element gets the value.
struct Fill;

impl ArrayBehavior for Fill {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        _: &mut [f64],
        _: &mut [String],
    ) {
        if ctx.inputs[1] == 0.0 {
            return;
        }
        for i in 0..arr.len() {
            if arr.is_text() {
                arr.set_text(i, &ctx.texts[0]);
            } else {
                arr.set_num(i, ctx.inputs[0]);
            }
        }
    }
}

/// `array_copy`: in 0 = go (rising edge), out 0 = err (no destination).
struct Copy {
    prev_go: bool,
}

impl ArrayBehavior for Copy {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        dest: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        _: &mut [String],
    ) {
        let go = ctx.inputs[0] != 0.0;
        let rising = go && !self.prev_go;
        self.prev_go = go;
        out[0] = f64::from(u8::from(dest.is_none()));
        if let (true, Some(dest)) = (rising, dest) {
            for i in 0..arr.len().min(dest.len()) {
                dest.copy_item(i, arr, i);
            }
        }
    }
}

/// `array_sum`: out 0 = sum. Int and byte wrap at 32 bits, float accumulates in `f32`.
struct Sum;

impl ArrayBehavior for Sum {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        _: &OpCtx,
        out: &mut [f64],
        _: &mut [String],
    ) {
        let Items::Num(v) = &arr.items else {
            return;
        };
        out[0] = if arr.ty == ElemType::Float {
            f64::from(v.iter().fold(0.0f32, |acc, x| acc + *x as f32))
        } else {
            f64::from(v.iter().fold(0i32, |acc, x| acc.wrapping_add(*x as i32)))
        };
    }
}

/// `array_min` / `array_max`: out 0 = value, out 1 = index of its first occurrence.
struct Extreme {
    max: bool,
}

impl ArrayBehavior for Extreme {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        _: &OpCtx,
        out: &mut [f64],
        _: &mut [String],
    ) {
        let Items::Num(v) = &arr.items else {
            return;
        };
        let mut best = 0;
        for (i, x) in v.iter().enumerate() {
            // Strict comparison keeps the first occurrence.
            let better = if self.max { *x > v[best] } else { *x < v[best] };
            if better {
                best = i;
            }
        }
        out[0] = v.get(best).copied().unwrap_or(0.0);
        out[1] = best as f64;
    }
}

/// `array_find`: in 0 = value; out 0 = index of the first equal element or -1, out 1 = found.
struct Find {
    probe: ArrayState,
}

impl ArrayBehavior for Find {
    fn step(
        &mut self,
        arr: &mut ArrayState,
        _: Option<&mut ArrayState>,
        ctx: &OpCtx,
        out: &mut [f64],
        _: &mut [String],
    ) {
        let at = if arr.is_text() {
            (0..arr.len()).find(|&i| arr.text(i) == ctx.texts[0])
        } else {
            self.probe.set_num(0, ctx.inputs[0]);
            let wanted = self.probe.num(0);
            (0..arr.len()).find(|&i| arr.num(i) == wanted)
        };
        out[0] = at.map_or(-1.0, |i| i as f64);
        out[1] = f64::from(u8::from(at.is_some()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crab_core::arrays::ArrayVar;
    use crab_core::ParamValue;

    fn arr(ty: ElemType, len: u32, items: Vec<ParamValue>) -> ArrayState {
        ArrayState::new(&ArrayVar {
            name: "a".into(),
            ty,
            len,
            items,
            description: String::new(),
        })
    }

    fn ints(ty: ElemType, v: &[i64]) -> ArrayState {
        arr(
            ty,
            v.len() as u32,
            v.iter().map(|n| ParamValue::Int(*n)).collect(),
        )
    }

    /// Runs one scan; returns the outputs (3 slots).
    fn run(
        b: &mut dyn ArrayBehavior,
        a: &mut ArrayState,
        dest: Option<&mut ArrayState>,
        inputs: &[f64],
        texts: &[String],
    ) -> [f64; 3] {
        let mut out = [0.0; 3];
        let mut texts_out = vec![String::new(); 2];
        b.step(a, dest, &OpCtx { inputs, texts }, &mut out, &mut texts_out);
        out
    }

    fn no_texts() -> Vec<String> {
        vec![String::new(); 2]
    }

    #[test]
    fn fill_writes_while_go_is_on() {
        let mut a = ints(ElemType::Byte, &[1, 2, 3]);
        let mut b = behavior(ArrayOp::Fill, ElemType::Byte);
        run(&mut *b, &mut a, None, &[9.0, 0.0], &no_texts());
        assert_eq!(a.items, Items::Num(vec![1.0, 2.0, 3.0]));
        run(&mut *b, &mut a, None, &[257.0, 1.0], &no_texts());
        assert_eq!(a.items, Items::Num(vec![1.0, 1.0, 1.0]));
    }

    #[test]
    fn fill_strings() {
        let mut a = arr(ElemType::String, 2, Vec::new());
        let mut b = behavior(ArrayOp::Fill, ElemType::String);
        let texts = vec!["ab".to_owned(), String::new()];
        run(&mut *b, &mut a, None, &[0.0, 1.0], &texts);
        assert_eq!((a.text(0), a.text(1)), ("ab", "ab"));
    }

    #[test]
    fn copy_on_the_rising_edge_by_the_shorter_length() {
        let mut src = ints(ElemType::Int, &[1, 2, 3, 4]);
        let mut dest = ints(ElemType::Int, &[0, 0]);
        let mut b = behavior(ArrayOp::Copy, ElemType::Int);
        let o = run(&mut *b, &mut src, Some(&mut dest), &[0.0], &no_texts());
        assert_eq!((o[0], dest.num(0)), (0.0, 0.0));
        run(&mut *b, &mut src, Some(&mut dest), &[1.0], &no_texts());
        assert_eq!(dest.items, Items::Num(vec![1.0, 2.0]));
        // Held high: no new copy.
        src.set_num(0, 50.0);
        run(&mut *b, &mut src, Some(&mut dest), &[1.0], &no_texts());
        assert_eq!(dest.num(0), 1.0);
        // New rising edge.
        run(&mut *b, &mut src, Some(&mut dest), &[0.0], &no_texts());
        run(&mut *b, &mut src, Some(&mut dest), &[1.0], &no_texts());
        assert_eq!(dest.num(0), 50.0);
    }

    #[test]
    fn copy_with_a_shorter_source_leaves_the_tail() {
        let mut src = ints(ElemType::Int, &[7]);
        let mut dest = ints(ElemType::Int, &[1, 2, 3]);
        let mut b = behavior(ArrayOp::Copy, ElemType::Int);
        run(&mut *b, &mut src, Some(&mut dest), &[1.0], &no_texts());
        assert_eq!(dest.items, Items::Num(vec![7.0, 2.0, 3.0]));
    }

    #[test]
    fn copy_strings_and_missing_destination_flag() {
        let mut src = arr(
            ElemType::String,
            2,
            vec![ParamValue::Text("x".into()), ParamValue::Text("y".into())],
        );
        let mut dest = arr(ElemType::String, 2, Vec::new());
        let mut b = behavior(ArrayOp::Copy, ElemType::String);
        let o = run(&mut *b, &mut src, Some(&mut dest), &[1.0], &no_texts());
        assert_eq!((o[0], dest.text(0), dest.text(1)), (0.0, "x", "y"));
        let o = run(&mut *b, &mut src, None, &[0.0], &no_texts());
        assert_eq!(o[0], 1.0);
        let o = run(&mut *b, &mut src, Some(&mut dest), &[0.0], &no_texts());
        assert_eq!(o[0], 0.0);
    }

    #[test]
    fn sum_wraps_like_the_board() {
        let mut a = ints(ElemType::Int, &[i64::from(i32::MAX), 2]);
        let mut b = behavior(ArrayOp::Sum, ElemType::Int);
        let o = run(&mut *b, &mut a, None, &[], &no_texts());
        assert_eq!(o[0], f64::from(i32::MIN + 1));
        let mut by = ints(ElemType::Byte, &[200, 100]);
        let mut b = behavior(ArrayOp::Sum, ElemType::Byte);
        assert_eq!(run(&mut *b, &mut by, None, &[], &no_texts())[0], 300.0);
    }

    #[test]
    fn sum_of_floats_accumulates_in_f32() {
        let mut a = arr(
            ElemType::Float,
            3,
            vec![
                ParamValue::Float(0.5),
                ParamValue::Float(1.25),
                ParamValue::Float(-0.25),
            ],
        );
        let mut b = behavior(ArrayOp::Sum, ElemType::Float);
        assert_eq!(run(&mut *b, &mut a, None, &[], &no_texts())[0], 1.5);
    }

    #[test]
    fn min_and_max_give_the_first_occurrence() {
        let mut a = ints(ElemType::Int, &[5, -3, 9, -3, 9]);
        let mut mn = behavior(ArrayOp::Min, ElemType::Int);
        let o = run(&mut *mn, &mut a, None, &[], &no_texts());
        assert_eq!((o[0], o[1]), (-3.0, 1.0));
        let mut mx = behavior(ArrayOp::Max, ElemType::Int);
        let o = run(&mut *mx, &mut a, None, &[], &no_texts());
        assert_eq!((o[0], o[1]), (9.0, 2.0));
    }

    #[test]
    fn min_of_an_empty_array_does_not_panic() {
        let mut a = ints(ElemType::Int, &[]);
        let mut mn = behavior(ArrayOp::Min, ElemType::Int);
        let o = run(&mut *mn, &mut a, None, &[], &no_texts());
        assert_eq!((o[0], o[1]), (0.0, 0.0));
    }

    #[test]
    fn find_numbers_with_the_storing_rule() {
        let mut a = ints(ElemType::Byte, &[4, 1, 1]);
        let mut b = behavior(ArrayOp::Find, ElemType::Byte);
        // 257 is stored as 1 on a byte array, so it equals element 1.
        let o = run(&mut *b, &mut a, None, &[257.0], &no_texts());
        assert_eq!((o[0], o[1]), (1.0, 1.0));
        let o = run(&mut *b, &mut a, None, &[9.0], &no_texts());
        assert_eq!((o[0], o[1]), (-1.0, 0.0));
    }

    #[test]
    fn find_strings_exactly() {
        let mut a = arr(
            ElemType::String,
            3,
            vec![ParamValue::Text("a".into()), ParamValue::Text("bc".into())],
        );
        let mut b = behavior(ArrayOp::Find, ElemType::String);
        let t = |s: &str| vec![s.to_owned(), String::new()];
        let o = run(&mut *b, &mut a, None, &[0.0], &t("bc"));
        assert_eq!((o[0], o[1]), (1.0, 1.0));
        let o = run(&mut *b, &mut a, None, &[0.0], &t("b"));
        assert_eq!((o[0], o[1]), (-1.0, 0.0));
        // An empty string matches the empty element 2.
        let o = run(&mut *b, &mut a, None, &[0.0], &t(""));
        assert_eq!((o[0], o[1]), (2.0, 1.0));
    }
}
