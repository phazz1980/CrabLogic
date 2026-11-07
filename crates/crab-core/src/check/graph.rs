use super::*;

/// Valid link between two known blocks.
#[derive(Debug, Clone, Copy)]
pub(super) struct Edge {
    /// Index in [`Logic::links`]; `None` for the implicit order of variable writes
    /// before reads.
    pub(super) link: Option<usize>,
    pub(super) from: usize,
    pub(super) to: usize,
}

/// Resolves link ends to blocks and ports.
pub(super) struct Resolver<'a, 'l> {
    pub(super) logic: &'l Logic,
    pub(super) defs: &'l [Option<&'a BlockDef>],
    /// Ids of blocks with a known definition.
    pub(super) index: HashMap<&'l str, usize>,
}

impl<'a> Resolver<'a, '_> {
    /// Returns the block index and port of one link end, or reports why it is invalid.
    pub(super) fn end(
        &self,
        r: &PortRef,
        is_input: bool,
        link: usize,
        what: &str,
        report: &mut Report,
    ) -> Option<(usize, &'a PortDef)> {
        let Some(&bi) = self.index.get(r.block.as_str()) else {
            // A known id with an unknown type was already reported with the block.
            if self.logic.block(&r.block).is_none() {
                report.link_error(
                    DiagKind::LinkEnds,
                    link,
                    format!(
                        "{what}: нет блока «{}» (веб-виджет мог быть удалён в конструкторе)",
                        r.block
                    ),
                );
            }
            return None;
        };
        let def = self.defs[bi]?;
        let (port, wrong) = if is_input {
            (def.input(&r.port), def.output(&r.port))
        } else {
            (def.output(&r.port), def.input(&r.port))
        };
        let params = &self.logic.blocks[bi].params;
        let msg = match (port, wrong) {
            (Some(p), _) if !def.enabled(p.when.as_deref(), params).unwrap_or(true) => format!(
                "{what}: порт «{r}» отключён параметрами блока ({})",
                p.when.as_deref().unwrap_or_default()
            ),
            (Some(p), _) => return Some((bi, p)),
            (None, Some(_)) if is_input => {
                format!("{what}: «{r}» — это выход, а связь должна вести во вход")
            }
            (None, Some(_)) => {
                format!("{what}: «{r}» — это вход, а связь должна начинаться с выхода")
            }
            (None, None) => format!(
                "{what}: у блока «{}» ({}) нет порта «{}»",
                r.block, def.info.kind, r.port
            ),
        };
        report.link_error(DiagKind::LinkEnds, link, msg);
        None
    }
}

/// Checks link ends, directions, types and duplicate sources. Returns valid edges.
pub(super) fn check_links(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    report: &mut Report,
) -> Vec<Edge> {
    let index = logic
        .blocks
        .iter()
        .enumerate()
        .filter(|(i, _)| defs[*i].is_some())
        .map(|(i, b)| (b.id.as_str(), i))
        .collect();
    let resolver = Resolver { logic, defs, index };
    let mut sources: HashMap<(usize, &str), usize> = HashMap::new();
    let mut edges = Vec::new();

    for (li, link) in logic.links.iter().enumerate() {
        let Link { from, to } = link;
        let what = format!("связь {from} → {to}");
        let src = resolver.end(from, false, li, &what, report);
        let dst = resolver.end(to, true, li, &what, report);
        let (Some((fi, fp)), Some((ti, tp))) = (src, dst) else {
            continue;
        };
        if !fp.ty.can_feed(tp.ty) {
            report.link_error(
                DiagKind::LinkType,
                li,
                format!(
                    "{what}: тип выхода {} не подходит ко входу {}",
                    type_name(fp.ty),
                    type_name(tp.ty)
                ),
            );
            continue;
        }
        if let Some(prev) = sources.insert((ti, to.port.as_str()), li) {
            let prev_from = &logic.links[prev].from;
            report.link_error(
                DiagKind::InputBusy,
                li,
                format!("{what}: на вход «{to}» уже подан «{prev_from}»"),
            );
            continue;
        }
        edges.push(Edge {
            link: Some(li),
            from: fi,
            to: ti,
        });
    }
    edges
}

