//! Master firmware of the WASM-on-board scheme (`specs/wasm-on-mcu`): the sketch is part
//! of the program, so the board can always be given the master that matches it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::{
    board_args, board_options, build_dir, check_status, plain_path, ArduinoCli, BuildError,
};
use crab_core::BoardConfig;

use crate::native::NativeBlock;

/// The master sketch (wasm3, LittleFS, serial protocol).
const SKETCH: &str = include_str!("../../../specs/wasm-on-mcu/proto/master/master.ino");
/// Partition table of the master: the app and the "crabmod" partition with the modules
/// (`specs/mcu-memory`). The ESP32 core takes `partitions.csv` from the sketch folder.
const PARTITIONS: &str = include_str!("../../../specs/wasm-on-mcu/proto/master/partitions.csv");
/// Hardware drivers the master carries. Each one costs flash and RAM, so the master is built
/// with only those the project's blocks need (`#define CRAB_<NAME>` in front of the sketch).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct MasterFeatures {
    /// DS18B20 and DHT22 (`sensor_read`).
    pub sensors: bool,
    /// PCF8574 expanders on I²C (`pcf_read`, `pcf_write`).
    pub pcf: bool,
    /// Modbus RTU master and slave (`modbus`); the libraries come from `libs/`.
    pub modbus: bool,
    /// UART bytes (`uart`): `uart_read`, `uart_send`, `uart_config`.
    pub uart: bool,
    /// Text operations (`text`): the `text_*` blocks, UART lines and text sending.
    pub text: bool,
    /// Network (`net`): the `wifi`, `ethernet` and `ntp` blocks; the library comes from `libs/`.
    pub net: bool,
    /// The Wi-Fi interface (`wifi`).
    pub wifi: bool,
    /// The W5500 Ethernet interface (`ethernet`).
    pub eth: bool,
    /// The MQTT client (`mqtt`).
    pub mqtt: bool,
    /// Sparkplug B on top of MQTT (`sparkplug_node`, `sparkplug_metric`): its NBIRTH needs MQTT
    /// packets of up to 768 bytes; without it the master takes 384 (`specs/mcu-memory`).
    pub sparkplug: bool,
    /// The web page (`web_ui` and widgets): the master serves the page and the module fills it.
    pub web: bool,
    /// Native parts of blocks (`native.cpp`, `specs/drivers`), sorted by block type: the position
    /// is the `kind` a module passes to `crab.native`. See [`crate::native::collect_natives`].
    pub natives: Vec<NativeBlock>,
}

impl MasterFeatures {
    /// The drivers the given block types (`ds18b20`, `pcf8574_in`...) need.
    pub fn of_blocks<'a>(kinds: impl IntoIterator<Item = &'a str>) -> Self {
        let mut f = Self::default();
        for kind in kinds {
            match kind {
                "ds18b20" | "dht22" => f.sensors = true,
                "pcf8574_in" | "pcf8574_out" => f.pcf = true,
                "modbus_rtu_master" | "modbus_rtu_slave" => f.modbus = true,
                // The TCP roles ride on the network interface the `wifi`/`ethernet` block brings.
                "modbus_tcp_master" | "modbus_tcp_slave" => {
                    f.modbus = true;
                    f.net = true;
                }
                "uart_read" | "uart_send" | "uart_config" => f.uart = true,
                // The address and the time are texts: they come through `text` too.
                "wifi" => {
                    f.net = true;
                    f.wifi = true;
                    f.text = true;
                }
                "ethernet" => {
                    f.net = true;
                    f.eth = true;
                    f.text = true;
                }
                "ntp" => {
                    f.net = true;
                    f.text = true;
                }
                "mqtt" | "mqtt_publish" | "mqtt_subscribe" => {
                    f.net = true;
                    f.mqtt = true;
                    f.text = true;
                }
                "sparkplug_node" | "sparkplug_metric" => {
                    f.net = true;
                    f.mqtt = true;
                    f.sparkplug = true;
                    f.text = true;
                }
                "web_ui" | "web_net" | "web_button" | "web_switch" | "web_label" | "web_clock"
                | "web_led" | "web_text" | "web_icon" | "web_image" | "web_shape" | "web_gauge"
                | "web_tank" | "web_value" | "web_slider" | "web_text_input" | "web_input"
                | "web_chart" => {
                    f.net = true;
                    f.web = true;
                    f.text = true;
                }
                "ip_to_text" | "text_to_ip" | "text_const" | "text_concat" | "text_length"
                | "num_to_text" | "text_compare" | "text_to_num" | "text_substring"
                | "text_find" | "text_char" | "text_add_char" => f.text = true,
                _ => {}
            }
        }
        f
    }

    pub fn defines(&self) -> String {
        let mut text = String::new();
        for (on, name) in [
            (self.sensors, "CRAB_SENSORS"),
            (self.pcf, "CRAB_PCF"),
            (self.modbus, "CRAB_MODBUS"),
            (self.uart, "CRAB_UART"),
            (self.text, "CRAB_TEXT"),
            (self.net, "CRAB_NET"),
            (self.wifi, "CRAB_WIFI"),
            (self.eth, "CRAB_ETH"),
            (self.mqtt, "CRAB_MQTT"),
            (self.sparkplug, "CRAB_SPARKPLUG"),
            (self.web, "CRAB_WEB"),
            (!self.natives.is_empty(), "CRAB_NATIVE"),
        ] {
            if on {
                text.push_str(&format!(
                    "#define {name} 1
"
                ));
            }
        }
        text
    }

    fn libraries(&self) -> Vec<String> {
        let mut libs: Vec<String> = vec!["Wasm3".into()];
        if self.sensors {
            libs.extend(
                [
                    "OneWire",
                    "DallasTemperature",
                    "DHT sensor library",
                    "Adafruit Unified Sensor",
                ]
                .map(String::from),
            );
        }
        for lib in self.natives.iter().flat_map(|n| &n.libs) {
            if !libs.contains(lib) {
                libs.push(lib.clone());
            }
        }
        libs
    }
}

