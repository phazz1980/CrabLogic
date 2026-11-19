use std::collections::BTreeMap;

use super::listing::parse_list;
use super::report::{BenchReport, Meta, StandResult};
use super::secrets::Secrets;
use super::stands::{build_project, sheets_project, standard_stands, Chip, Family, Stand};
use super::*;

const CHIPS: [Chip; 2] = [Chip::Esp32, Chip::Esp32c3];

const FAKE: &str = r#"
[wifi]
ssid = "FakeSsidXYZ"
password = "FakeWifiPassXYZ"
[mqtt]
host = "fakebroker.example"
port = 1883
user = "fakeuser"
password = "FakeMqttPassXYZ"
"#;

fn library() -> Library {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    Library::load(dir).expect("library loads").0
}

#[test]
fn memory_stand_sheet_is_a_50_30_20_mix() {
    for chip in CHIPS {
        let project = sheets_project(chip, 1);
        let count = |kinds: &[&str]| {
            project
                .logic
                .blocks
                .iter()
                .filter(|b| kinds.contains(&b.kind.as_str()))
                .count()
        };
        assert_eq!(count(&["and", "or", "xor", "not"]), 5);
        assert_eq!(count(&["add", "mul", "sub", "div"]), 3);
        assert_eq!(count(&["timer_on", "timer_off"]), 2);
    }
}

#[test]
fn mix_counts_keep_50_30_20() {
    assert_eq!(stands::mix_counts(10), (5, 3, 2));
    assert_eq!(stands::mix_counts(20), (10, 6, 4));
    assert_eq!(stands::mix_counts(15), (7, 4, 4));
    for n in [10, 15, 20, 50] {
        let (a, b, c) = stands::mix_counts(n);
        assert_eq!(a + b + c, n);
    }
}

#[test]
fn project_stands_hold_the_target_blocks() {
    let lib = library();
    let secrets = Secrets::parse(FAKE).unwrap();
    for chip in CHIPS {
        for stand in standard_stands()
            .into_iter()
            .filter(|s| matches!(s.family, Family::Project | Family::ProjectNet))
        {
            let (sheets, per_sheet) = stands::project_layout(chip, stand.n);
            assert_eq!(sheets * per_sheet, chip.project_blocks(), "{}", stand.id);
            let built = build_project(&stand, chip, Some(&secrets)).unwrap();
            let modules = generate_modules(&built.project, &lib).unwrap();
            assert_eq!(modules.len(), sheets, "{} on {}", stand.id, chip.name());
            // Without the network stand there are no Wi-Fi blocks, with it exactly those three.
            let net = built
                .project
                .logic
                .blocks
                .iter()
                .filter(|b| ["wifi", "mqtt", "mqtt_publish"].contains(&b.kind.as_str()))
                .count();
            assert_eq!(
                net,
                if stand.family == Family::ProjectNet {
                    3
                } else {
                    0
                }
            );
        }
    }
}

#[test]
fn old_logic_stand_is_kept_and_builds() {
    let lib = library();
    let project = stands::sheets_project_logic(Chip::Esp32, stands::MAX_SHEETS);
    let modules = generate_modules(&project, &lib).unwrap();
    assert_eq!(modules.len(), stands::MAX_SHEETS);
}

#[test]
fn every_stand_passes_check_and_wasm_generation() {
    let lib = library();
    let secrets = Secrets::parse(FAKE).unwrap();
    for chip in CHIPS {
        for stand in standard_stands() {
            let project = match stand.family {
                Family::Sheets => sheets_project(chip, stands::MAX_SHEETS),
                _ => {
                    build_project(&stand, chip, Some(&secrets))
                        .unwrap_or_else(|e| panic!("{} on {}: {e}", stand.id, chip.name()))
                        .project
                }
            };
            let modules = generate_modules(&project, &lib)
                .unwrap_or_else(|e| panic!("{} on {}: {e}", stand.id, chip.name()));
            let expected = match stand.family {
                Family::Sheets => stands::MAX_SHEETS,
                Family::Project | Family::ProjectNet => stands::project_layout(chip, stand.n).0,
                _ => 1,
            };
            assert_eq!(modules.len(), expected, "{} on {}", stand.id, chip.name());
        }
    }
}

