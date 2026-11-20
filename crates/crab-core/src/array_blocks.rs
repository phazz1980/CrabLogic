//! Blocks of the arrays (`specs/arrays-structs`): built in, like `var_get` / `var_set`. A block
//! names its array in the parameter `array`; the kind the block really has depends on the element
//! type of that array (`array_get` on a `float` array is `array_get_float`, on a `byte` array
//! `array_get_byte`), so the value ports carry the right signal type.
//!
//! Only scalars go over the wires: an index, a value, flags. The array itself is a project object
//! (see [`crate::arrays`]) that the blocks reach by name.
//!
//! An index outside the array never panics: a read gives 0 (an empty string) and the output `err`
//! is true, a write is skipped and `err` is true.

use std::collections::BTreeMap;

use crate::arrays::ElemType;
use crate::library::{BlockDef, BlockInfo, LibraryBlock, ParamDef, ParamKind, PortDef};
use crate::{Block, Logic, ParamValue, SignalType};

/// Parameter of the array blocks naming the array.
pub const ARRAY_PARAM: &str = "array";
/// Parameter of `array_copy` naming the destination array.
pub const DEST_PARAM: &str = "dest";

/// What an array block does; the base kind is `array_<op>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayOp {
    /// Reads the element at `index`.
    Get,
    /// Writes `value` into the element at `index`.
    Set,
    /// Writes `value` into every element while `go` is true.
    Fill,
    /// Copies the array into `dest` (the shorter length) on the rising edge of `go`.
    Copy,
    /// Sum of the elements (numeric types).
    Sum,
    /// Smallest element and its index (numeric types).
    Min,
    /// Largest element and its index (numeric types).
    Max,
    /// Index of the first element equal to `value`, -1 when there is none.
    Find,
    /// Shift register: on the rising edge of `go` every element moves one place up (the last one
    /// is dropped) and `value` goes into element 0.
    Shift,
    /// Ring buffer: on the rising edge of `push` `value` is written at the head and the head moves
    /// on; when the array is full the oldest element is overwritten.
    Ring,
    /// Stack: `push` (rising edge) puts `value` on top, `pop` (rising edge) takes the top off. A
    /// full stack does not take a new element: `err`.
    Stack,
}

impl ArrayOp {
    pub const ALL: [ArrayOp; 11] = [
        ArrayOp::Get,
        ArrayOp::Set,
        ArrayOp::Fill,
        ArrayOp::Copy,
        ArrayOp::Sum,
        ArrayOp::Min,
        ArrayOp::Max,
        ArrayOp::Find,
        ArrayOp::Shift,
        ArrayOp::Ring,
        ArrayOp::Stack,
    ];