/// The board as the master is built for. The master is flashed over USB and never updates itself
/// over the air: its own table (`partitions.csv`) has the 2 MB app of the "No OTA" scheme, in
/// which the network stack fits, and the "crabmod" partition for the modules. The scheme option
/// stays "No OTA" so that arduino-cli checks the app against the same 2 MB. A scheme set in the
/// project would not match the table: an error.
fn master_board(board: &BoardConfig) -> Result<BoardConfig, BuildError> {
    let mut board = board.clone();
    if board.fqbn.starts_with("esp32:esp32:") {
        if let Some(scheme) = board_options(&board).get("PartitionScheme") {
            if scheme != MASTER_PARTITIONS {
                return Err(BuildError::MasterPartitions(scheme.clone()));
            }
        }
        board
            .options
            .insert("PartitionScheme".into(), MASTER_PARTITIONS.into());
    }
    Ok(board)
}

/// Defines for the libraries too (`compiler.cpp.extra_flags`), not only for the sketch.
fn extra_flags(features: &MasterFeatures) -> Option<String> {
    if !features.net {
        return None;
    }
    // The master never updates over the air: leave ArduinoOTA and Update out of the network
    // library (they also pull in AES/bignum from mbedcrypto).
    let mut flags = String::from("-DFLPROG_NETWORK_COMPILE_OTA=0");
    // Two MQTT buffers of this size: the texts of modules are up to 128 bytes, a topic up to 127,
    // so 384 is enough; the library's 768 stays for the NBIRTH of Sparkplug (up to 600 bytes).
    if features.mqtt && !features.sparkplug {
        flags.push_str(" -DFLPROG_MQTT_MAX_PACKET_SIZE=384");
    }
    Some(flags)
}

/// Partition scheme whose app size the master's own table keeps (see `master_board`).
const MASTER_PARTITIONS: &str = "no_ota";

/// Where the sketch takes the native parts of blocks: after its includes, so that the
/// prototypes the Arduino preprocessor makes already know the wasm3 types.
const NATIVE_MARKER: &str = "// @crab-native-glue";

/// Text form of [`master_sketch_id`] as the master reports it (`id=` in `stats`).
pub fn master_id_text(features: &MasterFeatures) -> String {
    format!("{:016x}", master_sketch_id(features))
}

/// `native=` of `stats`: `type:instances`, comma separated.
fn native_kinds_text(features: &MasterFeatures) -> String {
    features
        .natives
        .iter()
        .map(|n| format!("{}:{}", n.kind, n.instances))
        .collect::<Vec<_>>()
        .join(",")
}

/// How the master on a board relates to the one a project needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterState {
    /// The same firmware.
    Current,
    /// An older master that does not tell what it is.
    Unknown,
    /// A different build that still has everything the project's native blocks need.
    Differs,
    /// Native parts the project needs are not in the master: (block type, instances needed).
    MissingNative(Vec<(String, u32)>),
}

