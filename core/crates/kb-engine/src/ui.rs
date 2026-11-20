//! Interfaz del host compartida por la consola y el panel web local.
//! Cada pedido de aprobación tiene un id: lo responde quien llegue primero.

use kb_proto::{DeviceKind, Permission};
use kb_session::host::{ApprovalRequest, HostEvent, HostUi};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub enum Ask {
    Approve(ApprovalRequest, oneshot::Sender<Option<Permission>>),
    Confirm(String, String, oneshot::Sender<bool>),
}

#[derive(Serialize, Clone, Default)]
pub struct SlotView {
    pub name: String,
    pub permission: String,
    pub monitor: String,
    pub width: u32,
    pub height: u32,
    pub fps: u16,
    pub encoder: String,
}

#[derive(Serialize)]
pub struct PendingView {
    pub id: u64,
    pub kind: &'static str,
    pub device: String,
    pub detail: String,
}

pub struct Ui {
    pending: Mutex<VecDeque<(u64, Ask)>>,
    next_id: AtomicU64,
    pin: Mutex<Option<(String, String, Instant)>>,
    pub slots: Mutex<[Option<SlotView>; 3]>,
    log: Mutex<VecDeque<String>>,
    last_names: Mutex<[String; 3]>,
}

pub fn perm_es(p: Permission) -> &'static str {
    match p {
        Permission::Control => "control táctil",
        Permission::ViewOnly => "solo ver",
    }
}

fn kind_es(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Tablet => "tablet",
        DeviceKind::Phone => "teléfono",
        DeviceKind::Other => "dispositivo",
    }
}

impl Ui {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            pin: Mutex::new(None),
            slots: Mutex::new([None, None, None]),
            log: Mutex::new(VecDeque::new()),
            last_names: Mutex::new(Default::default()),
        }
    }

    fn note(&self, msg: String) {
        println!("{msg}");
        let mut l = self.log.lock().unwrap();
        l.push_back(msg);
        while l.len() > 30 {
            l.pop_front();
        }
    }

    pub fn log_lines(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().cloned().collect()
    }

    pub fn pin_view(&self) -> Option<(String, String)> {
        let mut g = self.pin.lock().unwrap();
        if g.as_ref().is_some_and(|(_, _, t)| t.elapsed() > Duration::from_secs(120)) {
            *g = None;
        }
        g.as_ref().map(|(d, p, _)| (d.clone(), p.clone()))
    }

    pub fn pending_view(&self) -> Vec<PendingView> {
        self.pending
            .lock()
            .unwrap()
            .iter()
            .map(|(id, a)| match a {
                Ask::Approve(r, _) => PendingView {
                    id: *id,
                    kind: "approve",
                    device: r.device_name.clone(),
                    detail: format!("{} · por {} · huella {}", kind_es(r.kind), r.method, r.fingerprint_short),
                },
                Ask::Confirm(d, action, _) => PendingView { id: *id, kind: "confirm", device: d.clone(), detail: action.clone() },
            })
            .collect()
    }

    /// Responde un pedido. `answer`: "c" control, "v" solo ver, "s" sí, otro = no.
    /// `id = None` responde el más antiguo (consola).
    pub fn answer(&self, id: Option<u64>, answer: &str) -> bool {
        let ask = {
            let mut p = self.pending.lock().unwrap();
            let idx = match id {
                Some(id) => p.iter().position(|(i, _)| *i == id),
                None => (!p.is_empty()).then_some(0),
            };
            idx.and_then(|i| p.remove(i))
        };
        let Some((_, ask)) = ask else { return false };
        match ask {
            Ask::Approve(req, tx) => {
                let d = match answer {
                    "c" => Some(Permission::Control),
                    "v" => Some(Permission::ViewOnly),
                    _ => None,
                };
                self.note(match d {
                    Some(p) => format!("  «{}» aprobado: {}", req.device_name, perm_es(p)),
                    None => format!("  «{}» rechazado.", req.device_name),
                });
                *self.pin.lock().unwrap() = None;
                let _ = tx.send(d);
            }
            Ask::Confirm(device, _, tx) => {
                let ok = matches!(answer, "s" | "si" | "sí" | "yes");
                self.note(format!("  «{device}»: {}", if ok { "permitido" } else { "denegado" }));
                let _ = tx.send(ok);
            }
        }
        true
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.lock().unwrap().is_empty()
    }
}

impl HostUi for Ui {
    fn show_pin(&self, device: &str, pin: &str) {
        *self.pin.lock().unwrap() = Some((device.to_string(), pin.to_string(), Instant::now()));
        println!("\n  ┌──────────────────────────────────────────────┐");
        println!("  │  «{device}» quiere emparejarse.");
        println!("  │  PIN para ingresar en el dispositivo:  {pin}");
        println!("  │  (vence en 2 minutos, un solo uso)");
        println!("  └──────────────────────────────────────────────┘");
    }
    fn hide_pin(&self, _device: &str) {
        *self.pin.lock().unwrap() = None;
    }
    fn approve(&self, req: ApprovalRequest) -> oneshot::Receiver<Option<Permission>> {
        let (tx, rx) = oneshot::channel();
        println!(
            "\n  ¿Aprobar «{}» ({}, por {}, huella {})?\n    [c] control táctil   [v] solo ver   [n] rechazar  → escribí la letra y Enter (o usá el panel)",
            req.device_name,
            kind_es(req.kind),
            req.method,
            req.fingerprint_short
        );
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending.lock().unwrap().push_back((id, Ask::Approve(req, tx)));
        rx
    }
    fn confirm(&self, device: &str, action: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        println!("\n  «{device}» pide {action}. ¿Permitir? [s] sí  [n] no  → escribí la letra y Enter (o usá el panel)");
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending.lock().unwrap().push_back((id, Ask::Confirm(device.to_string(), action.to_string(), tx)));
        rx
    }
    fn event(&self, ev: HostEvent) {
        match ev {
            HostEvent::Connected { slot, name, permission } => {
                self.slots.lock().unwrap()[slot as usize] =
                    Some(SlotView { name: name.clone(), permission: perm_es(permission).into(), ..Default::default() });
                self.note(format!("● Ranura {}: «{name}» conectado ({})", slot + 1, perm_es(permission)));
            }
            HostEvent::Streaming { slot, monitor, width, height, fps, encoder } => {
                let mut slots = self.slots.lock().unwrap();
                // Reconexión rápida: el "desconectado" de la sesión vieja puede llegar
                // después del "conectado" de la nueva; se recrea la ranura.
                if slots[slot as usize].is_none() {
                    let name = self.last_names.lock().unwrap()[slot as usize].clone();
                    slots[slot as usize] = Some(SlotView { name, ..Default::default() });
                }
                if let Some(s) = slots[slot as usize].as_mut() {
                    s.monitor = monitor.clone();
                    s.width = width;
                    s.height = height;
                    s.fps = fps;
                    s.encoder = encoder.clone();
                }
                drop(slots);
                self.note(format!("  Ranura {}: transmitiendo {monitor} a {width}x{height} @ {fps} FPS — {encoder}", slot + 1));
            }
            HostEvent::Disconnected { slot, name } => {
                self.slots.lock().unwrap()[slot as usize] = None;
                self.last_names.lock().unwrap()[slot as usize] = name.clone();
                self.note(format!("○ Ranura {}: «{name}» desconectado", slot + 1));
            }
            HostEvent::Rejected { name, reason } => self.note(format!("✕ «{name}» rechazado: {reason}")),
        }
    }
}
