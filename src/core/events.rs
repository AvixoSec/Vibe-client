use crate::core::snapshot::{Camera, GameSnapshot};

#[derive(Debug, Clone)]
pub enum ClientEvent {
    GameTick {
        tick: u64,
        snapshot: GameSnapshot,
    },
    PreRender {
        delta_time: f32,
        width: u32,
        height: u32,
    },
    Render2D {
        width: u32,
        height: u32,
    },
    Render3D {
        camera: Camera,
    },
    KeyEvent {
        key_code: u32,
        pressed: bool,
    },
    MouseEvent {
        delta_x: f32,
        delta_y: f32,
    },
    PacketSend {
        packet_id: u32,
        data_len: usize,
        cancel: bool,
    },
    PacketReceive {
        packet_id: u32,
        data_len: usize,
    },
}

pub type EventHandler = Box<dyn FnMut(&mut ClientEvent) + Send + Sync>;

pub struct EventBus {
    handlers: Vec<EventHandler>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub fn subscribe<F>(&mut self, handler: F)
    where
        F: FnMut(&mut ClientEvent) + Send + Sync + 'static,
    {
        self.handlers.push(Box::new(handler));
    }

    pub fn dispatch(&mut self, mut event: ClientEvent) -> ClientEvent {
        for handler in &mut self.handlers {
            handler(&mut event);
        }
        event
    }

    pub fn clear(&mut self) {
        self.handlers.clear();
    }
}