/// Checks variable declarations and the blocks that access them. Returns implicit
/// edges "write before read" for the execution order.
pub(super) fn check_vars(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    report: &mut Report,
) -> Vec<Edge> {
    let mut names = HashMap::new();
    let mut storages = HashMap::new();
    for var in &logic.vars {
        let name = var.name.as_str();
        if !crate::vars::is_valid_var_name(name) {
            report.push(
                DiagKind::VariableName,
                Severity::Error,
                None,
                None,
                format!("недопустимое имя переменной «{name}»"),
            );
        }
        if names.insert(name, ()).is_some() {
            report.push(
                DiagKind::VariableName,
                Severity::Error,
                None,
                None,
                format!("переменная «{name}» объявлена дважды"),
            );
        }
        let storage = crate::vars::storage_name(name);
        if let Some(other) = storages
            .insert(storage.clone(), name)
            .filter(|&o| o != name)
        {
            report.push(DiagKind::VariableName,
                Severity::Error,
                None,
                None,
                format!("у переменных «{other}» и «{name}» совпало имя в скетче {storage}: переименуйте одну"),
            );
        }
        if var.init.as_ref().is_some_and(|v| !var.ty.accepts(v)) {
            report.push(
                DiagKind::VariableType,
                Severity::Error,
                None,
                None,
                format!(
                    "начальное значение переменной «{name}» не подходит к типу {}",
                    var.ty.name()
                ),
            );
        }
    }

    let mut writers: HashMap<&str, usize> = HashMap::new();
    let mut readers: Vec<(&str, usize)> = Vec::new();
    for (i, block) in logic.blocks.iter().enumerate() {
        if defs[i].is_none() {
            continue;
        }
        let Some(name) = block.var_name() else {
            continue;
        };
        if logic.var(name).is_none() {
            report.block_error(
                DiagKind::VariableLink,
                &block.id,
                format!("блок «{}»: нет переменной «{name}»", block.id),
            );
            continue;
        }
        if block.kind == crate::vars::VAR_SET {
            if let Some(&prev) = writers.get(name) {
                report.block_error(
                    DiagKind::VariableWriters,
                    &block.id,
                    format!(
                        "переменная «{name}» уже записывается блоком «{}»",
                        logic.blocks[prev].id
                    ),
                );
            } else {
                writers.insert(name, i);
            }
        } else {
            readers.push((name, i));
        }
    }
    for var in &logic.vars {
        let used = logic.blocks.iter().any(|b| {
            b.var_name() == Some(var.name.as_str()) || b.bound.values().any(|v| *v == var.name)
        });
        if !used {
            report.push(
                DiagKind::VariableUnused,
                Severity::Warning,
                None,
                None,
                format!("переменная «{}» не используется", var.name),
            );
        }
    }
    readers
        .into_iter()
        .filter_map(|(name, to)| {
            writers.get(name).map(|&from| Edge {
                link: None,
                from,
                to,
            })
        })
        .collect()
}

pub(super) fn check_layout(logic: &Logic, layout: &Layout, report: &mut Report) {
    for id in layout.blocks.keys() {
        if logic.block(id).is_none() {
            report.push(
                DiagKind::LayoutOrphan,
                Severity::Warning,
                None,
                None,
                format!("в разметке проекта есть координаты несуществующего блока «{id}»"),
            );
        }
    }
}

/// Copies of tags: the tag must exist, be no copy itself, have the same type and a
/// board pin. Returns edges "tag before the copy" for copies of inputs, so that the
/// copy gives the value read in the same scan.
pub(super) fn check_tags(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    report: &mut Report,
) -> Vec<Edge> {
    let mut edges = Vec::new();
    for (i, block) in logic.blocks.iter().enumerate() {
        let Some(tag) = &block.tag else { continue };
        let id = block.id.as_str();
        let target = logic.blocks.iter().position(|b| b.id == *tag);
        let problem = match target.map(|t| (t, &logic.blocks[t])) {
            None => Some(format!("копия «{id}»: нет тега «{tag}»")),
            Some((_, t)) if t.tag.is_some() => {
                Some(format!("копия «{id}»: «{tag}» сам копия, ссылаться надо на тег"))
            }
            Some((_, t)) if t.kind != block.kind => Some(format!(
                "копия «{id}»: тип «{}» не совпадает с типом тега «{tag}» («{}»)",
                block.kind, t.kind
            )),
            Some((t, tb))
                if !crate::project::is_web_kind(&tb.kind)
                    && !defs[t].is_some_and(|d| d.params.iter().any(|p| p.kind == ParamKind::Pin)) =>
            {
                Some(format!("копия «{id}»: у «{tag}» нет вывода платы, копировать можно только входы и выходы"))
            }
            Some((t, _)) => {
                if defs[i].is_some_and(|d| !d.outputs.is_empty()) {
                    edges.push(Edge {
                        link: None,
                        from: t,
                        to: i,
                    });
                }
                None
            }
        };
        if let Some(message) = problem {
            report.block_error(DiagKind::Tag, id, message);
        }
    }
    edges
}

