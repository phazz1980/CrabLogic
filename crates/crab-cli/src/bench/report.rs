//! The bench report: serde structs (`.toml` for machines), the `.md` table and `--compare`.
//! Nothing about Wi-Fi or MQTT settings lives here, so a report can be shared.

use std::fmt::Write;

use serde::{Deserialize, Serialize};

/// A change of a figure bigger than this (percent) is marked by `--compare`.
pub const MARK_PERCENT: f64 = 10.0;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    pub meta: Meta,
    #[serde(default, rename = "stand", skip_serializing_if = "Vec::is_empty")]
    pub stands: Vec<StandResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheets: Option<SheetsFit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart: Option<RestartTimes>,
    /// The Modbus TCP stand: what the PC saw while it polled the board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modbus: Option<ModbusResult>,
    /// Stands that did not run for a reason that is not theirs (no secrets, no AOT...).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub chip: String,
    /// `id=` of the master with the `rs-` prefix dropped.
    pub master: String,
    /// UTC, `YYYY-MM-DD HH:MM`.
    pub date: String,
    pub crablogic: String,
    #[serde(default)]
    pub aot_target: String,
    /// Seconds the module runs before the figures are taken, and the pause between two readings.
    pub warm_secs: u32,
    pub gap_secs: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StandResult {
    pub id: String,
    pub family: String,
    /// Blocks asked for.
    pub n: usize,
    /// Blocks the project really has (chain, generator, pins...).
    pub blocks: usize,
    /// Calls per step of a call stand.
    #[serde(default)]
    pub calls: usize,
    /// `interp` or `aot` as asked; what the board reports is in `actual_mode`.
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_mode: Option<String>,
    /// `ok`, `skipped` or `failed`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps_per_s: Option<f64>,
    /// Mean step, from the growth of the board's sum of step times over the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_us: Option<f64>,
    /// Mean of the `last` step readings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_us: Option<f64>,
    /// Longest step in the window (the board's maximum is reset after the warm-up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_us: Option<u32>,
    /// Longest gap between two steps in the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_us: Option<u32>,
    /// Biggest minus smallest `last` reading of the window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread_us: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub largest: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_free: Option<u32>,
    /// Heap the module takes and the code part of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_mem: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_code: Option<u32>,
}

impl StandResult {
    pub fn failed(id: &str, family: &str, n: usize, mode: &str, reason: String) -> Self {
        Self {
            id: id.to_owned(),
            family: family.to_owned(),
            n,
            mode: mode.to_owned(),
            status: "failed".into(),
            reason: Some(reason),
            ..Self::default()
        }
    }

    pub fn skipped(id: &str, family: &str, n: usize, mode: &str, reason: String) -> Self {
        Self {
            status: "skipped".into(),
            ..Self::failed(id, family, n, mode, reason)
        }
    }

    pub fn is_ok(&self) -> bool {
        self.status == "ok"
    }
}

/// Step figures of the sheet while the PC polls it (the idle ones are in the stand's own row).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StepFigures {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_us: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_us: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_us: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread_us: Option<u32>,
}

/// Modbus TCP polling of the board: the client side of the stand. The address of the board is
/// not kept: a report can be shared.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModbusResult {
    /// Requests made, and how many of them failed.
    pub requests: usize,
    pub errors: usize,
    /// Answer time of the good requests, milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mean_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub req_per_s: Option<f64>,
    /// The first error, or why there are no figures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Step figures taken while the polling went on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polled: Option<StepFigures>,
}

/// How many sheets of 10 blocks the board takes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SheetsFit {
    /// Sheets running at the end.
    pub fit: usize,
    /// Most sheets tried.
    pub tried: usize,
    /// Why more were not tried or did not go.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Free heap after each sheet was added.
    #[serde(default)]
    pub free_after: Vec<u32>,
}

/// Times of the two kinds of restart, milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RestartTimes {
    /// `swap`: the module stops and starts from the stored image, until its first step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_ms: Option<u32>,
    /// Reset of the board (the RTS line) until the first step of the module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Cost of one call of the master: (stand minus the empty stand) / calls.
#[derive(Debug, Clone, PartialEq)]
pub struct CallCost {
    pub family: String,
    pub calls: usize,
    pub us_per_call: f64,
}

