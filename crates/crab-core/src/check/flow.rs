use super::*;

/// Warns about a `float` variable that is counted up through `add`/`sub` (`n = n + 1`):
/// `float` is exact only up to 2^24, then the count stops. A float sum of measured values
/// is fine, so only an accumulator fed back through a variable is reported.
pub(super) fn check_float_accumulators(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    report: &mut Report,
) {
    let n = logic.blocks.len();
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for link in &logic.links {
        let from = logic.blocks.iter().position(|b| b.id == link.from.block);
        let to = logic.blocks.iter().position(|b| b.id == link.to.block);
        if let (Some(f), Some(t)) = (from, to) {
            next[f].push(t);
        }
    }
    for var in &logic.vars {
        if !matches!(
            var.ty,
            crate::vars::VarType::Float | crate::vars::VarType::Double
        ) {
            continue;
        }
        let of_var = |kind: &str| -> Vec<usize> {
            (0..n)
                .filter(|&i| defs[i].is_some())
                .filter(|&i| {
                    logic.blocks[i].kind == kind
                        && logic.blocks[i].var_name() == Some(var.name.as_str())
                })
                .collect()
        };
        let setters = of_var(crate::vars::VAR_SET);
        // States are (block, passed an add/sub on the way).
        let mut seen = vec![[false; 2]; n];
        let mut stack: Vec<(usize, bool)> = of_var(crate::vars::VAR_GET)
            .into_iter()
            .map(|i| (i, false))
            .collect();
        let mut found = false;
        while let Some((i, counted)) = stack.pop() {
            if std::mem::replace(&mut seen[i][usize::from(counted)], true) {
                continue;
            }
            if counted && setters.contains(&i) {
                found = true;
                break;
            }
            let counted = counted || matches!(logic.blocks[i].kind.as_str(), "add" | "sub");
            stack.extend(next[i].iter().map(|&t| (t, counted)));
        }
        if found {
            report.push(
                DiagKind::FloatCounter,
                Severity::Warning,
                None,
                None,
                format!(
                    "переменная «{}» типа float считает через сложение: float точен только до 16 777 216, дальше счёт встанет; для счётчика возьмите int",
                    var.name
                ),
            );
        }
    }
}

/// Real type of every output once `number` and `same` outputs follow their inputs, by
/// (block index, port name). Blocks go in execution order; an input whose source
/// is not known yet (a feedback loop) counts as `float`.
pub fn resolved_outputs<'a>(
    logic: &'a Logic,
    defs: &[Option<&'a BlockDef>],
    order: &[usize],
) -> HashMap<(usize, &'a str), SignalType> {
    let mut types: HashMap<(usize, &str), SignalType> = HashMap::new();
    for &bi in order {
        let (block, Some(def)) = (&logic.blocks[bi], defs[bi]) else {
            continue;
        };
        let numbers: Vec<SignalType> = def
            .active_inputs(&block.params)
            .filter(|p| p.ty == SignalType::Number)
            .map(|p| input_type(logic, defs, &types, block, p))
            .collect();
        let same = same_type(logic, defs, &types, block, def).unwrap_or(SignalType::Float);
        for out in def.active_outputs(&block.params) {
            let ty = match out.ty {
                SignalType::Number => SignalType::widest(numbers.iter().copied()),
                SignalType::Same => same,
                ty => ty,
            };
            types.insert((bi, out.name.as_str()), ty);
        }
    }
    types
}

/// Port types by (block index, port name).
pub type PortTypes = HashMap<(usize, String), SignalType>;

/// Real types of all active ports of every block, by (block index, port name): `(inputs, outputs)`.
/// A `number` or `same` input has the type of the signal that feeds it (a declared type otherwise).
pub fn port_types(logic: &Logic, lib: &Library, order: &[usize]) -> (PortTypes, PortTypes) {
    let defs: Vec<Option<&BlockDef>> = logic
        .blocks
        .iter()
        .map(|b| lib.resolve(logic, b).map(|l| &l.def))
        .collect();
    let outs = resolved_outputs(logic, &defs, order);
    let mut ins = HashMap::new();
    for (bi, block) in logic.blocks.iter().enumerate() {
        let Some(def) = defs[bi] else { continue };
        for port in def.active_inputs(&block.params) {
            let ty = match port.ty {
                SignalType::Number | SignalType::Same | SignalType::Any => {
                    input_type(logic, &defs, &outs, block, port)
                }
                ty => ty,
            };
            ins.insert((bi, port.name.clone()), ty);
        }
    }
    let outs = outs
        .into_iter()
        .map(|((bi, name), ty)| ((bi, name.to_owned()), ty))
        .collect();
    (ins, outs)
}

