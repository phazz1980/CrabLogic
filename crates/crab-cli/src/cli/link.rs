use std::path::Path;

use crab_core::Project;

/// How to reach the board: over USB (`port`, or the only found one) or over the network (`host`).
pub(crate) struct LinkSpec {
    pub(crate) port: Option<String>,
    pub(crate) host: Option<String>,
    /// Project password, the source of the network key.
    pub(crate) password: String,
}

impl LinkSpec {
    pub(crate) fn serial(port: Option<String>) -> Self {
        LinkSpec {
            port,
            host: None,
            password: String::new(),
        }
    }
}

/// The link of `upload-wasm`: `--host` (with the password of `--project`) or `--port`; with neither,
/// the choice by `--project` (a saved port that is present goes by USB, else the saved address) or
/// the only port found.
pub(crate) fn explicit_link(
    port: Option<String>,
    host: Option<String>,
    project: Option<&Path>,
) -> Result<LinkSpec, String> {
    if port.is_some() && host.is_some() {
        return Err("укажите что-то одно: --port или --host".into());
    }
    let loaded = match project {
        Some(path) => Some(Project::load(path).map_err(|e| e.to_string())?),
        None => None,
    };
    let password = loaded
        .as_ref()
        .map(|p| p.config.project.password.clone())
        .unwrap_or_default();
    if host.is_some() && loaded.is_none() {
        return Err(
            "для --host нужен --project <файл проекта>: пароль для входа по сети берётся из проекта"
                .into(),
        );
    }
    if port.is_some() || host.is_some() {
        return Ok(LinkSpec {
            port,
            host,
            password,
        });
    }
    match loaded {
        Some(project) => project_link_spec(&project),
        None => Ok(LinkSpec::serial(None)),
    }
}

/// The link of the project by the serial ports present now.
pub(crate) fn project_link_spec(project: &Project) -> Result<LinkSpec, String> {
    let mut link = LinkSpec::serial(None);
    link.password = project.config.project.password.clone();
    match crab_build::project_link(&project.config.board)? {
        crab_build::Chosen::Serial(port) => link.port = Some(port),
        crab_build::Chosen::Net(host) => link.host = Some(host),
    }
    Ok(link)
}

/// Opens the link to the master: TCP with the login for `host`, otherwise the serial port.
/// Returns the link and a name for messages.
pub(crate) fn open_link(
    spec: LinkSpec,
) -> Result<(Box<dyn crab_build::net_link::Link>, String), String> {
    match spec.host {
        Some(host) => {
            let stream =
                crab_build::net_link::connect(&host, &spec.password).map_err(|e| e.to_string())?;
            Ok((Box::new(stream), host))
        }
        None => {
            let (serial, port) = open_master_port(spec.port)?;
            Ok((Box::new(serial), port))
        }
    }
}

/// Opens the serial port of the board (the only found one if `port` is not given) without
/// resetting it. Returns the port and its name.
pub(crate) fn open_master_port(
    port: Option<String>,
) -> Result<(Box<dyn serialport::SerialPort>, String), String> {
    let port = match port {
        Some(port) => port,
        None => crab_build::serial_ports()
            .and_then(|ports| crab_build::auto_port(&ports))
            .map_err(|e| e.to_string())?,
    };
    let mut serial = serialport::new(&port, 115_200)
        .timeout(std::time::Duration::from_millis(200))
        .open()
        .map_err(|e| format!("порт {port} не открыт: {e}"))?;
    // Release DTR/RTS so opening the port does not reset the board.
    let _ = serial.write_data_terminal_ready(false);
    let _ = serial.write_request_to_send(false);
    Ok((serial, port))
}