/// Start of the `id=` of the Rust master. That master is not built per project, so it counts as
/// current whenever it has the native parts a project needs (it has none, so only projects
/// without native blocks).
pub const RUST_MASTER_ID_PREFIX: &str = "rs-";

/// Compares what the master reported in `stats` with the master the project needs.
pub fn master_state(info: &crate::mcu_module::MasterInfo, needed: &MasterFeatures) -> MasterState {
    let Some(id) = &info.id else {
        return MasterState::Unknown;
    };
    let missing: Vec<(String, u32)> = needed
        .natives
        .iter()
        .filter(|n| {
            info.natives
                .iter()
                .find(|(kind, _)| *kind == n.kind)
                .is_none_or(|(_, have)| *have < n.instances)
        })
        .map(|n| (n.kind.clone(), n.instances))
        .collect();
    if !missing.is_empty() {
        MasterState::MissingNative(missing)
    } else if id.starts_with(RUST_MASTER_ID_PREFIX) || *id == master_id_text(needed) {
        MasterState::Current
    } else {
        MasterState::Differs
    }
}

/// The master sketch with the defines of its drivers in front.
fn sketch_text(features: &MasterFeatures) -> String {
    let glue = crate::native::glue(&features.natives);
    let body = if glue.is_empty() {
        SKETCH.to_owned()
    } else {
        // The whole marker line goes, so the sketch has no trace of it.
        let marker_line = SKETCH
            .lines()
            .find(|l| l.starts_with(NATIVE_MARKER))
            .expect("master.ino has the native marker");
        SKETCH.replacen(marker_line, &glue, 1)
    };
    let mut defines = features.defines();
    defines.push_str(&format!(
        "#define CRAB_MASTER_ID \"{}\"\n",
        master_id_text(features)
    ));
    if !features.natives.is_empty() {
        defines.push_str(&format!(
            "#define CRAB_NATIVE_KINDS \"{}\"\n",
            native_kinds_text(features)
        ));
    }
    format!("{defines}{}{body}", page_text(features))
}

/// The web page as a flash constant: the same page a C++ sketch serves, so it is not copied into
/// the sketch file.
fn page_text(features: &MasterFeatures) -> String {
    if features.web {
        format!(
            "const char crab_html[] PROGMEM = R\"CRABUI({})CRABUI\";
",
            crab_blocks::device_page()
        )
    } else {
        String::new()
    }
}

/// Identifies the master sketch with these drivers: changes when the firmware (protocol,
/// ABI, drivers) does.
pub fn master_sketch_id(features: &MasterFeatures) -> u64 {
    use std::hash::{Hash, Hasher};
    use std::sync::OnceLock;
    static BASE: OnceLock<u64> = OnceLock::new();
    let base = *BASE.get_or_init(|| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        SKETCH.hash(&mut hasher);
        hasher.finish()
    });
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (base, features).hash(&mut hasher);
    // The page is part of the firmware: a new page needs a new master.
    page_text(features).hash(&mut hasher);
    hasher.finish()
}

/// Compiled master firmware, ready to be flashed.
pub struct MasterBuild {
    sketch_dir: PathBuf,
    out: PathBuf,
}

impl ArduinoCli {
    /// Builds the master firmware in `<work_dir>/master` without touching the board.
    /// `low_priority` keeps a background build from slowing the editor down.
    pub fn compile_master(
        &self,
        work_dir: &Path,
        board: &BoardConfig,
        features: &MasterFeatures,
        low_priority: bool,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<MasterBuild, BuildError> {
        let board = &master_board(board)?;
        let root = plain_path(work_dir).join("master");
        let sketch_dir = root.join("master");
        std::fs::create_dir_all(&sketch_dir).map_err(|e| BuildError::BadOutput(e.to_string()))?;
        // Untouched sources keep arduino-cli's cache valid.
        for (file, text) in [
            ("master.ino", sketch_text(features)),
            ("partitions.csv", PARTITIONS.to_owned()),
        ] {
            let path = sketch_dir.join(file);
            if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                std::fs::write(&path, &text).map_err(|e| BuildError::BadOutput(e.to_string()))?;
            }
        }
        if let Some(core) = self.ensure_core(&board.fqbn, on_line)? {
            on_line(&format!("Установлено ядро {core}"));
        }
        let libs = features.libraries();
        let libs: Vec<&str> = libs.iter().map(String::as_str).collect();
        let installed = self.ensure_libraries(&libs, on_line)?;
        if !installed.is_empty() {
            on_line(&format!("Установлены библиотеки: {}", installed.join(", ")));
        }
        let out = build_dir(&root);

        let mut args: Vec<OsString> = vec!["compile".into(), "-v".into()];
        args.extend(board_args(board));
        args.push("--build-path".into());
        args.push(out.clone().into());
        if let Some(flags) = extra_flags(features) {
            args.push("--build-property".into());
            args.push(format!("compiler.cpp.extra_flags={flags}").into());
        }
        // Always: Wasm3 and the sensor libraries are shipped there too.
        for dir in crate::bundled_libraries(work_dir) {
            args.push("--libraries".into());
            args.push(dir.into());
        }
        args.push(sketch_dir.clone().into());
        let status = self.run(&args, low_priority, on_line)?;
        check_status("сборка мастера", status)?;
        Ok(MasterBuild { sketch_dir, out })
    }