#[test]
fn bigger_stands_make_bigger_modules() {
    let lib = library();
    let size = |id: &str| {
        let stand = standard_stands().into_iter().find(|s| s.id == id).unwrap();
        let built = build_project(&stand, Chip::Esp32, None).unwrap();
        generate_modules(&built.project, &lib).unwrap()[0]
            .wasm
            .len()
    };
    for family in ["logic", "float", "timers", "text"] {
        let (a, b, c) = (
            size(&format!("{family}_10_aot")),
            size(&format!("{family}_100_aot")),
            size(&format!("{family}_500_aot")),
        );
        assert!(a < b && b < c, "{family}: {a} {b} {c}");
    }
}

#[test]
fn call_stands_really_hold_their_calls() {
    let lib = library();
    let size = |family: Family, chip: Chip| {
        let stand = standard_stands()
            .into_iter()
            .find(|s| s.family == family)
            .unwrap();
        let built = build_project(&stand, chip, None).unwrap();
        assert!(
            built.calls >= 4,
            "{family:?} on {}: {}",
            chip.name(),
            built.calls
        );
        generate_modules(&built.project, &lib).unwrap()[0]
            .wasm
            .len()
    };
    let empty = standard_stands()
        .into_iter()
        .find(|s| s.family == Family::Empty)
        .unwrap();
    for chip in CHIPS {
        let base = generate_modules(&build_project(&empty, chip, None).unwrap().project, &lib)
            .unwrap()[0]
            .wasm
            .len();
        for family in [
            Family::GpioIn,
            Family::GpioOut,
            Family::Adc,
            Family::I2c,
            Family::Retain,
            Family::Uart,
        ] {
            assert!(size(family, chip) > base, "{family:?} on {}", chip.name());
        }
    }
}

#[test]
fn net_stand_without_secrets_is_skipped_with_a_reason() {
    let stand = standard_stands()
        .into_iter()
        .find(|s| s.family == Family::Net)
        .unwrap();
    let error = build_project(&stand, Chip::Esp32, None).err().unwrap();
    assert!(error.contains("secrets.toml"));
    // A file naming no network is the same as no file.
    let empty = Secrets::parse("[wifi]\nssid = \"\"\n").unwrap();
    assert!(build_project(&stand, Chip::Esp32, Some(&empty)).is_err());
}

#[test]
fn report_does_not_contain_the_secrets() {
    let lib = library();
    let secrets = Secrets::parse(FAKE).unwrap();
    let stand = standard_stands()
        .into_iter()
        .find(|s| s.family == Family::Net)
        .unwrap();
    let built = build_project(&stand, Chip::Esp32, Some(&secrets)).unwrap();
    // The stand really carries the settings...
    let ssid_in_project = built
        .project
        .logic
        .blocks
        .iter()
        .any(|b| b.params.values().any(|v| v.to_string() == "FakeSsidXYZ"));
    assert!(ssid_in_project);
    let modules = generate_modules(&built.project, &lib).unwrap();
    assert_eq!(modules.len(), 1);
    // ...while the report of its run, and the text that fails say nothing of them.
    let report = BenchReport {
        meta: Meta {
            chip: "esp32".into(),
            master: "0.4.1".into(),
            date: "2026-11-12 14:30".into(),
            ..Meta::default()
        },
        stands: vec![
            StandResult {
                id: stand.id.clone(),
                family: "net".into(),
                n: stand.n,
                blocks: built.blocks,
                mode: "aot".into(),
                status: "ok".into(),
                mean_us: Some(1.0),
                ..StandResult::default()
            },
            StandResult::failed("x", "logic", 1, "aot", "плата отклонила модуль".into()),
        ],
        ..BenchReport::default()
    };
    let everything = format!(
        "{}\n{}\n{:?}\n{}",
        report.to_toml().unwrap(),
        report.to_markdown(),
        secrets,
        report::render_compare(&report, &report)
    );
    for secret in [
        "FakeSsidXYZ",
        "FakeWifiPassXYZ",
        "fakebroker",
        "fakeuser",
        "FakeMqttPassXYZ",
    ] {
        assert!(!everything.contains(secret), "{secret} leaked");
    }
    // A broken secrets file does not echo its text either.
    let error = Secrets::parse("[wifi]\npassword = FakeWifiPassXYZ\n")
        .err()
        .unwrap();
    assert!(!error.contains("FakeWifiPassXYZ"));
}

