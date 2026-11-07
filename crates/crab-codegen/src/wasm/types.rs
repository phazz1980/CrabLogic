use super::*;

/// The two machine types of the model: `i32` carries `bool` and `int`, `f32` carries `float`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Ty {
    I,
    F,
}

impl Ty {
    pub(super) fn of(ty: SignalType) -> Ty {
        if ty == SignalType::Float {
            Ty::F
        } else {
            Ty::I
        }
    }

    pub(super) fn of_kind(kind: ParamKind) -> Ty {
        if kind == ParamKind::Float {
            Ty::F
        } else {
            Ty::I
        }
    }

    pub(super) fn val(self) -> ValType {
        match self {
            Ty::I => ValType::I32,
            Ty::F => ValType::F32,
        }
    }

    /// `float` if either is.
    pub(super) fn widen(self, other: Ty) -> Ty {
        if self == Ty::F || other == Ty::F {
            Ty::F
        } else {
            Ty::I
        }
    }
}

pub(super) fn pv(value: &ParamValue) -> f64 {
    match value {
        ParamValue::Bool(b) => f64::from(u8::from(*b)),
        ParamValue::Int(i) => *i as f64,
        ParamValue::Float(x) => *x,
        ParamValue::Text(_) => 0.0,
    }
}

/// Parameter value as a number; durations become milliseconds.
pub(super) fn num(value: &ParamValue, kind: ParamKind) -> f64 {
    match value {
        ParamValue::Text(s) if kind == ParamKind::Duration => {
            parse_duration_ms(s).map_or(0.0, |ms| ms as f64)
        }
        other => pv(other),
    }
}

/// A global: its type and the bits of the initial value.
impl From<crab_blocks::WTy> for Ty {
    fn from(t: crab_blocks::WTy) -> Ty {
        match t {
            crab_blocks::WTy::I => Ty::I,
            crab_blocks::WTy::F => Ty::F,
        }
    }
}

impl From<Ty> for crab_blocks::WTy {
    fn from(t: Ty) -> Self {
        match t {
            Ty::I => crab_blocks::WTy::I,
            Ty::F => crab_blocks::WTy::F,
        }
    }
}