    /// Flashes a compiled master to `port`. This replaces whatever firmware the board runs.
    pub fn flash_master(
        &self,
        build: &MasterBuild,
        board: &BoardConfig,
        port: &str,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), BuildError> {
        let board = &master_board(board)?;
        let mut args: Vec<OsString> = vec!["upload".into()];
        args.extend(board_args(board));
        args.push("--port".into());
        args.push(port.into());
        args.push("--input-dir".into());
        args.push(build.out.clone().into());
        args.push(build.sketch_dir.clone().into());
        let status = self.run(&args, false, on_line)?;
        check_status("заливка мастера", status)
    }

    /// Builds the master firmware in `<work_dir>/master` and uploads it to `port`.
    pub fn upload_master(
        &self,
        work_dir: &Path,
        board: &BoardConfig,
        features: &MasterFeatures,
        port: &str,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<(), BuildError> {
        let build = self.compile_master(work_dir, board, features, false, on_line)?;
        self.flash_master(&build, board, port, on_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drivers_follow_the_blocks() {
        let none = MasterFeatures::of_blocks(["timer_on", "digital_in"]);
        assert_eq!(none, MasterFeatures::default());
        assert_eq!(none.defines(), "");
        assert_eq!(none.libraries(), ["Wasm3"]);
        assert_eq!(none.libraries(), vec!["Wasm3".to_string()]);
        let both = MasterFeatures::of_blocks(["dht22", "pcf8574_out"]);
        assert_eq!(
            both.defines(),
            "#define CRAB_SENSORS 1\n#define CRAB_PCF 1\n"
        );
        assert!(both.libraries().contains(&"DHT sensor library".to_string()));
        assert_ne!(master_sketch_id(&none), master_sketch_id(&both));
        let modbus = MasterFeatures::of_blocks(["modbus_rtu_slave"]);
        assert_eq!(
            modbus.defines(),
            "#define CRAB_MODBUS 1
"
        );
        assert_ne!(master_sketch_id(&none), master_sketch_id(&modbus));
        let tcp = MasterFeatures::of_blocks(["modbus_tcp_master", "wifi"]);
        assert!(tcp.modbus && tcp.net && tcp.wifi && !tcp.eth);
        let uart = MasterFeatures::of_blocks(["uart_send"]);
        assert_eq!(uart.defines(), "#define CRAB_UART 1\n");
        assert_ne!(master_sketch_id(&none), master_sketch_id(&uart));
        let text = MasterFeatures::of_blocks(["text_concat", "uart_read"]);
        assert_eq!(
            text.defines(),
            "#define CRAB_UART 1
#define CRAB_TEXT 1
"
        );
        let net = MasterFeatures::of_blocks(["ntp", "wifi"]);
        assert_eq!(
            net.defines(),
            "#define CRAB_TEXT 1
#define CRAB_NET 1
#define CRAB_WIFI 1
"
        );
        let eth = MasterFeatures::of_blocks(["ethernet", "ntp"]);
        assert_eq!(
            eth.defines(),
            "#define CRAB_TEXT 1
#define CRAB_NET 1
#define CRAB_ETH 1
"
        );
        let wifi_eth = MasterFeatures::of_blocks(["wifi", "ethernet"]);
        assert!(
            wifi_eth.defines().contains("CRAB_WIFI") && wifi_eth.defines().contains("CRAB_ETH")
        );
        let mqtt = MasterFeatures::of_blocks(["mqtt", "mqtt_publish", "wifi"]);
        assert!(mqtt.defines().contains("CRAB_MQTT") && mqtt.defines().contains("CRAB_NET"));
        assert_ne!(master_sketch_id(&net), master_sketch_id(&mqtt));
        let web = MasterFeatures::of_blocks(["web_ui", "web_slider", "wifi"]);
        assert!(web.defines().contains("CRAB_WEB") && web.defines().contains("CRAB_NET"));
        assert!(sketch_text(&web).contains("const char crab_html[] PROGMEM"));
        assert!(!sketch_text(&mqtt).contains("const char crab_html[] PROGMEM"));
        assert_ne!(master_sketch_id(&net), master_sketch_id(&web));
        assert_ne!(master_sketch_id(&none), master_sketch_id(&net));
        assert!(sketch_text(&both).ends_with(SKETCH));
    }

    #[test]
    fn native_parts_go_into_the_master() {
        use crate::native::{NativeBlock, NativeType};
        let native = |instances| NativeBlock {
            kind: "bmp".into(),
            source: "// crab:native libs=\"Adafruit BMP280\"
#include <Adafruit_BMP280.h>
CRAB_STEP(State& s, In in, Out& out) {}
"
            .into(),
            libs: vec!["Adafruit BMP280".into()],
            has_init: false,
            has_state: false,
            inputs: vec![],
            outputs: vec![("t".into(), NativeType::Float)],
            params: vec![],
            instances,
        };
        let plain = MasterFeatures::default();
        let one = MasterFeatures {
            natives: vec![native(1)],
            ..Default::default()
        };
        let two = MasterFeatures {
            natives: vec![native(2)],
            ..Default::default()
        };
        assert_eq!(
            one.defines(),
            "#define CRAB_NATIVE 1
"
        );
        assert_eq!(plain.defines(), "");
        assert!(one.libraries().contains(&"Adafruit BMP280".to_string()));
        let text = sketch_text(&one);
        assert!(text.starts_with(
            "#define CRAB_NATIVE 1
"
        ));
        assert!(text.contains("namespace crab_nat_0") && !text.contains(NATIVE_MARKER));
        // After wasm3 and before the first slot constant, not at the top.
        let at = text.find("namespace crab_nat_0").unwrap();
        assert!(text.find("#include <wasm3.h>").unwrap() < at);
        assert!(at < text.find("static const int kMaxSlots").unwrap());
        // Another set of natives is another firmware: a board with the old one needs the new master.
        assert_ne!(master_sketch_id(&plain), master_sketch_id(&one));
        assert_ne!(master_sketch_id(&one), master_sketch_id(&two));
        assert_eq!(master_sketch_id(&one), master_sketch_id(&one.clone()));
        assert!(
            sketch_text(&plain).ends_with(SKETCH) && !sketch_text(&plain).contains("crab_nat_")
        );
    }

    #[test]
    fn master_state_names_the_missing_native_parts() {
        use crate::mcu_module::MasterInfo;
        use crate::native::{NativeBlock, NativeType};
        let native = |kind: &str, instances| NativeBlock {
            kind: kind.into(),
            source: "CRAB_STEP(State& s, In in, Out& out) {}
"
            .into(),
            libs: vec![],
            has_init: false,
            has_state: false,
            inputs: vec![],
            outputs: vec![("t".into(), NativeType::Float)],
            params: vec![],
            instances,
        };
        let needed = MasterFeatures {
            natives: vec![native("bmp", 2)],
            ..Default::default()
        };
        let id = master_id_text(&needed);
        let info = |id: Option<&str>, natives: &[(&str, u32)]| MasterInfo {
            id: id.map(str::to_owned),
            natives: natives.iter().map(|(k, n)| ((*k).to_owned(), *n)).collect(),
            ..Default::default()
        };
        // The same firmware.
        assert_eq!(
            master_state(&info(Some(&id), &[("bmp", 2)]), &needed),
            MasterState::Current
        );
        // An old master tells nothing.
        assert_eq!(
            master_state(&info(None, &[]), &needed),
            MasterState::Unknown
        );
        // The block is not there, or there are fewer instances than the project has.
        for have in [&[][..], &[("bmp", 1)][..], &[("other", 5)][..]] {
            assert_eq!(
                master_state(&info(Some("0"), have), &needed),
                MasterState::MissingNative(vec![("bmp".into(), 2)])
            );
        }
        // More than needed and another fingerprint: works, but it is another build.
        assert_eq!(
            master_state(&info(Some("0"), &[("bmp", 3)]), &needed),
            MasterState::Differs
        );
        // The Rust master is never rebuilt per project; missing natives still count.
        assert_eq!(
            master_state(&info(Some("rs-0.1.0"), &[]), &MasterFeatures::default()),
            MasterState::Current
        );
        assert_eq!(
            master_state(&info(Some("rs-0.1.0"), &[]), &needed),
            MasterState::MissingNative(vec![("bmp".into(), 2)])
        );
        // The sketch carries what the master will report.
        let text = sketch_text(&needed);
        assert!(text.contains(&format!("#define CRAB_MASTER_ID \"{id}\"")));
        assert!(text.contains("#define CRAB_NATIVE_KINDS \"bmp:2\""));
        assert!(!sketch_text(&MasterFeatures::default()).contains("#define CRAB_NATIVE_KINDS"));
    }

    #[test]
    fn mqtt_packets_are_small_unless_sparkplug() {
        let flags = |kinds: &[&str]| extra_flags(&MasterFeatures::of_blocks(kinds.iter().copied()));
        assert_eq!(flags(&["ds18b20"]), None);
        assert_eq!(flags(&["wifi"]).unwrap(), "-DFLPROG_NETWORK_COMPILE_OTA=0");
        assert!(flags(&["wifi", "mqtt"])
            .unwrap()
            .ends_with(" -DFLPROG_MQTT_MAX_PACKET_SIZE=384"));
        let spark = flags(&["wifi", "mqtt", "sparkplug_node"]).unwrap();
        assert!(!spark.contains("MQTT_MAX_PACKET_SIZE"));
        // Another master build: a master without Sparkplug is not the one a Sparkplug project needs.
        let id =
            |kinds: &[&str]| master_sketch_id(&MasterFeatures::of_blocks(kinds.iter().copied()));
        assert_ne!(id(&["mqtt"]), id(&["mqtt", "sparkplug_metric"]));
    }

    #[test]
    fn master_takes_its_own_partitions() {
        let mut board = BoardConfig {
            fqbn: "esp32:esp32:esp32c3".into(),
            model: None,
            port: None,
            options: Default::default(),
        };
        let scheme = |b: &BoardConfig| master_board(b).unwrap().options["PartitionScheme"].clone();
        assert_eq!(scheme(&board), "no_ota");
        board.options.insert("FlashSize".into(), "4M".into());
        assert_eq!(scheme(&board), "no_ota");
        // A scheme of the project does not match the master's table.
        board
            .options
            .insert("PartitionScheme".into(), "huge_app".into());
        assert!(matches!(
            master_board(&board),
            Err(BuildError::MasterPartitions(s)) if s == "huge_app"
        ));
        board
            .options
            .insert("PartitionScheme".into(), "no_ota".into());
        assert_eq!(scheme(&board), "no_ota");
        // A scheme written into the FQBN counts too.
        board.options.clear();
        board.fqbn = "esp32:esp32:esp32c3:CDCOnBoot=cdc,PartitionScheme=huge_app".into();
        assert!(matches!(
            master_board(&board),
            Err(BuildError::MasterPartitions(s)) if s == "huge_app"
        ));
    }

    /// The table keeps the 2 MB app of "No OTA" and holds the module areas of the master
    /// (`kAreaBytes` × whole areas), within 4 MB of flash.
    #[test]
    fn partition_table_has_the_module_areas() {
        let num = |s: &str| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap();
        let rows: Vec<Vec<&str>> = PARTITIONS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .map(|l| l.split(',').map(str::trim).collect())
            .collect();
        let row = |name: &str| rows.iter().find(|r| r[0] == name).unwrap();
        assert_eq!(num(row("app0")[4]), 0x20_0000);
        let modules = row("crabmod");
        assert_eq!((modules[1], modules[2]), ("data", "0x40"));
        let area = 0x11000;
        assert!(SKETCH.contains("kAreaBytes = 0x11000"));
        assert_eq!(num(modules[4]) % area, 0);
        assert!(num(modules[4]) / area >= 8);
        let mut end = 0;
        for r in &rows {
            let (at, size) = (num(r[3]), num(r[4]));
            assert!(at >= end, "{} overlaps", r[0]);
            assert_eq!(at % 0x1000, 0);
            end = at + size;
        }
        assert!(end <= 0x40_0000);
        assert!(SKETCH.contains("\"crabmod\""));
    }
}