fn at(base: Instant, secs: u64) -> Instant {
    base + Duration::from_secs(secs)
}

#[test]
fn figures_are_taken_from_the_growth_over_the_window() {
    let line = |steps: u32, last: u32, max: u32, sum: u32| {
        parse_list([
            format!(
                "sheet1 run steps={steps} restarts=0 out0=0 us={last}/{max}/5000/{sum} qlost=0 mem=1000 code=400 wasm=300 mode=aot crc=00000001"
            )
            .as_str(),
            "free=80000 largest=60000 min=50000",
        ])
    };
    let t = Instant::now();
    let readings = vec![
        (t, line(1000, 100, 150, 100_000)),
        (at(t, 2), line(1100, 120, 180, 112_000)),
        (at(t, 4), line(1200, 110, 400, 123_000)),
    ];
    let f = figures_of(&readings, "sheet1").unwrap();
    // 23000 us over 200 steps.
    assert!((f.mean_us.unwrap() - 115.0).abs() < 1e-9);
    assert!((f.last_us.unwrap() - 110.0).abs() < 1e-9);
    assert_eq!(f.max_us, Some(400));
    assert_eq!(f.gap_us, Some(5000));
    assert_eq!(f.spread_us, Some(20));
    assert!((f.steps_per_s - 50.0).abs() < 1e-6);
    assert_eq!(f.mode.as_deref(), Some("aot"));
    assert_eq!(f.free, Some(80_000));
    assert_eq!(f.module_mem, Some(1000));
}

#[test]
fn the_sum_of_step_times_may_wrap() {
    let line = |steps: u32, sum: u32| {
        parse_list([
            format!("m run steps={steps} restarts=0 out0=0 us=10/20/30/{sum} crc=1").as_str(),
        ])
    };
    let t = Instant::now();
    let readings = vec![(t, line(10, u32::MAX - 99)), (at(t, 2), line(20, 100))];
    // 200 us over 10 steps.
    assert!((figures_of(&readings, "m").unwrap().mean_us.unwrap() - 20.0).abs() < 1e-9);
}

#[test]
fn trouble_during_the_window_is_a_failure_not_a_figure() {
    let t = Instant::now();
    let make = |state: &str, steps: u32, restarts: u32| {
        parse_list([
            format!("m {state} steps={steps} restarts={restarts} out0=0 us=1/2/3/4 crc=1").as_str(),
        ])
    };
    let run = |a, b| figures_of(&[(t, a), (at(t, 2), b)], "m");
    assert!(run(make("run", 5, 0), make("stop", 9, 0))
        .unwrap_err()
        .contains("остановился"));
    assert!(run(make("run", 5, 0), make("run", 9, 1))
        .unwrap_err()
        .contains("перезапускался"));
    assert!(run(make("run", 5, 0), make("run", 5, 0))
        .unwrap_err()
        .contains("не делает шагов"));
    assert!(run(make("run", 5, 0), ListAnswer::default())
        .unwrap_err()
        .contains("пропал"));
    // An older master without `us=`: steps only.
    let old = |steps| parse_list([format!("m run steps={steps} restarts=0 out0=0 crc=1").as_str()]);
    let f = run(old(5), old(9)).unwrap();
    assert_eq!(f.mean_us, None);
    assert!(f.steps_per_s > 0.0);
}

