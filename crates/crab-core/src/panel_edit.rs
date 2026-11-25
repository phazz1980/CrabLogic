//! Edits of the electrical panel that keep its links consistent (`specs/panel-editor`): who
//! feeds whom, and which input or output of the board a device is bound to.
//!
//! Every operation checks everything first and changes nothing when it refuses.

use crate::panel::{link_device, ContactForm, DeviceKind, Panel, WireLink};
use crate::{panel_binding, panel_tags, Library, ParamKind, ParamValue, Project};

/// Designation of the controller board in the wires (the same as `crab_schematic::BOARD`).
const BOARD: &str = "A1";

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
    /// Kind of `device` and whether it is a contact of a relay, which switches a load of its own
    /// and so is fed and feeds like a one-pole switching device.
    fn feed_kind(&self, device: &str) -> Result<(DeviceKind, bool), String> {
        let d = self
            .device(device)
            .ok_or_else(|| format!("нет устройства «{device}»"))?;
        Ok((d.kind, self.is_relay_contact(d)))
    }

    /// Puts `device` on a branch of the line of `source` (`from`): the source keeps what it feeds
    /// and feeds this device as well; the sheet draws the junction by itself. `None` takes the
    /// device off the branch. Nothing may feed itself through the branches.
    pub fn branch_power(&mut self, device: &str, source: Option<&str>) -> Result<(), String> {
        let (kind, relay_contact) = self.feed_kind(device)?;
        let Some(source) = source else {
            if let Some(d) = self.devices.iter_mut().find(|d| d.id == device) {
                d.from.clear();
            }
            return Ok(());
        };
        let (source_kind, source_relay_contact) = self.feed_kind(source)?;
        if source == device {
            return Err(format!("-{device}: устройство не может питать само себя"));
        }
        if never_fed(kind) && !relay_contact {
            return Err(format!(
                "-{device}: {} не питается от другого устройства",
                kind_name(kind)
            ));
        }
        if !can_feed(source_kind) && !source_relay_contact {
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

    /// «Питается от», the one way the editor connects power: `parent` feeds `device`. A parent
    /// that feeds nothing yet gets the device as its load ([`Self::reconnect_power`]); one that
    /// already feeds another device feeds this one too, on a branch of its line
    /// ([`Self::branch_power`]). `None`, or the feed, means straight from the feed. What fed the
    /// device before lets it go. Nothing changes when it refuses.
    pub fn feed_from(&mut self, device: &str, parent: Option<&str>) -> Result<(), String> {
        let mut next = self.clone();
        next.reconnect_power(device, None)?;
        if let Some(parent) = parent {
            let source = next
                .device(parent)
                .ok_or_else(|| format!("нет устройства «{parent}»"))?;
            let busy =
                source.kind != DeviceKind::Feed && !source.load.is_empty() && source.load != device;
            if busy {
                next.branch_power(device, Some(parent))?;
            } else {
                next.reconnect_power(device, Some(parent))?;
            }
        }
        *self = next;
        Ok(())
    }

    /// Makes `parent` feed `device`: `parent.load = device`. `None`, or a feed as the parent, means
    /// the device is fed straight from the feed, so no device feeds it afterwards. A device that
    /// fed `device` before is released; a load that `parent` fed before is replaced (to feed
    /// several devices from one source, see [`Self::branch_power`]).
    pub fn reconnect_power(&mut self, device: &str, parent: Option<&str>) -> Result<(), String> {
        let (kind, relay_contact) = self.feed_kind(device)?;
        let feeder = match parent {
            None => None,
            Some(id) if id == device => {
                return Err(format!("-{device}: устройство не может питать само себя"));
            }
            Some(id) => {
                let parent = self.feed_kind(id)?;
                // A feed as the parent means the device is fed straight from the feed.
                (parent.0 != DeviceKind::Feed).then_some((id, parent))
            }
        };
        if let Some((id, (parent_kind, parent_relay_contact))) = feeder {
            if never_fed(kind) && !relay_contact {
                return Err(format!(
                    "-{device}: {} не питается от другого устройства",
                    kind_name(kind)
                ));
            }
            if !can_feed(parent_kind) && !parent_relay_contact {
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

/// Gives the auxiliary contact `id` another owner `of`: a relay or a contactor (empty: a free
/// contact, which waits for its coil). The contact takes the next free designation of its owner
/// (`K2_1`; a free contact is `K_1`), and its layout and links follow it. A changeover form stays
/// only on a relay: a contactor has normally open or normally closed contacts. The contacts of a
/// breaker or an RCD do not move. Returns the new designation.
pub fn reassign_contact(project: &mut Project, id: &str, of: &str) -> Result<String, String> {
    let panel = &project.panel;
    let contact = panel
        .device(id)
        .filter(|d| d.kind == DeviceKind::Aux)
        .ok_or_else(|| format!("«{id}»: нет такого контакта"))?;
    if contact.of == of {
        return Ok(id.to_owned());
    }
    if matches!(
        panel.device(&contact.of).map(|d| d.kind),
        Some(DeviceKind::Breaker | DeviceKind::Rcd)
    ) {
        return Err(format!(
            "контакт «{id}» принадлежит автомату или УЗО и не переносится"
        ));
    }
    let owner_kind = if of.is_empty() {
        None
    } else {
        let owner = panel
            .device(of)
            .filter(|d| matches!(d.kind, DeviceKind::Relay | DeviceKind::Contactor))
            .ok_or_else(|| format!("«{of}»: нет такого реле или контактора"))?;
        Some(owner.kind)
    };
    // A relay contact that switches a load of its own stays a relay contact: only a relay's
    // contact may be fed and feed (the check forbids it on a contactor or a free contact).
    let powered = !contact.from.is_empty()
        || !contact.load.is_empty()
        || panel.devices.iter().any(|d| d.load == id || d.from == id);
    if powered && owner_kind != Some(DeviceKind::Relay) {
        return Err(format!(
            "контакт «{id}» коммутирует нагрузку: его можно перенести только на другое реле"
        ));
    }
    let form = match (owner_kind, contact.form) {
        (Some(DeviceKind::Contactor), ContactForm::Changeover) => ContactForm::No,
        (_, form) => form,
    };
    let new_id = panel.next_aux_id(&if of.is_empty() {
        DeviceKind::Relay.letter().to_string()
    } else {
        of.to_owned()
    });
    // Renamed first: a refused name changes nothing.
    rename_device(project, id, &new_id)?;
    if let Some(d) = project.panel.devices.iter_mut().find(|d| d.id == new_id) {
        d.of = of.to_owned();
        d.form = form;
        // A contact of a coil never stands in series with that same coil.
        if d.series == of {
            d.series.clear();
        }
    }
    Ok(new_id)
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

/// Connects the terminal `end` (`K1:A1`) by hand to `target`: a terminal (`K2_1:14`) or a line
/// (`bus:L1`); the hand wires that ended on `end` before are taken away. `None` takes them away
/// only, so the generator wires the terminal again. A target equal to `end` is refused.
pub fn connect_terminal(panel: &mut Panel, end: &str, target: Option<&str>) -> Result<(), String> {
    if split_end(end).is_none() {
        return Err(format!("неверный вывод «{end}»"));
    }
    if let Some(t) = target {
        if t == end {
            return Err("провод не может начинаться и кончаться на одном выводе".to_owned());
        }
        if crate::link_bus(t).is_none() && split_end(t).is_none() {
            return Err(format!("неверный вывод «{t}»"));
        }
    }
    panel.links.retain(|l| l.from != end && l.to != end);
    if let Some(t) = target {
        panel.links.push(WireLink {
            from: end.to_owned(),
            to: t.to_owned(),
        });
    }
    Ok(())
}

/// Which end of a wire key `A:pin>B:pin` is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireEnd {
    From,
    To,
}

/// Splits a wire end `DEV:pin` into the device and the terminal.
fn split_end(end: &str) -> Option<(&str, &str)> {
    let (device, pin) = end.split_once(':')?;
    (!device.is_empty() && !pin.is_empty()).then_some((device, pin))
}

/// The terminal of the board that the tag `tag` is wired to (`IO26`, or `expander.5`), as
/// `crab_schematic` writes it; `None` for a tag without a board pin.
pub fn board_terminal_of(project: &Project, lib: &Library, tag: &str) -> Option<String> {
    let block = project.logic.block(tag)?;
    let def = lib.resolve(&project.logic, block)?;
    let pin_param = def.def.params.iter().find(|p| p.kind == ParamKind::Pin)?;
    match block.params.get(&pin_param.name)? {
        ParamValue::Int(pin) => Some(match &block.expander {
            Some(expander) => format!("{expander}.{pin}"),
            None => format!("IO{pin}"),
        }),
        _ => None,
    }
}

/// The tag of the project that is wired to the terminal `terminal` of the board.
fn tag_on_terminal(project: &Project, lib: &Library, terminal: &str) -> Option<String> {
    project
        .logic
        .blocks
        .iter()
        .filter(|b| project.is_tag(lib, &b.id))
        .find(|b| board_terminal_of(project, lib, &b.id).as_deref() == Some(terminal))
        .map(|b| b.id.clone())
}

/// Whether the end of a wire (`DEV:pin`) is a protective earth terminal: the earth wire between
/// two devices of a power chain is not the chain itself.
fn is_pe(end: &str) -> bool {
    split_end(end).is_some_and(|(_, pin)| pin.to_ascii_lowercase().ends_with("pe"))
}

/// The upstream device and the downstream one of a power wire between `a` and `b`, and whether
/// the downstream device is fed by `load` (`true`) or by `from` (a branch).
fn power_wire(panel: &Panel, a: &str, b: &str) -> Option<(String, String, bool)> {
    if a == b {
        return None;
    }
    let (da, db) = (panel.device(a)?, panel.device(b)?);
    if da.load == b {
        Some((a.to_owned(), b.to_owned(), true))
    } else if db.load == a {
        Some((b.to_owned(), a.to_owned(), true))
    } else if db.from == a {
        Some((a.to_owned(), b.to_owned(), false))
    } else if da.from == b {
        Some((b.to_owned(), a.to_owned(), false))
    } else {
        None
    }
}

/// The refusal for a wire that the generator draws.
fn generator_wire(from: &str, to: &str) -> String {
    format!(
        "-{from} — -{to}: этот провод строит генератор; перепаять его нельзя, нарисуйте ручной провод"
    )
}

/// Moves one end of the wire `key` (`A:pin>B:pin`, as `WireLink::key` and wire numbers write it)
/// to the terminal `target` (`DEV:pin`), changing the real connection of the panel.
///
/// A hand-drawn wire changes its link. A power wire moves the feeding of its downstream device
/// (`load`, or a branch `from`). A wire to the board moves the tag bound to the terminal. Any
/// other wire is drawn by the generator and is refused. All or nothing: on Err the project is
/// unchanged.
pub fn rewire(
    project: &mut Project,
    lib: &Library,
    key: &str,
    end: WireEnd,
    target: &str,
) -> Result<(), String> {
    let bad_key = || format!("неверный провод «{key}»");
    let (a, b) = key.split_once('>').ok_or_else(bad_key)?;
    split_end(a).ok_or_else(bad_key)?;
    split_end(b).ok_or_else(bad_key)?;
    let (moving, staying) = match end {
        WireEnd::From => (a, b),
        WireEnd::To => (b, a),
    };
    let moving_dev = link_device(moving).to_owned();
    let staying_dev = link_device(staying).to_owned();
    let (target_dev, _) = split_end(target).ok_or_else(|| format!("неверный вывод «{target}»"))?;
    let target_dev = target_dev.to_owned();

    if target == staying {
        return Err("провод не может начинаться и кончаться на одном выводе".to_owned());
    }
    if target_dev != BOARD && project.panel.device(&target_dev).is_none() {
        return Err(format!("-{target_dev}: нет такого устройства в панели"));
    }
    if target == moving {
        return Ok(());
    }

    let mut next = project.clone();
    let hand = next.panel.links.iter().position(|l| l.key() == key);
    if let Some(index) = hand {
        let mut link = next.panel.links[index].clone();
        match end {
            WireEnd::From => link.from = target.to_owned(),
            WireEnd::To => link.to = target.to_owned(),
        }
        let taken = next
            .panel
            .links
            .iter()
            .enumerate()
            .any(|(i, l)| i != index && l.key() == link.key());
        if taken {
            return Err("такой провод уже есть".to_owned());
        }
        next.panel.links[index] = link;
    } else if moving_dev == BOARD && staying_dev == BOARD {
        return Err(generator_wire(link_device(a), link_device(b)));
    } else if moving_dev == BOARD {
        // The board end moves to another terminal: the device at the other end takes its tag.
        if target_dev != BOARD {
            return Err(format!(
                "-{staying_dev}: к выводу платы провод можно подключить только к другому выводу платы"
            ));
        }
        let (_, terminal) = split_end(target).unwrap_or_default();
        let tag = tag_on_terminal(project, lib, terminal)
            .ok_or_else(|| format!("к выводу {terminal} не привязан тег проекта"))?;
        rebind_device(&mut next, lib, &staying_dev, Some(&tag))?;
    } else if staying_dev == BOARD {
        // The device end moves to another device: it takes the tag of the terminal, the old
        // device lets go of it first (a tag has one device).
        if target_dev == BOARD {
            return Err(format!(
                "-{moving_dev}: провод нельзя подключить к плате с этого конца"
            ));
        }
        let (_, terminal) = split_end(staying).unwrap_or_default();
        let tag = tag_on_terminal(project, lib, terminal)
            .ok_or_else(|| format!("к выводу {terminal} не привязан тег проекта"))?;
        rebind_device(&mut next, lib, &moving_dev, None)?;
        rebind_device(&mut next, lib, &target_dev, Some(&tag))?;
    } else if let Some((up, down, load)) =
        power_wire(&project.panel, &moving_dev, &staying_dev).filter(|_| !is_pe(a) && !is_pe(b))
    {
        if target_dev == BOARD {
            return Err(generator_wire(link_device(a), link_device(b)));
        }
        // Another terminal of the same device: who feeds whom stays the same.
        if target_dev == moving_dev {
            return Ok(());
        }
        if moving_dev == down {
            // The new device is fed by the upstream one; the old downstream device is left.
            if load {
                next.panel.reconnect_power(&target_dev, Some(&up))?;
                // Reconnecting replaced the `load` of the upstream device; a branch from it goes too.
                if let Some(d) = next
                    .panel
                    .devices
                    .iter_mut()
                    .find(|d| d.id == down && d.from == up)
                {
                    d.from.clear();
                }
            } else {
                next.panel.branch_power(&target_dev, Some(&up))?;
                next.panel.branch_power(&down, None)?;
            }
        } else if load {
            // The upstream end moves: the downstream device is fed by the new device.
            next.panel.reconnect_power(&down, Some(&target_dev))?;
        } else {
            next.panel.branch_power(&down, Some(&target_dev))?;
        }
    } else {
        return Err(generator_wire(link_device(a), link_device(b)));
    }

    next.panel.wire_numbers.remove(key);
    *project = next;
    Ok(())
}
