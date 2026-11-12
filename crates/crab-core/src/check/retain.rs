use super::*;

/// Warns when a retained variable gets its value straight from an output marked `fast`
/// (`block.toml`): the value changes all the time and the flash would wear out.
/// Covers `var_set` blocks and variables bound to an output; tag copies are not followed.
pub(super) fn check_retain_fast(logic: &Logic, defs: &[Option<&BlockDef>], report: &mut Report) {
    let fast_out = |from: &PortRef| -> bool {
        logic
            .blocks
            .iter()
            .position(|b| b.id == from.block)
            .and_then(|i| defs[i])
            .and_then(|d| d.output(&from.port))
            .is_some_and(|p| p.fast)
    };
    let mut found: Vec<(String, PortRef)> = Vec::new();
    for (i, block) in logic.blocks.iter().enumerate() {
        if defs[i].is_none() {
            continue;
        }
        if block.kind == crate::vars::VAR_SET {
            let Some(name) = block.var_name() else {
                continue;
            };
            for link in logic.links.iter().filter(|l| l.to.block == block.id) {
                if link.to.port == "in" && fast_out(&link.from) {
                    found.push((name.to_owned(), link.from.clone()));
                }
            }
        }
        for (port, var) in &block.bound {
            if defs[i].is_some_and(|d| d.output(port).is_some_and(|p| p.fast)) {
                found.push((var.clone(), PortRef::new(block.id.clone(), port.clone())));
            }
        }
    }
    for (name, from) in found {
        if !logic.var(&name).is_some_and(|v| v.retain) {
            continue;
        }
        report.push(
            DiagKind::RetainFast,
            Severity::Warning,
            Some(&from.block),
            None,
            format!(
                "Переменная «{name}» сохраняется во флеш, а пишет в неё часто меняющийся выход «{}.{}»: \
                 флеш будет изнашиваться. Сохраняйте по событию блоком flash_store или задайте период сохранения.",
                from.block, from.port
            ),
        );
    }
}
