//! Editing of arrays (`specs/arrays-structs`, task 8): the commands the editor, the AI and the CLI
//! use. They keep the project consistent: names are shared with the variables, a rename follows into
//! the blocks, a removal takes the blocks of the array along.

use crate::array_blocks::{ArrayOp, ARRAY_PARAM, DEST_PARAM};
use crate::arrays::{ArrayVar, MAX_LEN};
use crate::edit::EditError;
use crate::layout::BlockPos;
use crate::{Block, ParamValue, Project};

impl Project {
    fn array_index(&self, name: &str) -> Result<usize, EditError> {
        self.logic
            .arrays
            .iter()
            .position(|a| a.name == name)
            .ok_or_else(|| EditError::NoArray(name.to_owned()))
    }

    /// A name is free text like a variable name, and unique among variables, arrays and structures
    /// (`old` is the array's own name while it is edited).
    fn validate_array(&self, array: &ArrayVar, old: Option<&str>) -> Result<(), EditError> {
        let name = array.name.as_str();
        if !crate::vars::is_valid_var_name(name) {
            return Err(EditError::InvalidVarName(name.to_owned()));
        }
        let taken = self.logic.var(name).is_some()
            || self.logic.struct_var(name).is_some()
            || (Some(name) != old && self.logic.array(name).is_some());
        if taken {
            return Err(EditError::BadArray(format!(
                "имя «{name}» уже занято переменной, массивом или структурой"
            )));
        }
        if array.len == 0 || array.len > MAX_LEN {
            return Err(EditError::BadArray(format!(
                "длина массива «{name}» должна быть от 1 до {MAX_LEN}"
            )));
        }
        if array.items.len() > array.len as usize {
            return Err(EditError::BadArray(format!(
                "у массива «{name}» начальных значений больше длины"
            )));
        }
        if let Some(i) = array.items.iter().position(|v| !array.ty.accepts(v)) {
            return Err(EditError::BadArray(format!(
                "у массива «{name}» начальное значение элемента {i} не подходит к типу {}",
                array.ty.name()
            )));
        }
        Ok(())
    }

    /// Declares a new array.
    pub fn add_array(&mut self, array: ArrayVar) -> Result<(), EditError> {
        self.validate_array(&array, None)?;
        self.logic.arrays.push(array);
        Ok(())
    }

    /// Replaces an array declaration (name, type, length, initial values). Blocks that name it
    /// follow a rename; a new type or length that no longer fits stays and the check reports it.
    pub fn update_array(&mut self, name: &str, array: ArrayVar) -> Result<(), EditError> {
        let index = self.array_index(name)?;
        self.validate_array(&array, Some(name))?;
        if array.name != name {
            for block in &mut self.logic.blocks {
                if ArrayOp::from_kind(&block.kind).is_none() {
                    continue;
                }
                for param in [ARRAY_PARAM, DEST_PARAM] {
                    if matches!(block.params.get(param), Some(ParamValue::Text(n)) if n == name) {
                        block
                            .params
                            .insert(param.to_owned(), ParamValue::Text(array.name.clone()));
                    }
                }
            }
        }
        self.logic.arrays[index] = array;
        Ok(())
    }

    /// Removes an array and the blocks that name it (as the array or as the destination of a copy).
    pub fn remove_array(&mut self, name: &str) -> Result<(), EditError> {
        let index = self.array_index(name)?;
        self.logic.arrays.remove(index);
        let users: Vec<String> = self
            .logic
            .blocks
            .iter()
            .filter(|b| {
                ArrayOp::from_kind(&b.kind).is_some()
                    && [ARRAY_PARAM, DEST_PARAM]
                        .iter()
                        .any(|p| matches!(b.params.get(*p), Some(ParamValue::Text(n)) if n == name))
            })
            .map(|b| b.id.clone())
            .collect();
        for id in users {
            self.remove_block(&id)?;
        }
        Ok(())
    }

