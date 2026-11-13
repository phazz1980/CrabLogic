//! Flashing the prebuilt Rust master (`firmware/<chip>/` next to the program) without arduino-cli:
//! `espflash` as a library builds the ESP-IDF image from the master's ELF (its own second-stage
//! bootloader, the partition table of the image) and writes it over the USB serial port.

use std::path::{Path, PathBuf};

use espflash::connection::{Connection, ResetAfterOperation, ResetBeforeOperation};
use espflash::flasher::{FlashData, FlashFrequency, FlashMode, FlashSettings, Flasher};
use espflash::image_format::idf::IdfBootloaderFormat;
use espflash::image_format::ImageFormat;
use espflash::target::{Chip, ProgressCallbacks};
use serialport::{FlowControl, SerialPortType, UsbPortInfo};

/// Folder of the master images in a distribution (next to the program).
pub const FIRMWARE_DIR: &str = "firmware";
/// File names inside `firmware/<chip>/`.
pub const ELF_FILE: &str = "crab-master-rs.elf";
pub const PARTITIONS_FILE: &str = "partitions.csv";
pub const INFO_FILE: &str = "info.toml";

/// Chips the Rust master is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterChip {
    Esp32,
    Esp32c3,
}

impl MasterChip {
    /// The chip of a board by its FQBN (`esp32:esp32:<board>`); `None` for boards the Rust master
    /// does not run on (ESP32-S2/S3/C6, other cores).
    pub fn of_fqbn(fqbn: &str) -> Option<Self> {
        let mut parts = fqbn.split(':');
        let (vendor, arch, board) = (parts.next()?, parts.next()?, parts.next()?);
        if vendor != "esp32" || arch != "esp32" {
            return None;
        }
        let board = board.to_ascii_lowercase();
        if board.contains("c3") {
            Some(Self::Esp32c3)
        } else if ["s2", "s3", "c2", "c5", "c6", "h2", "p4"]
            .iter()
            .any(|chip| board.contains(&format!("esp32{chip}")))
        {
            None
        } else {
            Some(Self::Esp32)
        }
    }

    /// Folder name inside `firmware/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Esp32 => "esp32",
            Self::Esp32c3 => "esp32c3",
        }
    }

    fn espflash(self) -> Chip {
        match self {
            Self::Esp32 => Chip::Esp32,
            Self::Esp32c3 => Chip::Esp32c3,
        }
    }
}

/// What `info.toml` of an image tells.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct ImageInfo {
    #[serde(default)]
    pub version: String,
    /// `MODCAPS` of the image, as the master reports it.
    #[serde(default)]
    pub modcaps: String,
}

/// A master image ready to be flashed.
#[derive(Debug, Clone)]
pub struct MasterImage {
    pub chip: MasterChip,
    pub elf: PathBuf,
    pub partitions: PathBuf,
    pub info: ImageInfo,
}

#[derive(Debug, thiserror::Error)]
pub enum FlashError {
    #[error("Rust-мастер не работает на плате {0}: нужна ESP32 или ESP32-C3")]
    UnsupportedBoard(String),
    #[error("нет образа мастера для {chip}: ожидался {path}")]
    MissingImage { chip: &'static str, path: PathBuf },
    #[error("не удалось прочитать {path}: {message}")]
    Read { path: PathBuf, message: String },
    #[error("порт {port}: {message}")]
    Port { port: String, message: String },
    #[error("прошивка: {0}")]
    Flash(String),
}

/// Where the master images are: `firmware/` next to the program, then up the folders (a checkout
/// runs from `target/<profile>`, the images are in `dist/CrabLogic/firmware`).
pub fn firmware_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().skip(1).find_map(|dir| {
        [
            dir.join(FIRMWARE_DIR),
            dir.join("dist").join("CrabLogic").join(FIRMWARE_DIR),
        ]
        .into_iter()
        .find(|d| d.is_dir())
    })
}

/// The image for a board in `firmware_dir`.
pub fn find_image(firmware_dir: &Path, fqbn: &str) -> Result<MasterImage, FlashError> {
    let chip =
        MasterChip::of_fqbn(fqbn).ok_or_else(|| FlashError::UnsupportedBoard(fqbn.to_owned()))?;
    let dir = firmware_dir.join(chip.dir_name());
    let elf = dir.join(ELF_FILE);
    let partitions = dir.join(PARTITIONS_FILE);
    for path in [&elf, &partitions] {
        if !path.is_file() {
            return Err(FlashError::MissingImage {
                chip: chip.dir_name(),
                path: path.clone(),
            });
        }
    }
    let info_path = dir.join(INFO_FILE);
    let info = match std::fs::read_to_string(&info_path) {
        Ok(text) => toml::from_str(&text).map_err(|e| FlashError::Read {
            path: info_path,
            message: e.to_string(),
        })?,
        Err(_) => ImageInfo::default(),
    };
    Ok(MasterImage {
        chip,
        elf,
        partitions,
        info,
    })
}