    /// Base kind in a schema (`array_get`).
    pub fn kind(self) -> &'static str {
        match self {
            ArrayOp::Get => "array_get",
            ArrayOp::Set => "array_set",
            ArrayOp::Fill => "array_fill",
            ArrayOp::Copy => "array_copy",
            ArrayOp::Sum => "array_sum",
            ArrayOp::Min => "array_min",
            ArrayOp::Max => "array_max",
            ArrayOp::Find => "array_find",
            ArrayOp::Shift => "array_shift",
            ArrayOp::Ring => "array_ring",
            ArrayOp::Stack => "array_stack",
        }
    }

    pub fn from_kind(kind: &str) -> Option<ArrayOp> {
        ArrayOp::ALL.into_iter().find(|op| op.kind() == kind)
    }

    /// Whether the block exists for arrays of `ty` (sum, min and max need numbers).
    pub fn exists_for(self, ty: ElemType) -> bool {
        match self {
            ArrayOp::Sum | ArrayOp::Min | ArrayOp::Max => {
                matches!(ty, ElemType::Int | ElemType::Float | ElemType::Byte)
            }
            _ => true,
        }
    }

    /// Name and description of the block in Russian.
    pub fn caption(self) -> (&'static str, &'static str) {
        match self {
            ArrayOp::Get => ("Массив: чтение", "Читает элемент массива по индексу."),
            ArrayOp::Set => ("Массив: запись", "Записывает значение в элемент массива по индексу."),
            ArrayOp::Fill => ("Массив: заполнить", "Пока вход go включён, записывает значение во все элементы."),
            ArrayOp::Copy => ("Массив: копировать", "По фронту go копирует массив в другой массив (по меньшей длине)."),
            ArrayOp::Sum => ("Массив: сумма", "Сумма всех элементов."),
            ArrayOp::Min => ("Массив: минимум", "Наименьший элемент и его индекс."),
            ArrayOp::Max => ("Массив: максимум", "Наибольший элемент и его индекс."),
            ArrayOp::Find => ("Массив: поиск", "Индекс первого элемента, равного значению; -1, если такого нет."),
            ArrayOp::Shift => ("Массив: сдвиг", "По фронту go элементы сдвигаются на одно место вверх, значение встаёт на место 0."),
            ArrayOp::Ring => ("Массив: кольцевой буфер", "По фронту push значение пишется в голову буфера; когда буфер полон, затирается самый старый элемент."),
            ArrayOp::Stack => ("Массив: стек", "push кладёт значение наверх, pop снимает верхний элемент; полный стек не принимает новое значение."),
        }
    }

    /// Input ports in order: (name, type; `None` is the element type, description).
    pub fn inputs(self) -> &'static [(&'static str, Option<SignalType>, &'static str)] {
        const IDX: Option<SignalType> = Some(SignalType::Int);
        const FLAG: Option<SignalType> = Some(SignalType::Bool);
        match self {
            ArrayOp::Get => &[("index", IDX, "Номер элемента с нуля")],
            ArrayOp::Set => &[
                ("index", IDX, "Номер элемента с нуля"),
                ("value", None, "Новое значение"),
            ],
            ArrayOp::Fill => &[
                ("value", None, "Значение для всех элементов"),
                ("go", FLAG, "Заполнять, пока включено"),
            ],
            ArrayOp::Copy => &[("go", FLAG, "Копировать по фронту")],
            ArrayOp::Sum | ArrayOp::Min | ArrayOp::Max => &[],
            ArrayOp::Find => &[("value", None, "Искомое значение")],
            ArrayOp::Shift => &[
                ("value", None, "Новое значение"),
                ("go", FLAG, "Сдвинуть по фронту"),
            ],
            ArrayOp::Ring => &[
                ("value", None, "Новое значение"),
                ("push", FLAG, "Записать по фронту"),
            ],
            ArrayOp::Stack => &[
                ("value", None, "Значение для push"),
                ("push", FLAG, "Положить по фронту"),
                ("pop", FLAG, "Снять по фронту"),
            ],
        }
    }

    /// Output ports in order: (name, type; `None` is the element type, description).
    pub fn outputs(self) -> &'static [(&'static str, Option<SignalType>, &'static str)] {
        const IDX: Option<SignalType> = Some(SignalType::Int);
        const FLAG: Option<SignalType> = Some(SignalType::Bool);
        match self {
            ArrayOp::Get => &[
                ("out", None, "Значение элемента"),
                ("err", FLAG, "Индекс вне массива"),
            ],
            ArrayOp::Set => &[("err", FLAG, "Индекс вне массива")],
            ArrayOp::Fill => &[],
            ArrayOp::Copy => &[("err", FLAG, "Массивы не найдены или разных типов")],
            ArrayOp::Sum => &[("out", None, "Сумма")],
            ArrayOp::Min | ArrayOp::Max => {
                &[("out", None, "Значение"), ("index", IDX, "Его индекс")]
            }
            ArrayOp::Find => &[("index", IDX, "Индекс или -1"), ("found", FLAG, "Нашлось")],
            ArrayOp::Shift => &[],
            ArrayOp::Ring => &[
                ("head", IDX, "Куда пойдёт следующее значение"),
                (
                    "count",
                    IDX,
                    "Сколько элементов записано (до длины массива)",
                ),
            ],
            ArrayOp::Stack => &[
                ("out", None, "Верхний элемент (0, если стек пуст)"),
                ("count", IDX, "Сколько элементов в стеке"),
                ("empty", FLAG, "Стек пуст"),
                ("full", FLAG, "Стек полон"),
                ("err", FLAG, "Не принято: стек полон (push) или пуст (pop)"),
            ],
        }
    }
}

/// Element type of the array a block names; `int` while the array is unknown (the check reports it).
fn elem_of(logic: &Logic, block: &Block) -> ElemType {
    match block.params.get(ARRAY_PARAM) {
        Some(ParamValue::Text(name)) => logic.array(name).map_or(ElemType::Int, |a| a.ty),
        _ => ElemType::Int,
    }
}

/// Array named by the block, or `None` for an ordinary block.
pub fn array_name(block: &Block) -> Option<&str> {
    ArrayOp::from_kind(&block.kind)?;
    match block.params.get(ARRAY_PARAM) {
        Some(ParamValue::Text(name)) => Some(name),
        _ => None,
    }
}

/// Internal library kind of an array block (`array_get_float`), or `None` for ordinary blocks.
/// A kind the element type does not allow (`array_sum` on strings) gives the kind of the nearest
/// allowed type: the check reports the mismatch.
pub(crate) fn builtin_kind(logic: &Logic, block: &Block) -> Option<String> {
    let op = ArrayOp::from_kind(&block.kind)?;
    let mut ty = elem_of(logic, block);
    if !op.exists_for(ty) {
        ty = ElemType::Int;
    }
    Some(format!("{}_{}", op.kind(), ty.name()))
}

/// Base operation and element type of an internal kind (`array_get_float` → `Get`, `Float`).
pub fn split_kind(kind: &str) -> Option<(ArrayOp, ElemType)> {
    ElemType::ALL.into_iter().find_map(|ty| {
        let base = kind.strip_suffix(ty.name())?.strip_suffix('_')?;
        Some((ArrayOp::from_kind(base)?, ty))
    })
}