    /// Adds a block of the array operation `op` for the array `array` and returns its id
    /// (`window_r1`, `window_ring1`, ...). The block of a copy has no destination yet.
    pub fn add_array_block(
        &mut self,
        op: ArrayOp,
        array: &str,
        pos: BlockPos,
    ) -> Result<String, EditError> {
        let Some(var) = self.logic.array(array) else {
            return Err(EditError::NoArray(array.to_owned()));
        };
        if !op.exists_for(var.ty) {
            return Err(EditError::BadArray(format!(
                "блок «{}» не работает с массивом «{array}» типа {}",
                op.kind(),
                var.ty.name()
            )));
        }
        let short = match op {
            ArrayOp::Get => "r",
            ArrayOp::Set => "w",
            other => other.kind().strip_prefix("array_").unwrap_or("op"),
        };
        let id = (1..)
            .map(|n| format!("{array}_{short}{n}"))
            .find(|id| self.logic.block(id).is_none())
            .expect("infinite range");
        self.logic.blocks.push(Block {
            id: id.clone(),
            kind: op.kind().to_owned(),
            params: [(ARRAY_PARAM.to_owned(), ParamValue::Text(array.to_owned()))].into(),
            ..Default::default()
        });
        self.layout.blocks.insert(id.clone(), pos);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrays::ElemType;
    use crate::{Logic, ProjectConfig};

    fn project() -> Project {
        let config: ProjectConfig =
            toml::from_str("[project]\nname = \"t\"\n[board]\nfqbn = \"x:y:z\"\n").unwrap();
        Project {
            path: Default::default(),
            config,
            logic: Logic::default(),
            layout: Default::default(),
        }
    }

    const POS: BlockPos = BlockPos {
        sheet: 1,
        x: 0.0,
        y: 0.0,
    };

    fn array(name: &str, ty: ElemType, len: u32) -> ArrayVar {
        ArrayVar {
            name: name.into(),
            ty,
            len,
            items: Vec::new(),
            description: String::new(),
        }
    }

    #[test]
    fn add_checks_name_length_and_values() {
        let mut p = project();
        p.add_array(array("a", ElemType::Int, 4)).unwrap();
        assert!(matches!(
            p.add_array(array("a", ElemType::Int, 4)),
            Err(EditError::BadArray(_))
        ));
        assert!(matches!(
            p.add_array(array(" x", ElemType::Int, 4)),
            Err(EditError::InvalidVarName(_))
        ));
        assert!(p.add_array(array("z", ElemType::Int, 0)).is_err());
        assert!(p.add_array(array("z", ElemType::Int, MAX_LEN + 1)).is_err());
        let mut many = array("m", ElemType::Byte, 1);
        many.items = vec![ParamValue::Int(1), ParamValue::Int(2)];
        assert!(p.add_array(many).is_err());
        let mut bad = array("b", ElemType::Byte, 2);
        bad.items = vec![ParamValue::Int(256)];
        assert!(p.add_array(bad).is_err());
        // The name is shared with the variables.
        p.add_var(crate::Var {
            name: "v".into(),
            ty: crate::VarType::Int,
            init: None,
            description: String::new(),
            retain: false,
            service: false,
            settle_s: None,
            period_s: None,
        })
        .unwrap();
        assert!(p.add_array(array("v", ElemType::Int, 2)).is_err());
        assert_eq!(p.logic.arrays.len(), 1);
    }

    #[test]
    fn a_rename_follows_into_the_blocks() {
        let mut p = project();
        p.add_array(array("a", ElemType::Int, 4)).unwrap();
        p.add_array(array("b", ElemType::Int, 4)).unwrap();
        let get = p.add_array_block(ArrayOp::Get, "a", POS).unwrap();
        let copy = p.add_array_block(ArrayOp::Copy, "b", POS).unwrap();
        p.logic
            .blocks
            .iter_mut()
            .find(|b| b.id == copy)
            .unwrap()
            .params
            .insert(DEST_PARAM.into(), ParamValue::Text("a".into()));
        p.update_array("a", array("c", ElemType::Int, 6)).unwrap();
        let param = |p: &Project, id: &str, name: &str| {
            p.logic.block(id).unwrap().params.get(name).cloned()
        };
        assert_eq!(
            param(&p, &get, ARRAY_PARAM),
            Some(ParamValue::Text("c".into()))
        );
        assert_eq!(
            param(&p, &copy, DEST_PARAM),
            Some(ParamValue::Text("c".into()))
        );
        assert_eq!(
            param(&p, &copy, ARRAY_PARAM),
            Some(ParamValue::Text("b".into()))
        );
        assert_eq!(p.logic.array("c").map(|a| a.len), Some(6));
        assert!(p
            .update_array("nope", array("x", ElemType::Int, 2))
            .is_err());
        // A rename onto another array's name is refused, a change that keeps the name is fine.
        assert!(p.update_array("c", array("b", ElemType::Int, 6)).is_err());
        p.update_array("c", array("c", ElemType::Float, 3)).unwrap();
    }

    #[test]
    fn remove_takes_the_blocks_along() {
        let mut p = project();
        p.add_array(array("a", ElemType::Int, 4)).unwrap();
        p.add_array(array("b", ElemType::Int, 4)).unwrap();
        p.add_array_block(ArrayOp::Get, "a", POS).unwrap();
        let copy = p.add_array_block(ArrayOp::Copy, "b", POS).unwrap();
        p.logic
            .blocks
            .iter_mut()
            .find(|b| b.id == copy)
            .unwrap()
            .params
            .insert(DEST_PARAM.into(), ParamValue::Text("a".into()));
        p.remove_array("a").unwrap();
        assert!(p.logic.array("a").is_none());
        assert!(p.logic.blocks.is_empty(), "{:?}", p.logic.blocks);
        assert!(p.layout.blocks.is_empty());
        assert!(p.remove_array("a").is_err());
        assert_eq!(p.logic.arrays.len(), 1);
    }

    #[test]
    fn blocks_get_free_ids_and_fit_the_type() {
        let mut p = project();
        p.add_array(array("w", ElemType::Float, 4)).unwrap();
        p.add_array(array("s", ElemType::String, 2)).unwrap();
        assert_eq!(p.add_array_block(ArrayOp::Get, "w", POS).unwrap(), "w_r1");
        assert_eq!(p.add_array_block(ArrayOp::Get, "w", POS).unwrap(), "w_r2");
        assert_eq!(p.add_array_block(ArrayOp::Set, "w", POS).unwrap(), "w_w1");
        assert_eq!(
            p.add_array_block(ArrayOp::Ring, "w", POS).unwrap(),
            "w_ring1"
        );
        assert!(p.add_array_block(ArrayOp::Sum, "s", POS).is_err());
        assert!(matches!(
            p.add_array_block(ArrayOp::Get, "none", POS),
            Err(EditError::NoArray(_))
        ));
        assert!(p.layout.blocks.contains_key("w_r2"));
    }
}
