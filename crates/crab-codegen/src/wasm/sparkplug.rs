//! Sparkplug B in modules (`specs/pubsub-bus`, step 2): the master owns the MQTT client and
//! writes the protobuf payloads (`master.ino`); a module only describes the node and hands
//! over the metric values through ops 30..33 of the `net` import.
//!
//! * op 30 setup: a = 1 group, 2 node id (b, c pointer, length), 4 enable (b), 63 setup complete
//! * op 31 declare a metric: a, b name (pointer, length), c type (0 bool, 1 int, 2 float);
//!   returns the slot, -1 when the table is full
//! * op 32 metric value: a = slot, b = the bits of the value
//! * op 33 read: a = field (0 online, 1 births sent)

use super::text::A;
use super::*;

impl Emitter<'_, '_> {
    pub(super) fn sparkplug_block(&mut self, inst: &Instance) -> Result<bool, WasmError> {
        if matches!(
            inst.block.kind.as_str(),
            "sparkplug_node" | "sparkplug_metric"
        ) {
            self.extras.insert("sparkplug");
        }
        match inst.block.kind.as_str() {
            "sparkplug_node" => self.sparkplug_node(inst)?,
            "sparkplug_metric" => self.sparkplug_metric(inst)?,
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn sparkplug_text(&mut self, inst: &Instance, code: i32, name: &str) -> Result<(), WasmError> {
        if inst.param_vars.contains_key(name) {
            return Err(self.unsupported(
                inst,
                &format!("параметр «{name}» из переменной пока не поддержан"),
            ));
        }
        let text = self.text_param(inst, name);
        if text.is_empty() || text.contains(['+', '#', '/']) {
            return Err(
                self.unsupported(inst, &format!("параметр «{name}» пуст или содержит + # /"))
            );
        }
        let s = self.const_str(text.as_bytes());
        self.net_call4(
            30,
            [
                A::C(code),
                A::C(s.ptr as i32),
                A::C(text.len() as i32),
                A::C(0),
            ],
        );
        self.op(I::Drop);
        Ok(())
    }

    fn sparkplug_node(&mut self, inst: &Instance) -> Result<(), WasmError> {
        let done = self.state_i();
        self.op(I::GlobalGet(done));
        self.op(I::I32Eqz);
        self.op(I::If(BlockType::Empty));
        self.sparkplug_text(inst, 1, "group")?;
        self.sparkplug_text(inst, 2, "node")?;
        self.net_call4(30, [A::C(63), A::C(0), A::C(0), A::C(0)]);
        self.op(I::Drop);
        self.c_i(1);
        self.op(I::GlobalSet(done));
        self.op(I::End);
        let en = self.tmp_i();
        self.flag(inst, 0);
        self.op(I::LocalSet(en));
        self.net_call4(30, [A::C(4), A::L(en), A::C(0), A::C(0)]);
        self.op(I::Drop);
        if inst.outputs.contains_key("online") {
            self.net_call4(33, [A::C(0), A::C(0), A::C(0), A::C(0)]);
            self.set_named(inst, "online", Ty::I);
        }
        Ok(())
    }

    fn sparkplug_metric(&mut self, inst: &Instance) -> Result<(), WasmError> {
        let kind = match inst.inputs.get("value").map(|i| i.ty) {
            Some(SignalType::Bool) => 0,
            Some(SignalType::Float) => 2,
            Some(SignalType::String) => {
                return Err(self.unsupported(inst, "строковые метрики Sparkplug не поддержаны"))
            }
            _ => 1,
        };
        let text = self.text_param(inst, "name");
        if text.is_empty() || text.len() > 32 {
            return Err(self.unsupported(inst, "имя метрики: от 1 до 32 символов"));
        }
        let name = self.const_str(text.as_bytes());
        // The slot is asked from the master once; -1 until then.
        let (slot, asked) = (self.state_i(), self.state_i());
        self.op(I::GlobalGet(asked));
        self.op(I::I32Eqz);
        self.op(I::If(BlockType::Empty));
        self.net_call4(
            31,
            [
                A::C(name.ptr as i32),
                A::C(text.len() as i32),
                A::C(kind),
                A::C(0),
            ],
        );
        self.op(I::GlobalSet(slot));
        self.c_i(1);
        self.op(I::GlobalSet(asked));
        self.op(I::End);
        self.op(I::GlobalGet(slot));
        self.c_i(0);
        self.op(I::I32GeS);
        self.op(I::If(BlockType::Empty));
        let arg = if kind == 2 {
            let x = self.tmp_f();
            self.input_as(inst, 0, Ty::F);
            self.op(I::LocalSet(x));
            A::F(x)
        } else {
            let n = self.tmp_i();
            self.input_as(inst, 0, Ty::I);
            if kind == 0 {
                self.c_i(0);
                self.op(I::I32Ne);
            }
            self.op(I::LocalSet(n));
            A::L(n)
        };
        self.net_call4(32, [A::G(slot), arg, A::C(0), A::C(0)]);
        self.op(I::Drop);
        self.op(I::End);
        Ok(())
    }
}