impl BenchReport {
    fn find(&self, id: &str) -> Option<&StandResult> {
        self.stands.iter().find(|s| s.id == id && s.is_ok())
    }

    pub fn call_costs(&self) -> Vec<CallCost> {
        let Some(base) = self.find("empty_aot").and_then(|s| s.mean_us) else {
            return Vec::new();
        };
        self.stands
            .iter()
            .filter(|s| s.is_ok() && s.calls > 0)
            .filter_map(|s| {
                Some(CallCost {
                    family: s.family.clone(),
                    calls: s.calls,
                    us_per_call: (s.mean_us? - base) / s.calls as f64,
                })
            })
            .collect()
    }

    /// The network stand against the same sheet without the network.
    pub fn net_vs_plain(&self) -> Option<(&StandResult, &StandResult)> {
        let net = self
            .stands
            .iter()
            .find(|s| s.family == "net" && s.is_ok())?;
        let plain = self.find(&format!("float_{}_aot", net.n))?;
        Some((net, plain))
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn file_stem(&self) -> String {
        let stamp: String = self
            .meta
            .date
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '-' || *c == ' ' || *c == ':')
            .collect::<String>()
            .replace(' ', "-")
            .replace(':', "");
        format!("{}-{}-{stamp}", self.meta.chip, self.meta.master)
    }

    pub fn to_markdown(&self) -> String {
        let mut o = String::new();
        let m = &self.meta;
        let _ = writeln!(
            o,
            "# Замеры производительности: {} / мастер {}\n",
            m.chip, m.master
        );
        let _ = writeln!(
            o,
            "Дата: {} (UTC), CrabLogic {}, процессор AOT: {}. Модуль работает {} с до замера, \
             три чтения `list` через {} с. «—» — нет данных.\n",
            m.date,
            m.crablogic,
            if m.aot_target.is_empty() {
                "—"
            } else {
                &m.aot_target
            },
            m.warm_secs,
            m.gap_secs
        );
        self.step_table(&mut o);
        self.calls_table(&mut o);
        self.net_table(&mut o);
        self.modbus_table(&mut o);
        self.memory_table(&mut o);
        self.times_table(&mut o);
        if !self.notes.is_empty() {
            let _ = writeln!(o, "\n## Примечания\n");
            for n in &self.notes {
                let _ = writeln!(o, "- {n}");
            }
        }
        o
    }

    fn step_table(&self, o: &mut String) {
        let rows: Vec<&StandResult> = self
            .stands
            .iter()
            .filter(|s| matches!(s.family.as_str(), "logic" | "float" | "timers" | "text"))
            .collect();
        if rows.is_empty() {
            return;
        }
        let _ = writeln!(o, "## Шаг листа, мкс\n");
        let _ = writeln!(
            o,
            "| Лист | Блоков | Режим | Средний | Последний | Максимум | Пауза между шагами (макс.) | Разброс | Шагов/с |"
        );
        let _ = writeln!(o, "|---|---:|---|---:|---:|---:|---:|---:|---:|");
        for s in rows {
            if !s.is_ok() {
                let _ = writeln!(
                    o,
                    "| {} | {} | {} | не замерено: {} | | | | | |",
                    s.family,
                    s.n,
                    s.mode,
                    s.reason.as_deref().unwrap_or("—")
                );
                continue;
            }
            let _ = writeln!(
                o,
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                s.family,
                s.blocks,
                s.actual_mode.as_deref().unwrap_or(&s.mode),
                f1(s.mean_us),
                f1(s.last_us),
                int(s.max_us),
                int(s.gap_us),
                int(s.spread_us),
                f1(s.steps_per_s)
            );
        }
        let _ = writeln!(o);
    }

    fn calls_table(&self, o: &mut String) {
        let costs = self.call_costs();
        let failed: Vec<&StandResult> = self
            .stands
            .iter()
            .filter(|s| s.family == "empty" || s.family_is_call())
            .filter(|s| !s.is_ok())
            .collect();
        if costs.is_empty() && failed.is_empty() {
            return;
        }
        let _ = writeln!(o, "## Цена вызовов мастера\n");
        let _ = writeln!(o, "| Вызов | Вызовов в шаге | мкс на вызов |");
        let _ = writeln!(o, "|---|---:|---:|");
        for c in costs {
            let _ = writeln!(o, "| {} | {} | {:.1} |", c.family, c.calls, c.us_per_call);
        }
        for s in failed {
            let _ = writeln!(
                o,
                "| {} | | не замерено: {} |",
                s.family,
                s.reason.as_deref().unwrap_or("—")
            );
        }
        let _ = writeln!(o);
    }