#[test]
fn only_picks_stands_and_calls_bring_the_empty_one() {
    let ids = |only: &[&str]| -> Vec<String> {
        let only: Vec<String> = only.iter().map(|s| (*s).to_owned()).collect();
        select_stands(&only).into_iter().map(|s| s.id).collect()
    };
    assert_eq!(ids(&["logic_10"]), ["logic_10_aot", "logic_10_interp"]);
    assert_eq!(ids(&["float_100_aot"]), ["float_100_aot"]);
    assert_eq!(ids(&["adc"]), ["empty_aot", "adc_aot"]);
    assert!(ids(&["nothing"]).is_empty());
    assert_eq!(select_stands(&[]).len(), standard_stands().len());
}

#[test]
fn stand_ids_are_unique() {
    let stands: Vec<Stand> = standard_stands();
    let mut ids: Vec<&str> = stands.iter().map(|s| s.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len());
}

#[test]
fn utc_date_is_formatted() {
    assert_eq!(format_utc(0), "1970-01-01 00:00");
    assert_eq!(format_utc(1_794_493_800), "2026-11-12 14:30");
}

fn modbus_stand() -> Stand {
    standard_stands()
        .into_iter()
        .find(|s| s.family == Family::ModbusTcp)
        .unwrap()
}

#[test]
fn modbus_stand_needs_wifi_only_and_feeds_twenty_registers() {
    let stand = modbus_stand();
    assert_eq!(stand.id, "modbus_tcp_100_aot");
    let error = build_project(&stand, Chip::Esp32, None).err().unwrap();
    assert!(error.contains("secrets.toml"));
    // No broker is needed, a network is.
    let wifi_only = Secrets::parse("[wifi]\nssid = \"FakeSsidXYZ\"\n").unwrap();
    let built = build_project(&stand, Chip::Esp32, Some(&wifi_only)).unwrap();
    let blocks = &built.project.logic.blocks;
    let slave = blocks
        .iter()
        .find(|b| b.kind == "modbus_tcp_slave")
        .unwrap();
    assert!(
        !slave.params.contains_key("holding"),
        "sizes are not parameters"
    );
    // Ten floats at holding registers 0, 2, ..., 18 take twenty registers.
    assert_eq!(
        built.project.logic.modbus_table_sizes("slave"),
        [20, 0, 0, 0]
    );
    // Ten floats take twenty registers, each fed by a tap of the 100-block chain.
    let writers = blocks.iter().filter(|b| b.kind == "modbus_tag").count();
    assert_eq!(writers, 10);
    let chain = blocks.iter().filter(|b| b.id.starts_with('f')).count();
    assert_eq!(chain, 100);
    let tapped: std::collections::BTreeSet<String> = built
        .project
        .logic
        .links
        .iter()
        .filter(|l| l.to.to_string().ends_with(".in_f"))
        .map(|l| l.from.to_string())
        .collect();
    assert_eq!(tapped.len(), 10);
    assert!(tapped.contains("f99.out") && tapped.contains("f9.out"));
    let none = Secrets::parse("[mqtt]\nhost = \"x\"\n").unwrap();
    assert!(build_project(&stand, Chip::Esp32, Some(&none)).is_err());
}

#[test]
fn modbus_report_does_not_contain_the_secrets() {
    let lib = library();
    let secrets = Secrets::parse(FAKE).unwrap();
    let stand = modbus_stand();
    let built = build_project(&stand, Chip::Esp32c3, Some(&secrets)).unwrap();
    assert_eq!(generate_modules(&built.project, &lib).unwrap().len(), 1);
    let report = BenchReport {
        stands: vec![StandResult {
            id: stand.id.clone(),
            family: "modbus_tcp".into(),
            n: stand.n,
            blocks: built.blocks,
            mode: "aot".into(),
            status: "ok".into(),
            mean_us: Some(1.0),
            ..StandResult::default()
        }],
        modbus: Some(report::ModbusResult {
            requests: 500,
            mean_ms: Some(12.5),
            p95_ms: Some(20.0),
            max_ms: Some(31.0),
            req_per_s: Some(80.0),
            ..report::ModbusResult::default()
        }),
        ..BenchReport::default()
    };
    let everything = format!(
        "{}\n{}\n{}",
        report.to_toml().unwrap(),
        report.to_markdown(),
        report::render_compare(&report, &report)
    );
    assert!(everything.contains("Modbus TCP"));
    for secret in ["FakeSsidXYZ", "FakeWifiPassXYZ", "fakebroker"] {
        assert!(!everything.contains(secret), "{secret} leaked");
    }
}