/// How the master on a board (its `id=` in `stats`) relates to the image of the program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterVerdict {
    /// A Rust master of the same version or newer.
    Current,
    /// A Rust master older than the image: (on the board, in the program).
    Older(String, String),
    /// Not the Rust master (the C++ one, or a master that does not tell its id).
    NotRust,
}

/// Start of the `id=` of the Rust master (`rs-<version>`).
pub const RUST_ID_PREFIX: &str = "rs-";

/// Compares `id=` of the master with the version of the image (`info.toml`); an image without a
/// version counts as current.
pub fn master_verdict(id: Option<&str>, image_version: Option<&str>) -> MasterVerdict {
    let Some(have) = id.and_then(|id| id.strip_prefix(RUST_ID_PREFIX)) else {
        return MasterVerdict::NotRust;
    };
    let numbers = |v: &str| -> Vec<u64> {
        v.split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    match image_version.filter(|v| !numbers(v).is_empty()) {
        Some(want) if numbers(have) < numbers(want) => {
            MasterVerdict::Older(have.to_owned(), want.to_owned())
        }
        _ => MasterVerdict::Current,
    }
}

/// Version of the image for the board of `fqbn` in the distribution, if there is one.
pub fn image_version(fqbn: &str) -> Option<String> {
    let dir = firmware_dir()?;
    let image = find_image(&dir, fqbn).ok()?;
    Some(image.info.version).filter(|v| !v.is_empty())
}

/// USB serial ports of the computer (where boards show up), by name.
pub fn usb_ports() -> Vec<String> {
    let mut ports: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| matches!(p.port_type, SerialPortType::UsbPort(_)))
        .map(|p| p.port_name)
        .collect();
    ports.sort();
    ports
}

/// The only USB serial port, or an error that lists what is there.
pub fn single_usb_port() -> Result<String, FlashError> {
    let ports = usb_ports();
    match ports.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(FlashError::Port {
            port: "USB".into(),
            message: "плата не найдена: подключите её по USB".into(),
        }),
        many => Err(FlashError::Port {
            port: many.join(", "),
            message: "найдено несколько портов, укажите нужный (--port)".into(),
        }),
    }
}

/// Progress of the writing, reported in whole tens of percent.
struct Progress<'a> {
    on_line: &'a mut dyn FnMut(&str),
    addr: u32,
    total: usize,
    shown: usize,
}

impl ProgressCallbacks for Progress<'_> {
    fn init(&mut self, addr: u32, total: usize) {
        self.addr = addr;
        self.total = total.max(1);
        self.shown = 0;
    }
    fn update(&mut self, current: usize) {
        let tenth = current * 10 / self.total;
        if tenth > self.shown {
            self.shown = tenth;
            (self.on_line)(&format!("0x{:06x}: {} %", self.addr, tenth * 10));
        }
    }
    fn verifying(&mut self) {}
    fn finish(&mut self, skipped: bool) {
        if skipped {
            (self.on_line)(&format!("0x{:06x}: без изменений, пропущено", self.addr));
        }
    }
}

