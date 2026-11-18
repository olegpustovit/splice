use super::*;

impl Editor {
    pub(super) fn mint(&mut self, prefix: &str) -> Result<u64, CommandError> {
        let serial = if let Some(replay) = &self.replay {
            let id = replay.get(self.report.mints.len()).ok_or_else(bad_replay)?;
            let (recorded_prefix, digits) = id.split_once('_').ok_or_else(bad_replay)?;
            let serial = u64::from_str_radix(digits, 36).map_err(|_| bad_replay())?;
            if recorded_prefix != prefix
                || id != &wire(prefix, serial)
                || self.occupied(prefix, serial)
                || self.report.mints.contains(id)
            {
                return Err(bad_replay());
            }
            serial
        } else {
            let mut serial = self.project.serial.max(self.serial_floor);
            while self.occupied(prefix, serial) {
                serial = serial.checked_add(1).ok_or_else(exhausted)?;
            }
            serial
        };
        self.project.serial = self
            .project
            .serial
            .max(serial.checked_add(1).ok_or_else(exhausted)?);
        self.report.mints.push(wire(prefix, serial));
        Ok(serial)
    }

    fn occupied(&self, prefix: &str, serial: u64) -> bool {
        let p = &self.project;
        match prefix {
            "med" => p.media.iter().any(|media| media.id.serial() == serial),
            "clp" => p.clips.find(ClipId::from_serial(serial)).is_some(),
            "ovl" => p.items.find(ItemId::from_serial(serial)).is_some(),
            "lan" => {
                p.spine.id.serial() == serial
                    || p.lanes.iter().any(|lane| lane.id.serial() == serial)
            }
            "mrk" => p.markers.iter().any(|marker| marker.id.serial() == serial),
            "fx" => {
                let found = |effects: &[Effect]| effects.iter().any(|fx| fx.id.serial() == serial);
                found(&p.effects)
                    || found(&p.spine.effects)
                    || p.lanes.iter().any(|lane| found(&lane.effects))
                    || p.clips.order().iter().any(|&slot| {
                        found(&p.clips.data(slot).expect("live clip").appearance.effects)
                    })
                    || p.items.order().iter().any(|&slot| {
                        found(&p.items.data(slot).expect("live item").appearance.effects)
                    })
            }
            _ => unreachable!("mint prefix must name a model type"),
        }
    }
}

fn wire(prefix: &str, serial: u64) -> String {
    match prefix {
        "med" => MediaId::from_serial(serial).to_string(),
        "clp" => ClipId::from_serial(serial).to_string(),
        "ovl" => ItemId::from_serial(serial).to_string(),
        "lan" => LaneId::from_serial(serial).to_string(),
        "mrk" => MarkerId::from_serial(serial).to_string(),
        "fx" => EffectId::from_serial(serial).to_string(),
        _ => unreachable!("mint prefix must name a model type"),
    }
}

pub(super) fn bad_replay() -> CommandError {
    error(
        "REPLAY_MINTS",
        "The batch does not match its recorded IDs.",
        "Recover from the verified journal prefix.",
    )
}

fn exhausted() -> CommandError {
    error(
        "OUT_OF_RANGE",
        "The project has exhausted its ID serials.",
        "Recover from a project with a valid next serial.",
    )
}
