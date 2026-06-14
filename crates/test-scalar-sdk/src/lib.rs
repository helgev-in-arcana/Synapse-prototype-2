//! SDK で書いた const / add / subfold。raw 版 test-scalar-plugin と等価。
//! unsafe ・グローバル・negotiate・SynValue 変換は一切現れない（SDK が隠蔽）。

use synapse_sdk::prelude::*;

/// 定数 float を出力。内部パラメータ value を save/load_state で永続化。
struct Const {
    value: f32,
    out: OutPort<f32>,
}
impl Default for Const {
    fn default() -> Self {
        Self {
            value: 1.0,
            out: OutPort::default(),
        }
    }
}
impl Node for Const {
    const URI: &'static CStr = c"synapse.test.const";
    const DISPLAY_NAME: &'static CStr = c"Const Float";

    fn declare(&mut self, d: &mut Declarer) {
        self.out = d.output::<f32>(c"out", c"Out");
    }
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<()> {
        ctx.set(self.out, self.value);
        Ok(())
    }
    fn save_state(&self) -> Option<Vec<u8>> {
        Some(self.value.to_ne_bytes().to_vec())
    }
    fn load_state(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() < 4 {
            return Err(Error::BadState);
        }
        self.value = f32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        Ok(())
    }
}

/// out = a + b。a 既定 0.0 / b 既定 4.0。
#[derive(Default)]
struct Add {
    a: InPort<f32>,
    b: InPort<f32>,
    out: OutPort<f32>,
}
impl Node for Add {
    const URI: &'static CStr = c"synapse.test.add";
    const DISPLAY_NAME: &'static CStr = c"Add";

    fn declare(&mut self, d: &mut Declarer) {
        self.a = d.input::<f32>(c"a", c"A", 0.0);
        self.b = d.input::<f32>(c"b", c"B", 4.0);
        self.out = d.output::<f32>(c"out", c"Out");
    }
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<()> {
        let a = ctx.get(self.a).unwrap_or(0.0);
        let b = ctx.get(self.b).unwrap_or(0.0);
        ctx.set(self.out, a + b);
        Ok(())
    }
}

/// fan-in 検証: out = in[0] - in[1] - ... - in[N-1]（順序依存）。
#[derive(Default)]
struct SubFold {
    inputs: MultiInPort<f32>,
    out: OutPort<f32>,
}
impl Node for SubFold {
    const URI: &'static CStr = c"synapse.test.subfold";
    const DISPLAY_NAME: &'static CStr = c"Subtract Fold (fan-in)";

    fn declare(&mut self, d: &mut Declarer) {
        self.inputs = d.input_multi::<f32>(c"in", c"In");
        self.out = d.output::<f32>(c"out", c"Out");
    }
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<()> {
        let n = ctx.link_count(self.inputs);
        let mut acc = 0.0f32;
        if n > 0 {
            acc = ctx.get_link(self.inputs, 0).unwrap_or(0.0);
            for l in 1..n {
                acc -= ctx.get_link(self.inputs, l).unwrap_or(0.0);
            }
        }
        ctx.set(self.out, acc);
        Ok(())
    }
}

synapse_module! {
    uri: c"synapse.test",
    version: c"0.1.0",
    types: [f32],
    nodes: [Const, Add, SubFold],
}