    fn net_table(&self, o: &mut String) {
        if let Some((net, plain)) = self.net_vs_plain() {
            let _ = writeln!(o, "## Влияние сети (Wi-Fi + MQTT раз в 100 мс)\n");
            let _ = writeln!(o, "| Лист | Средний | Максимум | Пауза (макс.) | Разброс |");
            let _ = writeln!(o, "|---|---:|---:|---:|---:|");
            for (name, s) in [("без сети", plain), ("с сетью", net)] {
                let _ = writeln!(
                    o,
                    "| {name} | {} | {} | {} | {} |",
                    f1(s.mean_us),
                    int(s.max_us),
                    int(s.gap_us),
                    int(s.spread_us)
                );
            }
            let _ = writeln!(o);
        } else if let Some(s) = self.stands.iter().find(|s| s.family == "net") {
            let _ = writeln!(
                o,
                "## Влияние сети\n\nНе замерено: {}\n",
                s.reason.as_deref().unwrap_or("—")
            );
        }
    }

    fn modbus_table(&self, o: &mut String) {
        let Some(stand) = self.stands.iter().find(|s| s.family == "modbus_tcp") else {
            return;
        };
        let title = "## Modbus TCP (плата — слейв по Wi-Fi, ПК читает 10 регистров)";
        let (true, Some(m)) = (stand.is_ok(), &self.modbus) else {
            let _ = writeln!(
                o,
                "{title}\n\nНе замерено: {}\n",
                stand.reason.as_deref().unwrap_or("—")
            );
            return;
        };
        let _ = writeln!(o, "{title}\n");
        let _ = writeln!(
            o,
            "Запросов: {}, ошибок: {}. Ответ, мс: среднее {}, 95 % {}, максимум {}. \
             Запросов в секунду: {}.{}\n",
            m.requests,
            m.errors,
            f2(m.mean_ms),
            f2(m.p95_ms),
            f2(m.max_ms),
            f1(m.req_per_s),
            m.note
                .as_deref()
                .map(|n| format!(" {n}"))
                .unwrap_or_default()
        );
        let _ = writeln!(
            o,
            "| Шаг листа, мкс | Средний | Максимум | Пауза (макс.) | Разброс |"
        );
        let _ = writeln!(o, "|---|---:|---:|---:|---:|");
        let _ = writeln!(
            o,
            "| без опроса | {} | {} | {} | {} |",
            f1(stand.mean_us),
            int(stand.max_us),
            int(stand.gap_us),
            int(stand.spread_us)
        );
        let p = m.polled.clone().unwrap_or_default();
        let _ = writeln!(
            o,
            "| во время опроса | {} | {} | {} | {} |",
            f1(p.mean_us),
            int(p.max_us),
            int(p.gap_us),
            int(p.spread_us)
        );
        let _ = writeln!(o);
    }

    fn memory_table(&self, o: &mut String) {
        let rows: Vec<&StandResult> = self
            .stands
            .iter()
            .filter(|s| s.is_ok() && s.free.is_some())
            .collect();
        if rows.is_empty() && self.sheets.is_none() {
            return;
        }
        let _ = writeln!(o, "## Память, байт\n");
        if !rows.is_empty() {
            let _ = writeln!(
                o,
                "| Стенд | Свободно | Наибольший блок | Минимум с включения | Модуль (куча) | из них код |"
            );
            let _ = writeln!(o, "|---|---:|---:|---:|---:|---:|");
            for s in rows {
                let _ = writeln!(
                    o,
                    "| {} | {} | {} | {} | {} | {} |",
                    s.id,
                    int(s.free),
                    int(s.largest),
                    int(s.min_free),
                    int(s.module_mem),
                    int(s.module_code)
                );
            }
            let _ = writeln!(o);
        }
        if let Some(f) = &self.sheets {
            let _ = writeln!(
                o,
                "Листов по 10 блоков поместилось: **{}** (пробовали до {}){}.\n",
                f.fit,
                f.tried,
                f.reason
                    .as_deref()
                    .map(|r| format!(", дальше: {r}"))
                    .unwrap_or_default()
            );
            if !f.free_after.is_empty() {
                let free: Vec<String> = f.free_after.iter().map(u32::to_string).collect();
                let _ = writeln!(
                    o,
                    "Свободная куча после каждого листа: {}.\n",
                    free.join(", ")
                );
            }
        }
    }

