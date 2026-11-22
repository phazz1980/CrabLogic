//! Edits of the electrical panel that keep its links consistent (`specs/panel-editor`): who
//! feeds whom, and which input or output of the board a device is bound to.
//!
//! Every operation checks everything first and changes nothing when it refuses.

use crate::panel::{DeviceKind, Panel};
use crate::{panel_binding, panel_tags, Library, Project};

/// Kinds that are never fed by another device: a feed is the source, a sensor, a button and a
/// terminal are not in the power graph (`specs/electrical-schematics`, "Топология цепей").
fn never_fed(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Feed | DeviceKind::Sensor | DeviceKind::Button | DeviceKind::Aux
    )
}

/// Kinds that can feed their load: the breakers, the supplies, the switching devices and the
/// terminal blocks.
fn can_feed(kind: DeviceKind) -> bool {
    matches!(
        kind,
        DeviceKind::Feed
            | DeviceKind::Breaker
            | DeviceKind::Rcd
            | DeviceKind::Fuse
            | DeviceKind::Switch
            | DeviceKind::Psu
            | DeviceKind::Contactor
            | DeviceKind::Relay
            | DeviceKind::Terminal
    )
}

/// Name of a kind in the messages.
fn kind_name(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Feed => "ввод",
        DeviceKind::Breaker => "автомат",
        DeviceKind::Rcd => "УЗО",
        DeviceKind::Fuse => "предохранитель",
        DeviceKind::Switch => "рубильник",
        DeviceKind::Psu => "блок питания",
        DeviceKind::Contactor => "контактор",
        DeviceKind::Relay => "реле",
        DeviceKind::Motor => "двигатель",
        DeviceKind::Load => "нагрузка",
        DeviceKind::Terminal => "клеммник",
        DeviceKind::Sensor => "датчик",
        DeviceKind::Button => "кнопка",
        DeviceKind::Aux => "доп. контакт",
    }
}

impl Panel {
    /// Puts `device` on a branch of the line of `source` (`from`): the source keeps what it feeds
    /// and feeds this device as well; the sheet draws the junction by itself. `None` takes the
    /// device off the branch. Nothing may feed itself through the branches.
    pub fn branch_power(&mut self, device: &str, source: Option<&str>) -> Result<(), String> {
        let kind = self
            .device(device)
            .ok_or_else(|| format!("нет устройства «{device}»"))?
            .kind;
        let Some(source) = source else {
            if let Some(d) = self.devices.iter_mut().find(|d| d.id == device) {
                d.from.clear();
            }
            return Ok(());
        };
        let source_kind = self
            .device(source)
            .ok_or_else(|| format!("нет устройства «{source}»"))?
            .kind;
        if source == device {
            return Err(format!("-{device}: устройство не может питать само себя"));
        }
        if never_fed(kind) {
            return Err(format!(
                "-{device}: {} не питается от другого устройства",
                kind_name(kind)
            ));
        }
        if !can_feed(source_kind) {
            return Err(format!(
                "-{source}: {} не может питать другие устройства",
                kind_name(source_kind)
            ));
        }
        // Walking up from the source must not come back to the device.
        let mut up = source.to_owned();
        for _ in 0..=self.devices.len() {
            if up == device {
                return Err(format!(
                    "-{device} уже питает -{source}: цепь замкнулась бы сама на себя"
                ));
            }
            let next = self
                .devices
                .iter()
                .find(|d| d.id == up && !d.from.is_empty())
                .map(|d| d.from.clone())
                .or_else(|| {
                    self.devices
                        .iter()
                        .find(|p| p.load == up)
                        .map(|p| p.id.clone())
                });
            match next {
                Some(next) => up = next,
                None => break,
            }
        }
        for d in &mut self.devices {
            if d.load == device {
                d.load.clear();
            }
            if d.id == device {
                d.from = source.to_owned();
                d.free = false;
            }
        }
        Ok(())
    }

