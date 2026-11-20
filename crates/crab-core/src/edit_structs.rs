//! Editing of structures (`specs/arrays-structs`, task 3б): the commands the editor, the AI and the
//! CLI use. Names are shared with the variables and arrays, a rename follows into the blocks, a
//! changed field list takes the wires of the ports that are gone, a removal takes the blocks along.

use crate::arrays::{ElemType, StructVar};
use crate::edit::EditError;
use crate::layout::BlockPos;
use crate::struct_blocks::{is_struct_kind, struct_name, GO, STRUCT_PARAM};
use crate::{Block, ParamValue, Project};

impl Project {
    fn struct_index(&self, name: &str) -> Result<usize, EditError> {
        self.logic
            .structs
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| EditError::NoStruct(name.to_owned()))
    }

    /// `old` is the name of the structure itself while it is edited.
    fn validate_struct(&self, st: &StructVar, old: Option<&str>) -> Result<(), EditError> {
        let name = st.name.as_str();
        if !crate::vars::is_valid_var_name(name) {
            return Err(EditError::InvalidVarName(name.to_owned()));
        }
        let taken = self.logic.var(name).is_some()
            || self.logic.array(name).is_some()
            || (Some(name) != old && self.logic.struct_var(name).is_some());
        if taken {
            return Err(EditError::BadStruct(format!(
                "имя «{name}» уже занято переменной, массивом или структурой"
            )));
        }
        if st.fields.is_empty() {
            return Err(EditError::BadStruct(format!(
                "у структуры «{name}» нет ни одного поля"
            )));
        }
        let mut seen: Vec<&str> = Vec::new();
        for f in &st.fields {
            let field = f.name.as_str();
            if !crate::vars::is_valid_var_name(field) {
                return Err(EditError::BadStruct(format!(
                    "недопустимое имя поля «{field}» структуры «{name}»"
                )));
            }
            if field == GO {
                return Err(EditError::BadStruct(format!(
                    "поле «{GO}» занято входом разрешения блока «Структура: собрать»"
                )));
            }
            if seen.contains(&field) {
                return Err(EditError::BadStruct(format!(
                    "поле «{field}» структуры «{name}» объявлено дважды"
                )));
            }
            seen.push(field);
            if f.ty == ElemType::String {
                return Err(EditError::BadStruct(format!(
                    "поле «{field}»: строки в структурах пока не поддерживаются"
                )));
            }
            if f.init.as_ref().is_some_and(|v| !f.ty.accepts(v)) {
                return Err(EditError::BadStruct(format!(
                    "начальное значение поля «{field}» не подходит к типу {}",
                    f.ty.name()
                )));
            }
        }
        Ok(())
    }

    /// Declares a new structure.
    pub fn add_struct(&mut self, st: StructVar) -> Result<(), EditError> {
        self.validate_struct(&st, None)?;
        self.logic.structs.push(st);
        Ok(())
    }

    /// Replaces a structure declaration. Blocks that name it follow a rename; the wires of ports
    /// that no longer exist (a removed or renamed field, a changed type) are taken off.
    pub fn update_struct(&mut self, name: &str, st: StructVar) -> Result<(), EditError> {
        let index = self.struct_index(name)?;
        self.validate_struct(&st, Some(name))?;
        let users: Vec<String> = self
            .logic
            .blocks
            .iter()
            .filter(|b| struct_name(b) == Some(name))
            .map(|b| b.id.clone())
            .collect();
        let old = self.logic.structs[index].clone();
        for block in self
            .logic
            .blocks
            .iter_mut()
            .filter(|b| users.contains(&b.id))
        {
            block
                .params
                .insert(STRUCT_PARAM.to_owned(), ParamValue::Text(st.name.clone()));
        }
        // A port survives when the new structure has a field of that name and signal type.
        let gone: Vec<&str> = old
            .fields
            .iter()
            .filter(|f| {
                !st.fields
                    .iter()
                    .any(|n| n.name == f.name && n.ty.signal() == f.ty.signal())
            })
            .map(|f| f.name.as_str())
            .collect();
        self.logic.links.retain(|l| {
            let own =
                |p: &crate::PortRef| users.contains(&p.block) && gone.contains(&p.port.as_str());
            !(own(&l.from) || own(&l.to))
        });
        for block in self
            .logic
            .blocks
            .iter_mut()
            .filter(|b| users.contains(&b.id))
        {
            block
                .consts
                .retain(|port, _| !gone.contains(&port.as_str()));
        }
        self.logic.structs[index] = st;
        Ok(())
    }

    /// Removes a structure and the blocks that name it.
    pub fn remove_struct(&mut self, name: &str) -> Result<(), EditError> {
        let index = self.struct_index(name)?;
        self.logic.structs.remove(index);
        let users: Vec<String> = self
            .logic
            .blocks
            .iter()
            .filter(|b| struct_name(b) == Some(name))
            .map(|b| b.id.clone())
            .collect();
        for id in users {
            self.remove_block(&id)?;
        }
        Ok(())
    }

    /// Adds a structure block from the library: the structure is taken when the project has exactly
    /// one, otherwise it is chosen in the properties.
    pub(crate) fn add_struct_op_block(&mut self, kind: &str, pos: BlockPos) -> String {
        let id = (1..)
            .map(|n| format!("{kind}_{n}"))
            .find(|id| self.logic.block(id).is_none())
            .expect("infinite range");
        let only = match self.logic.structs.as_slice() {
            [s] => Some(s.name.clone()),
            _ => None,
        };
        self.logic.blocks.push(Block {
            id: id.clone(),
            kind: kind.to_owned(),
            params: only
                .into_iter()
                .map(|name| (STRUCT_PARAM.to_owned(), ParamValue::Text(name)))
                .collect(),
            ..Default::default()
        });
        self.layout.blocks.insert(id.clone(), pos);
        id
    }

    /// Adds a block `struct_pack` or `struct_unpack` for the structure `st` and returns its id
    /// (`recipe_pack1`, `recipe_unpack1`).
    pub fn add_struct_block(
        &mut self,
        kind: &str,
        st: &str,
        pos: BlockPos,
    ) -> Result<String, EditError> {
        if !is_struct_kind(kind) {
            return Err(EditError::UnknownType(kind.to_owned()));
        }
        if self.logic.struct_var(st).is_none() {
            return Err(EditError::NoStruct(st.to_owned()));
        }
        let short = kind.strip_prefix("struct_").unwrap_or("op");
        let id = (1..)
            .map(|n| format!("{st}_{short}{n}"))
            .find(|id| self.logic.block(id).is_none())
            .expect("infinite range");
        self.logic.blocks.push(Block {
            id: id.clone(),
            kind: kind.to_owned(),
            params: [(STRUCT_PARAM.to_owned(), ParamValue::Text(st.to_owned()))].into(),
            ..Default::default()
        });
        self.layout.blocks.insert(id.clone(), pos);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrays::StructField;
    use crate::struct_blocks::{PACK, UNPACK};
    use crate::{Link, Logic, PortRef, ProjectConfig};

    fn project() -> Project {
        let config: ProjectConfig = toml::from_str(
            "[project]
name = \"t\"
[board]
fqbn = \"x:y:z\"
",
        )
        .unwrap();
        Project {
            path: Default::default(),
            config,
            logic: Logic::default(),
            layout: Default::default(),
        }
    }

    fn field(name: &str, ty: ElemType) -> StructField {
        StructField {
            name: name.into(),
            ty,
            init: None,
        }
    }

    fn recipe() -> StructVar {
        StructVar {
            name: "recipe".into(),
            fields: vec![field("temp", ElemType::Float), field("fan", ElemType::Bool)],
            description: String::new(),
        }
    }

    fn pos() -> BlockPos {
        BlockPos {
            sheet: 1,
            x: 0.0,
            y: 0.0,
        }
    }

    #[test]
    fn a_structure_is_checked_when_added() {
        let mut p = project();
        p.add_struct(recipe()).unwrap();
        assert!(p.add_struct(recipe()).is_err(), "twice");
        let mut s = recipe();
        s.name = "other".into();
        s.fields.clear();
        assert!(p.add_struct(s).is_err(), "no fields");
        let mut s = recipe();
        s.name = "other".into();
        s.fields.push(field("go", ElemType::Int));
        assert!(p.add_struct(s).is_err(), "go is taken");
        let mut s = recipe();
        s.name = "other".into();
        s.fields.push(field("text", ElemType::String));
        assert!(p.add_struct(s).is_err(), "strings");
        let mut s = recipe();
        s.name = "other".into();
        s.fields.push(field("temp", ElemType::Int));
        assert!(p.add_struct(s).is_err(), "twice in one structure");
    }

    #[test]
    fn a_rename_follows_and_a_gone_field_takes_its_wire() {
        let mut p = project();
        p.add_struct(recipe()).unwrap();
        let pack = p.add_struct_block(PACK, "recipe", pos()).unwrap();
        let unpack = p.add_struct_block(UNPACK, "recipe", pos()).unwrap();
        assert_eq!(pack, "recipe_pack1");
        p.logic.links.push(Link {
            from: PortRef::new(&unpack, "fan"),
            to: PortRef::new(&pack, "fan"),
        });
        p.logic.links.push(Link {
            from: PortRef::new(&unpack, "temp"),
            to: PortRef::new(&pack, "temp"),
        });
        let mut s = recipe();
        s.name = "mix".into();
        s.fields.remove(1); // `fan` goes
        p.update_struct("recipe", s).unwrap();
        assert_eq!(struct_name(p.logic.block(&pack).unwrap()), Some("mix"));
        assert_eq!(p.logic.links.len(), 1);
        assert_eq!(p.logic.links[0].to.port, "temp");
    }

    #[test]
    fn remove_takes_the_blocks_along() {
        let mut p = project();
        p.add_struct(recipe()).unwrap();
        p.add_struct_block(PACK, "recipe", pos()).unwrap();
        p.remove_struct("recipe").unwrap();
        assert!(p.logic.structs.is_empty() && p.logic.blocks.is_empty());
        assert!(p.remove_struct("recipe").is_err());
    }

    #[test]
    fn the_only_structure_is_taken_by_a_new_block() {
        let mut p = project();
        let id = p.add_struct_op_block(UNPACK, pos());
        assert!(struct_name(p.logic.block(&id).unwrap()).is_none());
        p.add_struct(recipe()).unwrap();
        let id = p.add_struct_op_block(UNPACK, pos());
        assert_eq!(struct_name(p.logic.block(&id).unwrap()), Some("recipe"));
    }
}