/// Type shared by the `same` inputs of a block: of the fed ones (link, variable or
/// constant), or of the defaults when none is fed; `float` for a block without them.
/// `Err` with two types that cannot meet. `types` holds the outputs resolved so far.
pub fn same_type(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    types: &HashMap<(usize, &str), SignalType>,
    block: &crate::Block,
    def: &BlockDef,
) -> Result<SignalType, (SignalType, SignalType)> {
    let ports: Vec<_> = def
        .active_inputs(&block.params)
        .filter(|p| p.ty == SignalType::Same)
        .collect();
    let fed = |p: &&&crate::PortDef| {
        block.bound.contains_key(&p.name)
            || block.consts.contains_key(&p.name)
            || logic
                .links
                .iter()
                .any(|l| l.to.block == block.id && l.to.port == p.name)
    };
    let real = |p: &&crate::PortDef| input_type(logic, defs, types, block, p);
    let shared = match SignalType::common(ports.iter().filter(fed).map(real))? {
        Some(ty) => Some(ty),
        None => SignalType::common(ports.iter().map(real))?,
    };
    Ok(shared.unwrap_or(SignalType::Float))
}

/// Real type of the signal on an input port: of its link, variable or constant.
pub(super) fn input_type(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    types: &HashMap<(usize, &str), SignalType>,
    block: &crate::Block,
    port: &crate::PortDef,
) -> SignalType {
    let from_value = |v: &crate::ParamValue| SignalType::of_value(v);
    if let Some(var) = block.bound.get(&port.name).and_then(|v| logic.var(v)) {
        return var.ty.signal();
    }
    if let Some(link) = logic
        .links
        .iter()
        .find(|l| l.to.block == block.id && l.to.port == port.name)
    {
        let src = logic.blocks.iter().position(|b| b.id == link.from.block);
        return src
            .and_then(|i| {
                types
                    .get(&(i, link.from.port.as_str()))
                    .copied()
                    .or_else(|| {
                        defs[i]?
                            .output(&link.from.port)
                            .map(|o| o.ty)
                            .filter(|t| !matches!(t, SignalType::Number | SignalType::Same))
                    })
            })
            .unwrap_or(SignalType::Float);
    }
    block
        .consts
        .get(&port.name)
        .or(port.default.as_ref())
        .map_or(SignalType::Float, from_value)
}

/// A `number` output that turned out `float` cannot go to an `int` input: the
/// narrowing needs an explicit block. The same for a `same` output of the wrong type;
/// the `same` inputs of a block must share one type.
pub(super) fn check_number_flow(logic: &Logic, defs: &[Option<&BlockDef>], report: &mut Report) {
    let types = resolved_outputs(logic, defs, &report.order.clone());
    for (bi, block) in logic.blocks.iter().enumerate() {
        let Some(def) = defs[bi] else { continue };
        if let Err((a, b)) = same_type(logic, defs, &types, block, def) {
            let ports: Vec<&str> = def
                .active_inputs(&block.params)
                .filter(|p| p.ty == SignalType::Same)
                .map(|p| p.name.as_str())
                .collect();
            report.block_error(
                DiagKind::LinkType,
                &block.id,
                format!(
                    "блок «{}»: на входы {} поданы разные типы — {} и {}; нужен один тип",
                    block.id,
                    ports.join(", "),
                    type_name(a),
                    type_name(b)
                ),
            );
        }
    }
    for (li, link) in logic.links.iter().enumerate() {
        let (Some(fi), Some(ti)) = (
            logic.blocks.iter().position(|b| b.id == link.from.block),
            logic.blocks.iter().position(|b| b.id == link.to.block),
        ) else {
            continue;
        };
        let (Some(fd), Some(td)) = (defs[fi], defs[ti]) else {
            continue;
        };
        let (Some(out), Some(inp)) = (fd.output(&link.from.port), td.input(&link.to.port)) else {
            continue;
        };
        if !matches!(out.ty, SignalType::Number | SignalType::Same) {
            continue;
        }
        let real = types
            .get(&(fi, link.from.port.as_str()))
            .copied()
            .unwrap_or(SignalType::Float);
        if !real.can_feed(inp.ty) {
            report.link_error(
                DiagKind::LinkType,
                li,
                format!(
                    "связь {} → {}: выход получился {}, а вход ждёт {}; нужен блок преобразования",
                    link.from,
                    link.to,
                    type_name(real),
                    type_name(inp.ty)
                ),
            );
        }
    }
}

pub(super) fn type_name(ty: SignalType) -> &'static str {
    match ty {
        SignalType::Bool => "bool (логический)",
        SignalType::Int => "int (целый)",
        SignalType::Float => "float (дробный)",
        SignalType::String => "string (строка)",
        SignalType::Any => "any (любой)",
        SignalType::Number => "число (int или float)",
        SignalType::Same => "same (как у соседних входов)",
    }
}