/// Flashes the image to the board on `port` and restarts it. Replaces the firmware of the board;
/// the modules in the `crabmod` partition stay (the master checks them at start).
pub fn flash_master(
    image: &MasterImage,
    port: &str,
    on_line: &mut dyn FnMut(&str),
) -> Result<(), FlashError> {
    let read = |path: &Path| {
        std::fs::read(path).map_err(|e| FlashError::Read {
            path: path.to_owned(),
            message: e.to_string(),
        })
    };
    let elf = read(&image.elf)?;
    let port_err = |message: String| FlashError::Port {
        port: port.to_owned(),
        message,
    };
    let usb = serialport::available_ports()
        .ok()
        .and_then(|ports| ports.into_iter().find(|p| p.port_name == port))
        .and_then(|p| match p.port_type {
            SerialPortType::UsbPort(info) => Some(info),
            _ => None,
        })
        .unwrap_or(UsbPortInfo {
            vid: 0,
            pid: 0,
            serial_number: None,
            manufacturer: None,
            product: None,
        });
    let serial = serialport::new(port, 115_200)
        .flow_control(FlowControl::None)
        .open_native()
        .map_err(|e| port_err(e.to_string()))?;
    let connection = Connection::new(
        serial,
        usb,
        ResetAfterOperation::HardReset,
        ResetBeforeOperation::DefaultReset,
        115_200,
    );
    on_line(&format!("Подключение к {port}..."));
    let flash_err = |e: espflash::Error| FlashError::Flash(e.to_string());
    let mut flasher = Flasher::connect(
        connection,
        true,
        true,
        true,
        Some(image.chip.espflash()),
        Some(460_800),
    )
    .map_err(flash_err)?;
    let chip = flasher.chip();
    let xtal = chip
        .xtal_frequency(flasher.connection())
        .map_err(flash_err)?;
    let size = flasher.flash_detect().ok().flatten();
    // The classic ESP32 master is built for 80 MHz flash (`sdkconfig.defaults.esp32`); the
    // image header must say so, or the bootloader keeps the slow default of 40 MHz.
    let freq = (chip == Chip::Esp32).then_some(FlashFrequency::_80Mhz);
    let flash_data = FlashData::new(
        FlashSettings::new(Some(FlashMode::Dio), size, freq),
        0,
        None,
        chip,
        xtal,
    );
    let format =
        IdfBootloaderFormat::new(&elf, &flash_data, Some(&image.partitions), None, None, None)
            .map_err(flash_err)?;
    on_line(&format!(
        "Запись мастера {} ({})...",
        image.chip.dir_name(),
        if image.info.version.is_empty() {
            "версия не указана"
        } else {
            image.info.version.as_str()
        }
    ));
    let mut progress = Progress {
        on_line,
        addr: 0,
        total: 1,
        shown: 0,
    };
    flasher
        .load_image_to_flash(&mut progress, ImageFormat::EspIdf(format))
        .map_err(flash_err)?;
    flasher
        .connection()
        .reset_after(true, chip)
        .map_err(flash_err)?;
    (progress.on_line)("Мастер записан, плата перезапущена");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_follows_the_board() {
        assert_eq!(
            MasterChip::of_fqbn("esp32:esp32:esp32"),
            Some(MasterChip::Esp32)
        );
        assert_eq!(
            MasterChip::of_fqbn("esp32:esp32:esp32da"),
            Some(MasterChip::Esp32)
        );
        assert_eq!(
            MasterChip::of_fqbn("esp32:esp32:esp32c3"),
            Some(MasterChip::Esp32c3)
        );
        assert_eq!(
            MasterChip::of_fqbn("esp32:esp32:lolin_c3_mini"),
            Some(MasterChip::Esp32c3)
        );
        assert_eq!(MasterChip::of_fqbn("esp32:esp32:esp32s3"), None);
        assert_eq!(MasterChip::of_fqbn("arduino:avr:uno"), None);
    }

    #[test]
    fn master_on_the_board_is_compared_with_the_image() {
        assert_eq!(master_verdict(None, Some("2.0.0")), MasterVerdict::NotRust);
        assert_eq!(
            master_verdict(Some("0123456789abcdef"), Some("2.0.0")),
            MasterVerdict::NotRust
        );
        assert_eq!(
            master_verdict(Some("rs-0.1.0"), Some("2.0.0")),
            MasterVerdict::Older("0.1.0".into(), "2.0.0".into())
        );
        assert_eq!(
            master_verdict(Some("rs-2.0.0"), Some("2.0.0")),
            MasterVerdict::Current
        );
        assert_eq!(
            master_verdict(Some("rs-2.0.10"), Some("2.0.9")),
            MasterVerdict::Current
        );
        assert_eq!(
            master_verdict(Some("rs-0.1.0"), None),
            MasterVerdict::Current
        );
    }

    #[test]
    fn image_is_found_by_chip() {
        let root = std::env::temp_dir().join(format!("crab_flash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("esp32c3");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(
            find_image(&root, "esp32:esp32:esp32c3"),
            Err(FlashError::MissingImage { .. })
        ));
        std::fs::write(dir.join(ELF_FILE), b"elf").unwrap();
        std::fs::write(dir.join(PARTITIONS_FILE), b"csv").unwrap();
        std::fs::write(
            dir.join(INFO_FILE),
            "version = \"2.0.0\"\nmodcaps = \"flashmod,abi4\"\n",
        )
        .unwrap();
        let image = find_image(&root, "esp32:esp32:esp32c3").unwrap();
        assert_eq!(image.chip, MasterChip::Esp32c3);
        assert_eq!(image.info.version, "2.0.0");
        assert!(matches!(
            find_image(&root, "esp32:esp32:esp32"),
            Err(FlashError::MissingImage { chip: "esp32", .. })
        ));
        assert!(matches!(
            find_image(&root, "esp32:esp32:esp32s3"),
            Err(FlashError::UnsupportedBoard(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }
}