#[test]
fn only_modbus_picks_the_modbus_stand() {
    let ids: Vec<String> = select_stands(&["modbus_tcp".to_owned()])
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, ["modbus_tcp_100_aot"]);
}

fn cycle_stand() -> Stand {
    standard_stands()
        .into_iter()
        .find(|s| s.family == Family::Cycle)
        .unwrap()
}

#[test]
fn cycle_stand_runs_at_one_ms_with_net_and_modbus() {
    let stand = cycle_stand();
    assert_eq!(stand.id, "cycle_1ms_aot");
    let error = build_project(&stand, Chip::Esp32, None).err().unwrap();
    assert!(error.contains("secrets.toml"));
    let secrets = Secrets::parse(FAKE).unwrap();
    let built = build_project(&stand, Chip::Esp32, Some(&secrets)).unwrap();
    assert_eq!(built.project.config.run.period_ms, 1);
    let blocks = &built.project.logic.blocks;
    let gates = blocks
        .iter()
        .filter(|b| matches!(b.kind.as_str(), "and" | "or" | "xor" | "not"))
        .count();
    assert_eq!(gates, 100);
    for kind in ["wifi", "mqtt", "mqtt_publish", "modbus_tcp_slave"] {
        assert_eq!(
            blocks.iter().filter(|b| b.kind == kind).count(),
            1,
            "{kind}"
        );
    }
    // One sheet only.
    assert!(built.project.layout.blocks.values().all(|p| p.sheet == 1));
    let report = crab_codegen::check_project(&built.project, &library());
    assert!(report.errors().next().is_none());
}

#[test]
fn only_cycle_picks_the_cycle_stand_by_id_and_family() {
    for token in ["cycle_1ms", "cycle"] {
        let ids: Vec<String> = select_stands(&[token.to_owned()])
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["cycle_1ms_aot"], "{token}");
    }
}

#[test]
fn cycle_line_is_parsed_and_judged_by_criterion_two() {
    let answer = parse_list([
        "free=100000 largest=60000 min=90000",
        "cycle period=1000 steps=10000 gap=1090 over=0 late50=50 late100=5 later=1",
    ]);
    let c = answer.cycle.unwrap();
    assert_eq!(
        (c.period_us, c.steps, c.gap_us, c.over, c.late50, c.late100),
        (1000, 10_000, 1090, 0, 50, 5)
    );
    assert_eq!(answer.free, Some(100_000));
    assert!(parse_list(["free=1"]).cycle.is_none());
    let mut r = report::CycleResult {
        period_us: 1000,
        steps: 10_000,
        gap_us: 1090,
        over: 0,
        late50: 50,
        late100: 5,
        between_us: 0,
    };
    assert_eq!(r.on_time_50_percent(), Some(99.5));
    assert!(r.passes());
    r.late50 = 200;
    assert!(!r.passes());
    r.late50 = 50;
    r.gap_us = 1200;
    assert!(!r.passes());
}

#[test]
fn cycle_report_has_the_table_and_round_trips() {
    let mut s = StandResult {
        id: "cycle_1ms_aot".into(),
        family: "cycle".into(),
        status: "ok".into(),
        mean_us: Some(80.0),
        max_us: Some(150),
        cycle: Some(report::CycleResult {
            period_us: 1000,
            steps: 1000,
            gap_us: 1080,
            over: 0,
            late50: 3,
            late100: 1,
            between_us: 0,
        }),
        ..StandResult::default()
    };
    let r = BenchReport {
        stands: vec![s.clone()],
        ..BenchReport::default()
    };
    let md = r.to_markdown();
    assert!(md.contains("Быстрый цикл") && md.contains("99.70 %") && md.contains("выполнен"));
    let back = BenchReport::from_toml(&r.to_toml().unwrap()).unwrap();
    assert_eq!(back, r);
    s.cycle.as_mut().unwrap().late50 = 100;
    let worse = BenchReport {
        stands: vec![s],
        ..BenchReport::default()
    };
    let diffs = report::compare(&r, &worse);
    assert!(diffs
        .iter()
        .any(|d| d.metric.starts_with("цикл: опоздавших") && d.worse));
}