    fn times_table(&self, o: &mut String) {
        let uploads: Vec<&StandResult> = self
            .stands
            .iter()
            .filter(|s| s.is_ok() && s.upload_ms.is_some())
            .collect();
        if uploads.is_empty() && self.restart.is_none() {
            return;
        }
        let _ = writeln!(o, "## Время операций\n");
        if !uploads.is_empty() {
            let _ = writeln!(o, "| Стенд | WASM, байт | Образ, байт | Заливка, мс |");
            let _ = writeln!(o, "|---|---:|---:|---:|");
            for s in uploads {
                let _ = writeln!(
                    o,
                    "| {} | {} | {} | {} |",
                    s.id,
                    int(s.wasm_bytes),
                    int(s.image_bytes),
                    int(s.upload_ms)
                );
            }
            let _ = writeln!(o);
        }
        if let Some(r) = &self.restart {
            let _ = writeln!(
                o,
                "Перезапуск модуля до первого шага: {} мс. Сброс платы до первого шага: {} мс.{}\n",
                int(r.module_ms),
                int(r.boot_ms),
                r.note
                    .as_deref()
                    .map(|n| format!(" {n}"))
                    .unwrap_or_default()
            );
        }
    }
}

impl StandResult {
    fn family_is_call(&self) -> bool {
        self.calls > 0
            || matches!(
                self.family.as_str(),
                "gpio_in" | "gpio_out" | "adc" | "i2c" | "retain" | "uart"
            )
    }
}

fn f1(v: Option<f64>) -> String {
    v.map_or_else(|| "—".into(), |v| format!("{v:.1}"))
}

fn f2(v: Option<f64>) -> String {
    v.map_or_else(|| "—".into(), |v| format!("{v:.2}"))
}

fn int(v: Option<u32>) -> String {
    v.map_or_else(|| "—".into(), |v| v.to_string())
}

/// One figure of a stand in both reports.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    pub stand: String,
    pub metric: &'static str,
    pub before: f64,
    pub after: f64,
    pub percent: f64,
    /// More than [`MARK_PERCENT`] either way.
    pub marked: bool,
    /// The change is for the worse (slower, less memory).
    pub worse: bool,
}

