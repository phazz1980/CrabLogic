//! Putting a block into an existing link (`specs/link-insert`).

use crate::edit::EditError;
use crate::{Library, PortRef, Project};

/// What [`Project::insert_into_link`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Insert {
    /// The link was cut: `source → block.input` and `block.output → target`.
    Spliced { input: String, output: String },
    /// The block has no input that fits: `block.output → target` replaces the link.
    Replaced { output: String },
    /// The block has no output that fits: the link stays and `source → block.input` is added.
    Branch { input: String },
}

impl Project {
    /// Puts the block `id`, already in the schema, into the link that ends at `to`.
    ///
    /// The input is the first free one (no link, constant or binding) that accepts the type of the
    /// link source; the output is the first one the target input accepts. Nothing changes on error.
    pub fn insert_into_link(
        &mut self,
        lib: &Library,
        id: &str,
        to: &PortRef,
    ) -> Result<Insert, EditError> {
        let link = self
            .logic
            .links
            .iter()
            .find(|l| &l.to == to)
            .cloned()
            .ok_or_else(|| EditError::NoLink(to.clone()))?;
        if link.from.block == id || link.to.block == id {
            return Err(EditError::InsertOwnLink(id.to_owned()));
        }
        let source_ty = self
            .block_def(lib, &link.from.block)?
            .output(&link.from.port)
            .ok_or_else(|| EditError::NoOutput {
                block: link.from.block.clone(),
                port: link.from.port.clone(),
            })?
            .ty;
        let target_ty = self
            .block_def(lib, &to.block)?
            .input(&to.port)
            .ok_or_else(|| EditError::NoInput {
                block: to.block.clone(),
                port: to.port.clone(),
            })?
            .ty;
        let block = self
            .logic
            .block(id)
            .ok_or_else(|| EditError::NoBlock(id.to_owned()))?;
        let def = self.block_def(lib, id)?;
        let input = def
            .inputs
            .iter()
            .find(|p| {
                source_ty.can_feed(p.ty)
                    && !block.consts.contains_key(&p.name)
                    && !block.bound.contains_key(&p.name)
                    && !self
                        .logic
                        .links
                        .iter()
                        .any(|l| l.to.block == id && l.to.port == p.name)
            })
            .map(|p| p.name.clone());
        let output = def
            .outputs
            .iter()
            .find(|p| p.ty.can_feed(target_ty))
            .map(|p| p.name.clone());
        match (input, output) {
            (Some(input), Some(output)) => {
                self.connect(lib, PortRef::new(id, &output), to.clone())?;
                self.connect(lib, link.from, PortRef::new(id, &input))?;
                Ok(Insert::Spliced { input, output })
            }
            (None, Some(output)) => {
                self.connect(lib, PortRef::new(id, &output), to.clone())?;
                Ok(Insert::Replaced { output })
            }
            (Some(input), None) => {
                self.connect(lib, link.from, PortRef::new(id, &input))?;
                Ok(Insert::Branch { input })
            }
            (None, None) => Err(EditError::InsertNoPort(id.to_owned())),
        }
    }
}