/// Module heap (`из них код`) of the AOT stands, bytes, from the tracked reports
/// `bench/esp32-0.1.0-2026-11-14-1746.md` and `bench/esp32c3-0.1.0-2026-11-14-1757.md`.
const ESP32_HEAP_REF: [(&str, u32); 21] = [
    ("empty_aot", 11540),
    ("logic_10_aot", 11964),
    ("logic_100_aot", 16132),
    ("logic_500_aot", 34576),
    ("float_10_aot", 12024),
    ("float_100_aot", 16172),
    ("float_500_aot", 34636),
    ("timers_10_aot", 12880),
    ("timers_100_aot", 24952),
    ("timers_500_aot", 78616),
    ("text_10_aot", 12096),
    ("text_100_aot", 16244),
    ("text_500_aot", 34648),
    ("gpio_in_aot", 11852),
    ("gpio_out_aot", 11852),
    ("adc_aot", 11760),
    ("i2c_aot", 12260),
    ("retain_aot", 12568),
    ("uart_aot", 13112),
    ("net_100_aot", 17304),
    ("modbus_tcp_100_aot", 16812),
];

const C3_HEAP_REF: [(&str, u32); 21] = [
    ("empty_aot", 11464),
    ("logic_10_aot", 11912),
    ("logic_100_aot", 16052),
    ("logic_500_aot", 34504),
    ("float_10_aot", 11932),
    ("float_100_aot", 16080),
    ("float_500_aot", 34536),
    ("timers_10_aot", 12812),
    ("timers_100_aot", 24848),
    ("timers_500_aot", 78524),
    ("text_10_aot", 12072),
    ("text_100_aot", 16328),
    ("text_500_aot", 35276),
    ("gpio_in_aot", 11780),
    ("gpio_out_aot", 11780),
    ("adc_aot", 11656),
    ("i2c_aot", 12164),
    ("retain_aot", 12512),
    ("uart_aot", 13112),
    ("net_100_aot", 17308),
    ("modbus_tcp_100_aot", 20336),
];

fn core_chip(chip: Chip) -> Option<crab_core::pins::Chip> {
    crab_core::pins::Chip::from_fqbn(&chip.fqbn())
}

fn within(estimate: u32, measured: u32, percent: u32) -> bool {
    estimate.abs_diff(measured) * 100 <= measured * percent
}

#[test]
fn heap_estimate_matches_the_stand_reports_within_10_percent() {
    let secrets = Secrets::parse(FAKE).unwrap();
    for (chip, reference) in [
        (Chip::Esp32, &ESP32_HEAP_REF[..]),
        (Chip::Esp32c3, &C3_HEAP_REF[..]),
    ] {
        let heap = crab_core::cost::heap_for(core_chip(chip));
        for (id, measured) in reference {
            let stand = standard_stands()
                .into_iter()
                .find(|s| s.id == *id)
                .unwrap_or_else(|| panic!("no stand {id}"));
            let built = build_project(&stand, chip, Some(&secrets)).unwrap();
            let kinds = built.project.logic.blocks.iter().map(|b| b.kind.as_str());
            // The `code` of the report is the load and the blocks, without the master's part of a
            // module and without the linear memory.
            let estimate = heap.module_bytes(kinds, 0) - heap.slot;
            assert!(
                within(estimate, *measured, 10),
                "{id} on {}: estimate {estimate}, measured {measured}",
                chip.name()
            );
        }
    }
}