/// Compares the figures of the stands both reports have measured.
pub fn compare(a: &BenchReport, b: &BenchReport) -> Vec<Diff> {
    type Pick = fn(&StandResult) -> Option<f64>;
    // (name, figure, bigger is better)
    let metrics: [(&'static str, Pick, bool); 6] = [
        ("средний шаг, мкс", |s| s.mean_us, false),
        ("максимум шага, мкс", |s| s.max_us.map(f64::from), false),
        (
            "пауза между шагами, мкс",
            |s| s.gap_us.map(f64::from),
            false,
        ),
        ("заливка, мс", |s| s.upload_ms.map(f64::from), false),
        ("свободная куча, байт", |s| s.free.map(f64::from), true),
        ("минимум кучи, байт", |s| s.min_free.map(f64::from), true),
    ];
    let mut diffs = Vec::new();
    for sa in a.stands.iter().filter(|s| s.is_ok()) {
        let Some(sb) = b.stands.iter().find(|s| s.id == sa.id && s.is_ok()) else {
            continue;
        };
        for (metric, pick, bigger_is_better) in metrics {
            let (Some(before), Some(after)) = (pick(sa), pick(sb)) else {
                continue;
            };
            diffs.extend(make_diff(&sa.id, metric, before, after, bigger_is_better));
        }
    }
    // Modbus polling: answer times, request rate and the step time under polling.
    if let (Some(ma), Some(mb)) = (measured_modbus(a), measured_modbus(b)) {
        type MPick = fn(&ModbusResult) -> Option<f64>;
        let picks: [(&'static str, MPick, bool); 5] = [
            ("ответ Modbus, среднее, мс", |m| m.mean_ms, false),
            ("ответ Modbus, 95 %, мс", |m| m.p95_ms, false),
            ("запросов в секунду", |m| m.req_per_s, true),
            (
                "шаг при опросе, средний, мкс",
                |m| m.polled.as_ref()?.mean_us,
                false,
            ),
            (
                "шаг при опросе, максимум, мкс",
                |m| m.polled.as_ref()?.max_us.map(f64::from),
                false,
            ),
        ];
        for (metric, pick, bigger_is_better) in picks {
            if let (Some(before), Some(after)) = (pick(ma), pick(mb)) {
                diffs.extend(make_diff(
                    "modbus_tcp",
                    metric,
                    before,
                    after,
                    bigger_is_better,
                ));
            }
        }
    }
    diffs
}

/// The Modbus figures of a report whose stand ran and got answers.
fn measured_modbus(r: &BenchReport) -> Option<&ModbusResult> {
    let ran = r
        .stands
        .iter()
        .any(|s| s.family == "modbus_tcp" && s.is_ok());
    r.modbus.as_ref().filter(|m| ran && m.mean_ms.is_some())
}

fn make_diff(
    stand: &str,
    metric: &'static str,
    before: f64,
    after: f64,
    bigger_is_better: bool,
) -> Option<Diff> {
    if before == 0.0 && after == 0.0 {
        return None;
    }
    let percent = if before == 0.0 {
        100.0
    } else {
        (after - before) / before * 100.0
    };
    Some(Diff {
        stand: stand.to_owned(),
        metric,
        before,
        after,
        percent,
        marked: percent.abs() > MARK_PERCENT,
        worse: if bigger_is_better {
            percent < 0.0
        } else {
            percent > 0.0
        },
    })
}

pub fn render_compare(a: &BenchReport, b: &BenchReport) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "Сравнение: {} {} ({}) -> {} {} ({})",
        a.meta.chip, a.meta.master, a.meta.date, b.meta.chip, b.meta.master, b.meta.date
    );
    if a.meta.chip != b.meta.chip {
        let _ = writeln!(o, "Внимание: отчёты с разных плат, сравнение условно.");
    }
    let diffs = compare(a, b);
    if diffs.is_empty() {
        let _ = writeln!(o, "Общих замеров нет.");
        return o;
    }
    let mut marked = 0;
    for d in &diffs {
        if !d.marked {
            continue;
        }
        marked += 1;
        let verdict = if d.worse { "ХУЖЕ" } else { "лучше" };
        let _ = writeln!(
            o,
            "{verdict:<6} {:<22} {:<26} {:>12.1} -> {:>12.1}  ({:+.1} %)",
            d.stand, d.metric, d.before, d.after, d.percent
        );
    }
    let _ = writeln!(
        o,
        "Сравнено цифр: {}, изменилось больше чем на {MARK_PERCENT:.0} %: {marked}.",
        diffs.len()
    );
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stand(id: &str, mean: f64, max: u32, free: u32) -> StandResult {
        StandResult {
            id: id.into(),
            family: "logic".into(),
            n: 100,
            blocks: 102,
            mode: "aot".into(),
            status: "ok".into(),
            mean_us: Some(mean),
            max_us: Some(max),
            free: Some(free),
            ..StandResult::default()
        }
    }

    fn report(mean: f64, max: u32, free: u32) -> BenchReport {
        BenchReport {
            meta: Meta {
                chip: "esp32".into(),
                master: "0.4.1".into(),
                date: "2026-11-12 14:30".into(),
                ..Meta::default()
            },
            stands: vec![stand("logic_100_aot", mean, max, free)],
            ..BenchReport::default()
        }
    }

    #[test]
    fn compare_marks_changes_over_ten_percent() {
        let a = report(100.0, 200, 50_000);
        // mean +5 % (not marked), max +50 % (worse), free -20 % (worse).
        let b = report(105.0, 300, 40_000);
        let diffs = compare(&a, &b);
        let get = |m: &str| diffs.iter().find(|d| d.metric.starts_with(m)).unwrap();
        assert!(!get("средний").marked);
        assert!(get("максимум").marked && get("максимум").worse);
        assert!(get("свободная").marked && get("свободная").worse);
        // Faster is marked too, and not "worse".
        let better = compare(&b, &a);
        let max = better
            .iter()
            .find(|d| d.metric.starts_with("максимум"))
            .unwrap();
        assert!(max.marked && !max.worse);
        let text = render_compare(&a, &b);
        assert!(text.contains("ХУЖЕ"));
        assert!(!text.contains("средний шаг"));
    }

    #[test]
    fn toml_round_trip_keeps_the_report() {
        let mut r = report(100.0, 200, 50_000);
        r.sheets = Some(SheetsFit {
            fit: 5,
            tried: 12,
            reason: Some("нет памяти".into()),
            free_after: vec![90_000, 80_000],
        });
        r.restart = Some(RestartTimes {
            module_ms: Some(120),
            boot_ms: None,
            note: None,
        });
        r.stands.push(StandResult::failed(
            "logic_500_interp",
            "logic",
            500,
            "interp",
            "не влезло".into(),
        ));
        let text = r.to_toml().unwrap();
        assert_eq!(BenchReport::from_toml(&text).unwrap(), r);
        assert_eq!(r.file_stem(), "esp32-0.4.1-2026-11-12-1430");
    }

    fn modbus_report(mean: f64, p95: f64, rps: f64) -> BenchReport {
        let mut r = report(100.0, 200, 50_000);
        let mut s = stand("modbus_tcp_100_aot", 100.0, 200, 50_000);
        s.family = "modbus_tcp".into();
        r.stands = vec![s];
        r.modbus = Some(ModbusResult {
            requests: 500,
            mean_ms: Some(mean),
            p95_ms: Some(p95),
            max_ms: Some(p95 * 2.0),
            req_per_s: Some(rps),
            polled: Some(StepFigures {
                mean_us: Some(120.0),
                max_us: Some(900),
                ..StepFigures::default()
            }),
            ..ModbusResult::default()
        });
        r
    }

    #[test]
    fn compare_covers_modbus_latency_and_rate() {
        let a = modbus_report(20.0, 40.0, 50.0);
        // mean +5 % (not marked), p95 +50 % (worse), rate -30 % (worse).
        let b = modbus_report(21.0, 60.0, 35.0);
        let diffs = compare(&a, &b);
        let get = |m: &str| diffs.iter().find(|d| d.metric.starts_with(m)).unwrap();
        assert!(!get("ответ Modbus, среднее").marked);
        assert!(get("ответ Modbus, 95").marked && get("ответ Modbus, 95").worse);
        assert!(get("запросов").marked && get("запросов").worse);
        // The same polled step is not a change.
        assert!(!get("шаг при опросе, средний").marked);
        assert!(render_compare(&a, &b).contains("запросов в секунду"));
    }

    #[test]
    fn modbus_table_is_in_the_markdown_and_the_toml_round_trips() {
        let r = modbus_report(20.0, 40.0, 50.0);
        let md = r.to_markdown();
        assert!(md.contains("## Modbus TCP"));
        assert!(md.contains("без опроса"));
        assert!(md.contains("во время опроса"));
        assert_eq!(BenchReport::from_toml(&r.to_toml().unwrap()).unwrap(), r);
        // A skipped stand says why instead.
        let mut skipped = BenchReport::default();
        skipped.stands.push(StandResult::skipped(
            "modbus_tcp_100_aot",
            "modbus_tcp",
            100,
            "aot",
            "нет Wi-Fi".into(),
        ));
        assert!(skipped.to_markdown().contains("Не замерено: нет Wi-Fi"));
    }

    #[test]
    fn call_cost_is_the_difference_to_the_empty_stand_per_call() {
        let mut r = report(100.0, 200, 50_000);
        r.stands.push(stand("empty_aot", 10.0, 20, 60_000));
        let mut gpio = stand("gpio_in_aot", 90.0, 120, 60_000);
        gpio.family = "gpio_in".into();
        gpio.calls = 8;
        r.stands.push(gpio);
        let costs = r.call_costs();
        assert_eq!(costs.len(), 1);
        assert!((costs[0].us_per_call - 10.0).abs() < 1e-9);
        assert!(r.to_markdown().contains("| gpio_in | 8 | 10.0 |"));
    }
}
