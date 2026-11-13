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
            let expected = if stand.family == Family::Sheets {
                stands::MAX_SHEETS
            } else {
                1
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
    assert_eq!(slave.params["holding"].to_string(), "20");
    // Ten floats take twenty registers, each fed by a tap of the 100-block chain.
    let writers = blocks.iter().filter(|b| b.kind == "modbus_write").count();
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