    /// Makes `parent` feed `device`: `parent.load = device`. `None`, or a feed as the parent, means
    /// the device is fed straight from the feed, so no device feeds it afterwards. A device that
    /// fed `device` before is released; a load that `parent` fed before is replaced (to feed
    /// several devices from one source, see [`Self::branch_power`]).
    pub fn reconnect_power(&mut self, device: &str, parent: Option<&str>) -> Result<(), String> {
        let kind = self
            .device(device)
            .ok_or_else(|| format!("нет устройства «{device}»"))?
            .kind;
        let feeder = match parent {
            None => None,
            Some(id) if id == device => {
                return Err(format!("-{device}: устройство не может питать само себя"));
            }
            Some(id) => {
                let parent_kind = self
                    .device(id)
                    .ok_or_else(|| format!("нет устройства «{id}»"))?
                    .kind;
                // A feed as the parent means the device is fed straight from the feed.
                (parent_kind != DeviceKind::Feed).then_some((id, parent_kind))
            }
        };
        if let Some((id, parent_kind)) = feeder {
            if never_fed(kind) {
                return Err(format!(
                    "-{device}: {} не питается от другого устройства",
                    kind_name(kind)
                ));
            }
            if !can_feed(parent_kind) {
                return Err(format!(
                    "-{id}: {} не может питать другие устройства",
                    kind_name(parent_kind)
                ));
            }
            // Walk down the loads from the device: reaching the parent would close a loop.
            let mut current = device;
            for _ in 0..=self.devices.len() {
                if current == id {
                    return Err(format!(
                        "-{id} уже питается от -{device}: цепь замкнулась бы сама на себя"
                    ));
                }
                match self.device(current).map(|d| d.load.as_str()) {
                    Some(next) if !next.is_empty() => current = next,
                    _ => break,
                }
            }
        }

        // Nothing is refused from here on.
        for d in &mut self.devices {
            if d.load == device {
                d.load.clear();
            }
            if d.id == device {
                d.from.clear();
                d.free = false;
            }
        }
        if let Some((id, _)) = feeder {
            if let Some(d) = self.devices.iter_mut().find(|d| d.id == id) {
                d.load = device.to_owned();
            }
        }
        Ok(())
    }
}

/// Renames a panel device together with its manual layout (see `Panel::rename`).
pub fn rename_device(project: &mut Project, old: &str, new: &str) -> Result<(), String> {
    project.panel.rename(old, new)?;
    project.panel_layout.rename(old, new);
    Ok(())
}

/// Renames several panel devices at once together with the layout (see `Panel::rename_all`).
pub fn rename_devices(project: &mut Project, names: &[(String, String)]) -> Result<(), String> {
    project.panel.rename_all(names)?;
    project.panel_layout.rename_all(names);
    Ok(())
}

/// Binds the device to the input or output `tag` of the project (`None` clears the binding).
/// The tag is checked as in [`crate::check_panel`]: it must exist, its direction must fit the
/// device, a coil or a button needs a digital signal, and no other device may be bound to it.
pub fn rebind_device(
    project: &mut Project,
    lib: &Library,
    device: &str,
    tag: Option<&str>,
) -> Result<(), String> {
    let kind = project
        .panel
        .device(device)
        .ok_or_else(|| format!("нет устройства «{device}»"))?
        .kind;
    let designation = format!("-{device}");
    let Some(tag) = tag else {
        set_bind(project, device, String::new());
        return Ok(());
    };

    let Some((want_input, digital_only)) = panel_binding(kind) else {
        return Err(format!(
            "{designation}: {} не привязывается к входу или выходу платы",
            kind_name(kind)
        ));
    };
    let name = tag.split('.').next().unwrap_or_default();
    let target = project.logic.tag_of(name).to_owned();
    let tags = panel_tags(project, lib);
    let Some(found) = tags.iter().find(|t| t.name == target) else {
        return Err(format!(
            "{designation}: в проекте нет входа или выхода «{target}»"
        ));
    };
    if found.input != want_input {
        let (need, have) = if want_input {
            ("входу", "выход")
        } else {
            ("выходу", "вход")
        };
        return Err(format!(
            "{designation}: {} привязывается к {need} платы, а «{target}» — {have}",
            kind_name(kind)
        ));
    }
    if digital_only && !found.digital {
        return Err(format!(
            "{designation}: {} привязывается к дискретному сигналу, а «{target}» — аналоговый (АЦП, ШИМ или ЦАП)",
            kind_name(kind)
        ));
    }
    let owner = project.panel.devices.iter().find(|d| {
        d.id != device
            && !d.bind.is_empty()
            && project
                .logic
                .tag_of(d.bind.split('.').next().unwrap_or_default())
                == target
    });
    if let Some(owner) = owner {
        return Err(format!(
            "«{target}» уже привязан к {}, второе устройство — {designation}",
            owner.designation()
        ));
    }
    set_bind(project, device, tag.to_owned());
    Ok(())
}

fn set_bind(project: &mut Project, device: &str, bind: String) {
    if let Some(d) = project.panel.devices.iter_mut().find(|d| d.id == device) {
        d.bind = bind;
    }
}