/// Whole-project stands without the network, from `bench/esp32-2.0.0-2026-11-15-0929.md` and
/// `bench/esp32c3-2.0.0-2026-11-15-0932.md`: stand, sheets that ran, free heap before and after.
const PROJECT_REF: [(Chip, &str, usize, u32, u32); 4] = [
    (Chip::Esp32, "project_10_aot", 13, 209_612, 18_620),
    (Chip::Esp32, "project_20_aot", 10, 209_612, 54_272),
    (Chip::Esp32c3, "project_10_aot", 5, 89_436, 22_760),
    (Chip::Esp32c3, "project_20_aot", 5, 89_276, 20_540),
];

#[test]
fn heap_estimate_matches_the_project_stands_within_10_percent() {
    let lib = library();
    for (chip, id, running, before, after) in PROJECT_REF {
        let heap = crab_core::cost::heap_for(core_chip(chip));
        let stand = standard_stands().into_iter().find(|s| s.id == id).unwrap();
        let built = build_project(&stand, chip, None).unwrap();
        let modules = generate_modules(&built.project, &lib).unwrap();
        // The blocks of every sheet, in the order of the modules.
        let mut sheets: BTreeMap<u32, Vec<&str>> = BTreeMap::new();
        for block in &built.project.logic.blocks {
            let sheet = built.project.layout.blocks[&block.id].sheet;
            sheets.entry(sheet).or_default().push(block.kind.as_str());
        }
        let bytes: Vec<u32> = sheets
            .values()
            .zip(&modules)
            .take(running)
            .map(|(kinds, module)| {
                let memory = mm::module_memory(&module.wasm).unwrap_or(0);
                heap.module_bytes(kinds.iter().copied(), memory)
            })
            .collect();
        assert_eq!(bytes.len(), running);
        let estimate = heap.project_bytes(&bytes);
        let measured = before - after;
        assert!(
            within(estimate, measured, 10),
            "{id} on {}: estimate {estimate}, measured {measured}",
            chip.name()
        );
    }
}

/// The whole-project stands, which did not fit as 10 or 20 sheets (`plan.md` of module-ram), are
/// one module for the plan: every sheet joins, and the module is valid WASM that, by the heap
/// table, fits a fresh board.
#[test]
fn the_plan_joins_the_project_stands_into_one_module() {
    let lib = library();
    let secrets = Secrets::parse(FAKE).unwrap();
    // Free heap of a fresh board without the network (`bench/esp32-2.0.0-2026-11-15-0929.md`,
    // `bench/esp32c3-2.0.0-2026-11-15-0932.md`).
    let fresh = |chip: Chip| match chip {
        Chip::Esp32 => 209_612u32,
        Chip::Esp32c3 => 89_436,
    };
    for chip in CHIPS {
        let core = core_chip(chip);
        let heap = crab_core::cost::heap_for(core);
        for stand in standard_stands()
            .into_iter()
            .filter(|s| matches!(s.family, Family::Project | Family::ProjectNet))
        {
            let built = build_project(&stand, chip, Some(&secrets)).unwrap();
            let project = crab_codegen::prepare(&built.project, &lib);
            let mut program = Program::build(&project, &lib).unwrap();
            let (sheets, _) = stands::project_layout(chip, stand.n);
            let all: Vec<u32> = (1..=sheets as u32).collect();
            assert_eq!(
                program.plan_groups(core),
                vec![all],
                "{} on {}",
                stand.id,
                chip.name()
            );
            program.auto_group(core).unwrap();
            assert_eq!(program.sheets(), vec![1]);
            let module = program.wasm_module(1).unwrap();
            assert!(module.wasm.starts_with(b"\0asm"));

            if stand.family == Family::Project {
                let sheet = program.sheet_module(1).unwrap();
                let kinds = sheet
                    .blocks
                    .iter()
                    .map(|&i| program.blocks[i].block.kind.as_str());
                let memory = mm::module_memory(&module.wasm).unwrap_or(0);
                let bytes = heap.project_bytes(&[heap.module_bytes(kinds, memory)]);
                assert!(
                    bytes + crab_core::cost::MASTER_RESERVE <= fresh(chip),
                    "{} on {}: {bytes} B do not fit {} B",
                    stand.id,
                    chip.name(),
                    fresh(chip)
                );
            }
        }
    }
}
