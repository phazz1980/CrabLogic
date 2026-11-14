use super::*;
use crate::cost::costs_for;
use crate::pins::Chip;
use crate::project::PERIODS_MS;

/// Share of the period a step may take before the check warns.
const WARN_SHARE: f32 = 0.7;

/// The cycle period must be one of the supported values; for each sheet the estimated step
/// (`crate::cost`) must stay under 70 % of the period, otherwise a warning.
pub(super) fn check_cycle(project: &Project, report: &mut Report) {
    let period = project.period_ms();
    if !PERIODS_MS.contains(&period) {
        report.push(
            DiagKind::CyclePeriod,
            Severity::Error,
            None,
            None,
            format!("период цикла может быть 1, 2, 5 или 10 мс, а в проекте {period} мс"),
        );
        return;
    }
    let costs = costs_for(Chip::of_board(&project.config.board));
    let mut sheets: std::collections::BTreeMap<u32, Vec<&str>> = Default::default();
    for b in &project.logic.blocks {
        let sheet = project.layout.blocks.get(&b.id).map_or(1, |p| p.sheet);
        sheets.entry(sheet).or_default().push(b.kind.as_str());
    }
    for (sheet, kinds) in sheets {
        let us = costs.sheet_us(kinds.iter().copied());
        if us > WARN_SHARE * period as f32 * 1000.0 {
            let name = match project.layout.sheet(sheet) {
                Some(s) if !s.name.is_empty() => s.name.clone(),
                _ => format!("Лист {sheet}"),
            };
            report.push(
                DiagKind::StepOverPeriod,
                Severity::Warning,
                None,
                None,
                format!(
                    "лист «{name}»: шаг ≈ {} мкс при периоде цикла {period} мс (оценка)",
                    us.round() as u32
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(fqbn: &str, period: u32, kinds: &[&str]) -> Project {
        let mut src = format!("[project]\nname = \"p\"\n[board]\nfqbn = \"{fqbn}\"\n");
        if period != 10 {
            src += &format!("[run]\nperiod_ms = {period}\n");
        }
        let mut blocks = String::new();
        for (i, k) in kinds.iter().enumerate() {
            blocks += &format!("[[block]]\nid = \"b{i}\"\ntype = \"{k}\"\n");
        }
        Project {
            path: Default::default(),
            config: toml::from_str(&src).unwrap(),
            logic: toml::from_str(&blocks).unwrap(),
            layout: Default::default(),
        }
    }

    fn kinds_of(project: &Project) -> Vec<(DiagKind, Severity, String)> {
        let mut report = Report::default();
        check_cycle(project, &mut report);
        report
            .diagnostics
            .into_iter()
            .map(|d| (d.kind, d.severity, d.message))
            .collect()
    }

    #[test]
    fn logic_100_at_10ms_is_quiet() {
        let p = project("esp32:esp32:esp32", 10, &vec!["and"; 100]);
        assert!(kinds_of(&p).is_empty());
    }

    #[test]
    fn timers_500_at_1ms_warn() {
        let p = project("esp32:esp32:esp32", 1, &vec!["timer_on"; 500]);
        let d = kinds_of(&p);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, DiagKind::StepOverPeriod);
        assert_eq!(d[0].1, Severity::Warning);
        assert!(d[0].2.contains("при периоде цикла 1 мс"), "{}", d[0].2);
        // The same sheet at 10 ms is fine.
        let p = project("esp32:esp32:esp32", 10, &vec!["timer_on"; 500]);
        assert!(kinds_of(&p).is_empty());
    }

    #[test]
    fn c3_is_slower_than_esp32_on_logic() {
        // 600 logic blocks: ~120 us on ESP32 (quiet at 1 ms), ~820 us on C3 (over 70 %).
        let kinds = vec!["and"; 600];
        assert!(kinds_of(&project("esp32:esp32:esp32", 1, &kinds)).is_empty());
        assert_eq!(
            kinds_of(&project("esp32:esp32:esp32c3", 1, &kinds)).len(),
            1
        );
    }

    #[test]
    fn bad_period_is_an_error() {
        let d = kinds_of(&project("esp32:esp32:esp32", 3, &["and"]));
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, DiagKind::CyclePeriod);
        assert_eq!(d[0].1, Severity::Error);
    }
}