/// Builds the execution order. Feedback loops are allowed only through a block with
/// memory: links into such blocks inside a loop take the previous scan's value.
pub(super) fn order_blocks(
    logic: &Logic,
    defs: &[Option<&BlockDef>],
    edges: &[Edge],
    report: &mut Report,
) {
    let n = logic.blocks.len();
    let has_memory = |i: usize| defs[i].is_some_and(|d| d.info.memory);

    let mut succ = vec![Vec::new(); n];
    for e in edges {
        succ[e.from].push(e.to);
    }
    let scc = strongly_connected(&succ);
    let in_loop = |e: &Edge| scc[e.from] == scc[e.to];

    let mut indegree = vec![0usize; n];
    let mut active = vec![Vec::new(); n];
    for e in edges {
        if in_loop(e) && has_memory(e.to) {
            report.feedback.extend(e.link);
        } else {
            active[e.from].push(e.to);
            indegree[e.to] += 1;
        }
    }

    // Kahn's algorithm; ties are broken by declaration order for stable output.
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&i| indegree[i] == 0).map(Reverse).collect();
    while let Some(Reverse(i)) = ready.pop() {
        report.order.push(i);
        for &j in &active[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                ready.push(Reverse(j));
            }
        }
    }

    if report.order.len() < n {
        // Report only the loops themselves; blocks downstream of a loop are stuck too,
        // but that is a consequence, not a separate problem.
        let mut stuck: Vec<usize> = (0..n)
            .filter(|&i| indegree[i] > 0 && succ[i].iter().any(|&j| scc[j] == scc[i]))
            .collect();
        stuck.sort_by_key(|&i| (scc[i], i));
        for group in stuck.chunk_by(|&a, &b| scc[a] == scc[b]) {
            let ids: Vec<&str> = group.iter().map(|&i| logic.blocks[i].id.as_str()).collect();
            report.block_error(
                DiagKind::Loop,
                ids[0],
                format!(
                    "цикл без блока с памятью: {} — разорвите его триггером или таймером",
                    ids.join(", ")
                ),
            );
        }
    }
}

/// Tarjan's algorithm. Returns the component number of each node.
pub(super) fn strongly_connected(succ: &[Vec<usize>]) -> Vec<usize> {
    struct State<'a> {
        succ: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        comp: Vec<usize>,
        next_index: usize,
        next_comp: usize,
    }

    pub(super) fn visit(s: &mut State, v: usize) {
        s.index[v] = Some(s.next_index);
        s.low[v] = s.next_index;
        s.next_index += 1;
        s.stack.push(v);
        s.on_stack[v] = true;
        for k in 0..s.succ[v].len() {
            let w = s.succ[v][k];
            match s.index[w] {
                None => {
                    visit(s, w);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(wi) if s.on_stack[w] => s.low[v] = s.low[v].min(wi),
                Some(_) => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            while let Some(w) = s.stack.pop() {
                s.on_stack[w] = false;
                s.comp[w] = s.next_comp;
                if w == v {
                    break;
                }
            }
            s.next_comp += 1;
        }
    }

    let n = succ.len();
    let mut s = State {
        succ,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        comp: vec![0; n],
        next_index: 0,
        next_comp: 0,
    };
    for v in 0..n {
        if s.index[v].is_none() {
            visit(&mut s, v);
        }
    }
    s.comp
}