pub(crate) fn port(name: &str, ty: SignalType, description: &str, input: bool) -> PortDef {
    PortDef {
        name: name.to_owned(),
        ty,
        // Flags and indexes of an unconnected input stay off / zero.
        default: match (input, ty) {
            (true, SignalType::Bool) => Some(ParamValue::Bool(false)),
            (true, SignalType::Int) => Some(ParamValue::Int(0)),
            _ => None,
        },
        when: None,
        label: None,
        description: description.to_owned(),
        sim: None,
        fast: false,
        optional: false,
    }
}

pub(crate) fn text_param(name: &str, description: &str) -> ParamDef {
    ParamDef {
        name: name.to_owned(),
        kind: ParamKind::Text,
        default: None,
        min: None,
        max: None,
        when: None,
        label: None,
        description: description.to_owned(),
        show: Default::default(),
        local: None,
        mode: Default::default(),
        shared: false,
        bus: String::new(),
        fixed: false,
        runtime: false,
        static_only: false,
        choices: Vec::new(),
        of_block: String::new(),
    }
}

pub(crate) fn builtin_blocks() -> BTreeMap<String, LibraryBlock> {
    let mut blocks = BTreeMap::new();
    for op in ArrayOp::ALL {
        for ty in ElemType::ALL {
            if !op.exists_for(ty) {
                continue;
            }
            let value = ty.signal();
            let ports = |list: &[(&str, Option<SignalType>, &str)], input: bool| -> Vec<PortDef> {
                list.iter()
                    .map(|(name, t, d)| port(name, t.unwrap_or(value), d, input))
                    .collect()
            };
            let (name, description) = op.caption();
            let mut params = vec![text_param(ARRAY_PARAM, "Имя массива")];
            if op == ArrayOp::Copy {
                params.push(text_param(DEST_PARAM, "Имя массива-приёмника"));
            }
            let def = BlockDef {
                info: BlockInfo {
                    kind: format!("{}_{}", op.kind(), ty.name()),
                    symbol: None,
                    symbol_param: None,
                    name: name.to_owned(),
                    group: "Массивы".to_owned(),
                    shared: String::new(),
                    description: description.to_owned(),
                    // Reading takes the value of the previous scan, so loops through an array
                    // are allowed, like through a variable.
                    memory: op == ArrayOp::Get,
                    single: false,
                    hidden: false,
                    pullable: false,
                    uart: None,
                    board: None,
                    code_of: None,
                    expander: false,
                },
                inputs: ports(op.inputs(), true),
                outputs: ports(op.outputs(), false),
                params,
            };
            blocks.insert(
                def.info.kind.clone(),
                LibraryBlock {
                    def,
                    dir: Default::default(),
                },
            );
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrays::ArrayVar;

    fn block(kind: &str, array: &str) -> Block {
        let mut b = Block {
            id: "b1".into(),
            kind: kind.into(),
            ..Default::default()
        };
        b.params
            .insert(ARRAY_PARAM.into(), ParamValue::Text(array.into()));
        b
    }

    fn logic(ty: ElemType) -> Logic {
        Logic {
            arrays: vec![ArrayVar {
                name: "a".into(),
                ty,
                len: 4,
                items: Vec::new(),
                description: String::new(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn the_kind_follows_the_element_type() {
        let l = logic(ElemType::Float);
        assert_eq!(
            builtin_kind(&l, &block("array_get", "a")).as_deref(),
            Some("array_get_float")
        );
        assert_eq!(
            builtin_kind(&l, &block("array_get", "none")).as_deref(),
            Some("array_get_int")
        );
        assert_eq!(builtin_kind(&l, &block("and", "a")), None);
        let s = logic(ElemType::String);
        assert_eq!(
            builtin_kind(&s, &block("array_sum", "a")).as_deref(),
            Some("array_sum_int")
        );
    }

    #[test]
    fn every_kind_is_built_and_split_back() {
        let blocks = builtin_blocks();
        let mut count = 0;
        for op in ArrayOp::ALL {
            for ty in ElemType::ALL {
                let kind = format!("{}_{}", op.kind(), ty.name());
                if op.exists_for(ty) {
                    count += 1;
                    let def = &blocks[&kind].def;
                    assert_eq!(split_kind(&kind), Some((op, ty)));
                    assert_eq!(def.inputs.len(), op.inputs().len());
                    assert_eq!(def.outputs.len(), op.outputs().len());
                } else {
                    assert!(!blocks.contains_key(&kind), "{kind}");
                }
            }
        }
        assert_eq!(blocks.len(), count);
    }

    #[test]
    fn value_ports_carry_the_element_signal() {
        let blocks = builtin_blocks();
        let get = &blocks["array_get_float"].def;
        assert_eq!(get.outputs[0].ty, SignalType::Float);
        assert_eq!(get.inputs[0].ty, SignalType::Int);
        let set = &blocks["array_set_string"].def;
        assert_eq!(set.inputs[1].ty, SignalType::String);
        assert_eq!(blocks["array_set_byte"].def.inputs[1].ty, SignalType::Int);
    }
}
