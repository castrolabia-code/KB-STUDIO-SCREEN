//! Perfiles de estudio: una foto de cómo está configurado cada dispositivo
//! (monitor de origen, calidad y permiso) que se puede volver a aplicar con
//! un clic. Se guardan en `profiles.json` (no contiene secretos: sólo
//! huellas públicas, nombres y preferencias).

use kb_geometry::Topology;
use kb_pairing::TrustStore;
use kb_proto::{Permission, QualityProfile};
use kb_session::host::Host;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const NAMES: [&str; 4] = ["Grabación vocal", "Mezcla", "Mastering", "Personalizado"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeviceSetup {
    pub name: String,
    pub monitor_id: Option<String>,
    /// Nombre del monitor al guardar: Windows puede renumerar `\\.\DISPLAYn`.
    pub monitor_name: Option<String>,
    pub profile: Option<QualityProfile>,
    pub permission: Permission,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Profiles {
    /// perfil -> (huella del dispositivo -> configuración)
    pub items: BTreeMap<String, BTreeMap<String, DeviceSetup>>,
}

impl Profiles {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &PathBuf) {
        if let Ok(j) = serde_json::to_vec_pretty(self) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, j).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    /// Guarda la configuración actual de todos los dispositivos de confianza.
    pub fn snapshot(&mut self, name: &str, host: &Host, topo: &Topology) -> usize {
        let mut m = BTreeMap::new();
        for (key, d) in host.trusted() {
            let mon = d.last_monitor.clone().or_else(|| topo.primary().map(|p| p.id.clone()));
            let mon_name = mon.as_ref().and_then(|id| topo.by_id(id)).map(|x| x.name.clone());
            m.insert(key, DeviceSetup { name: d.name, monitor_id: mon, monitor_name: mon_name, profile: d.profile, permission: d.permission });
        }
        let n = m.len();
        self.items.insert(name.to_string(), m);
        n
    }

    /// Aplica un perfil. Devuelve cuántos dispositivos (todavía de confianza) se configuraron.
    pub fn apply(&self, name: &str, host: &Host, topo: &Topology) -> usize {
        let Some(p) = self.items.get(name) else { return 0 };
        let mut n = 0;
        for (key, setup) in p {
            let Some(fp) = TrustStore::fingerprint_from_key(key) else { continue };
            let monitor = resolve_monitor(topo, setup.monitor_id.as_deref(), setup.monitor_name.as_deref());
            if host.apply_device(&fp, monitor, setup.profile, Some(setup.permission)) {
                n += 1;
            }
        }
        n
    }
}

/// Busca el monitor: mismo id y nombre; si no, por nombre; si no, por id.
pub fn resolve_monitor(topo: &Topology, id: Option<&str>, name: Option<&str>) -> Option<String> {
    let by_both = topo.monitors.iter().find(|m| Some(m.id.as_str()) == id && Some(m.name.as_str()) == name);
    let by_name = || topo.monitors.iter().find(|m| Some(m.name.as_str()) == name && !m.is_virtual);
    let by_id = || topo.monitors.iter().find(|m| Some(m.id.as_str()) == id);
    by_both.or_else(by_name).or_else(by_id).map(|m| m.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kb_geometry::{MonitorDesc, Rect};

    fn mon(id: &str, name: &str, x: i32) -> MonitorDesc {
        let b = Rect::new(x, 0, 1920, 1080);
        MonitorDesc { id: id.into(), name: name.into(), bounds: b, work_area: b, dpi: 96, primary: x == 0, is_virtual: false }
    }

    #[test]
    fn monitor_resolution_survives_renumbering() {
        // Al guardar: DISPLAY2 = "DELL". Después de reiniciar Windows lo renumeró a DISPLAY3.
        let topo = Topology { monitors: vec![mon(r"\\.\DISPLAY1", "LG", 0), mon(r"\\.\DISPLAY3", "DELL", 1920), mon(r"\\.\DISPLAY2", "Samsung", 3840)] };
        assert_eq!(resolve_monitor(&topo, Some(r"\\.\DISPLAY2"), Some("DELL")).as_deref(), Some(r"\\.\DISPLAY3"));
        // Mismo id y nombre: directo.
        assert_eq!(resolve_monitor(&topo, Some(r"\\.\DISPLAY1"), Some("LG")).as_deref(), Some(r"\\.\DISPLAY1"));
        // Monitor desaparecido y sin nombre conocido: por id si existe, si no nada.
        assert_eq!(resolve_monitor(&topo, Some(r"\\.\DISPLAY9"), Some("Viejo")), None);
    }
}
