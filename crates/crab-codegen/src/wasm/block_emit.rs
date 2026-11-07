use super::*;

/// What a built-in block sees of the module builder while it writes its code.
pub(super) struct BlockEmit<'e, 'a, 'p> {
    pub(super) e: &'e mut Emitter<'a, 'p>,
    pub(super) inst: &'e Instance<'e>,
}

impl crab_blocks::WasmEmit for BlockEmit<'_, '_, '_> {
    fn input_ty(&self, i: usize) -> crab_blocks::WTy {
        self.e.input_ty(self.inst, i).into()
    }
    fn input_as(&mut self, i: usize, want: crab_blocks::WTy) {
        self.e.input_as(self.inst, i, want.into());
    }
    fn op(&mut self, i: I<'static>) {
        self.e.op(i);
    }
    fn c_i(&mut self, x: i32) {
        self.e.c_i(x);
    }
    fn c_f(&mut self, x: f32) {
        self.e.c_f(x);
    }
    fn flag(&mut self, i: usize) {
        self.e.flag(self.inst, i);
    }
    fn const_param(&self, i: usize) -> Result<f64, String> {
        self.e.const_param(self.inst, i).map_err(|e| e.to_string())
    }
    fn state(&mut self, ty: crab_blocks::WTy) -> u32 {
        self.e.global(ty.into(), &ParamValue::Int(0))
    }
    fn set_out(&mut self, i: usize, from: crab_blocks::WTy) {
        let connected = self
            .inst
            .lib
            .def
            .outputs
            .get(i)
            .is_some_and(|p| self.inst.outputs.contains_key(p.name.as_str()));
        if connected {
            self.e.set_out(self.inst, i, from.into());
        } else {
            self.e.op(I::Drop);
        }
    }
    fn state_init(&mut self, ty: crab_blocks::WTy, bits: i32) -> u32 {
        self.e.global(ty.into(), &ParamValue::Int(i64::from(bits)))
    }
    fn text_param(&self, i: usize) -> Result<String, String> {
        let name = self.inst.lib.def.params[i].name.as_str();
        if self.inst.param_vars.contains_key(name) {
            return Err(format!("параметр «{name}» из переменной пока не поддержан"));
        }
        Ok(self.e.text_param(self.inst, name))
    }
    fn when(&mut self, then: &mut dyn FnMut(&mut dyn crab_blocks::WasmEmit)) {
        self.e.op(I::If(BlockType::Empty));
        then(self);
        self.e.op(I::End);
    }
    fn branch(
        &mut self,
        then: &mut dyn FnMut(&mut dyn crab_blocks::WasmEmit),
        other: &mut dyn FnMut(&mut dyn crab_blocks::WasmEmit),
    ) {
        self.e.op(I::If(BlockType::Empty));
        then(self);
        self.e.op(I::Else);
        other(self);
        self.e.op(I::End);
    }
    fn now(&mut self) {
        self.e.op(I::GlobalGet(GLOBAL_NOW));
    }
    fn param_as(&mut self, i: usize, want: crab_blocks::WTy) {
        self.e.param_as(self.inst, i, want.into());
    }
    fn block_key(&self) -> i32 {
        crate::retain::fnv(&format!("block:{}", self.inst.block.id)) as i32
    }
    fn output_connected(&self, i: usize) -> bool {
        self.inst
            .lib
            .def
            .outputs
            .get(i)
            .is_some_and(|p| self.inst.outputs.contains_key(p.name.as_str()))
    }
    fn out_get(&mut self, i: usize) {
        self.e.out_get(self.inst, i);
    }
    fn retain(&mut self, op: i32, key: i32, value: &mut dyn FnMut(&mut dyn crab_blocks::WasmEmit)) {
        self.e.c_i(op);
        self.e.c_i(key);
        value(self);
        self.e.hw_call(HW_RETAIN);
    }
    fn math(&mut self, op: i32, a: u32, b: u32) {
        self.e.c_i(op);
        self.e.op(I::LocalGet(a));
        self.e.op(I::LocalGet(b));
        self.e.hw_call(HW_MATH);
    }
    fn tmp(&mut self, ty: crab_blocks::WTy) -> u32 {
        self.e.tmp(ty.into())
    }
    fn choose(
        &mut self,
        ty: crab_blocks::WTy,
        then: &mut dyn FnMut(&mut dyn crab_blocks::WasmEmit),
        other: &mut dyn FnMut(&mut dyn crab_blocks::WasmEmit),
    ) {
        let ty: Ty = ty.into();
        self.e.op(I::If(BlockType::Result(ty.val())));
        then(self);
        self.e.op(I::Else);
        other(self);
        self.e.op(I::End);
    }
}
